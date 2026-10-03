//! fork(A6-1): pane 前台进程探测的 stale-while-revalidate 缓存。
//!
//! Windows 没有 tcgetpgrp，前台进程只能靠「全量进程快照 + 逐进程读 PEB」
//! 推断，单次可达数十毫秒。标签栏/标题重算在 GUI 主线程以
//! `CachePolicy::AllowStale` 逐 pane 查询；这里让过期读立即返回旧值，由
//! 单个后台线程刷新（single-flight），刷新结果与旧值不同才回调
//! `on_changed` 让前端重算。`CachePolicy::FetchImmediate`（关闭确认等）走
//! `fetch_now` 同步路径。缓存只归所属 `LocalPane` 所有，本模块不碰 Mux/GUI。

use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

struct Slot<T> {
    /// `None` 也会被缓存：探测失败同样占住 TTL，避免每次查询都起线程。
    value: Option<Arc<T>>,
    updated: Instant,
}

pub(crate) struct StaleWhileRevalidate<T> {
    slot: Mutex<Option<Slot<T>>>,
    /// single-flight 标记：同一时刻最多一个后台刷新线程。
    updating: AtomicBool,
}

/// 后台刷新结束（含 fetch panic）时清除 in-flight 标记。
struct ClearOnDrop<'a>(&'a AtomicBool);

impl Drop for ClearOnDrop<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

impl<T: Send + Sync + 'static> StaleWhileRevalidate<T> {
    pub fn new() -> Self {
        Self {
            slot: Mutex::new(None),
            updating: AtomicBool::new(false),
        }
    }

    /// 在调用线程同步抓取并写回缓存（`CachePolicy::FetchImmediate`）。
    pub fn fetch_now(&self, fetch: impl FnOnce() -> Option<T>) -> Option<Arc<T>> {
        let value = fetch().map(Arc::new);
        self.slot.lock().replace(Slot {
            value: value.clone(),
            updated: Instant::now(),
        });
        value
    }

    /// 立即返回当前缓存（可能已过期；从未成功刷新过时为 `None`）。无缓存
    /// 或已过期时起后台线程执行 `fetch`；新值与旧值经 `same` 比较不同时，
    /// 在该后台线程回调 `on_changed`。调用线程从不阻塞在 `fetch` 上。
    pub fn get_stale<F, S, N>(
        self: &Arc<Self>,
        ttl: Duration,
        fetch: F,
        same: S,
        on_changed: N,
    ) -> Option<Arc<T>>
    where
        F: FnOnce() -> Option<T> + Send + 'static,
        S: FnOnce(&T, &T) -> bool + Send + 'static,
        N: FnOnce() + Send + 'static,
    {
        let (value, fresh) = match &*self.slot.lock() {
            Some(slot) => (slot.value.clone(), slot.updated.elapsed() < ttl),
            None => (None, false),
        };
        if !fresh {
            self.spawn_refresh(fetch, same, on_changed);
        }
        value
    }

    fn spawn_refresh<F, S, N>(self: &Arc<Self>, fetch: F, same: S, on_changed: N)
    where
        F: FnOnce() -> Option<T> + Send + 'static,
        S: FnOnce(&T, &T) -> bool + Send + 'static,
        N: FnOnce() + Send + 'static,
    {
        if self.updating.swap(true, Ordering::AcqRel) {
            // 已有刷新在途，本次直接用旧值
            return;
        }
        let cache = Arc::clone(self);
        let started = Instant::now();
        let spawned = std::thread::Builder::new()
            .name("pane-proc-refresh".to_string())
            .spawn(move || {
                let _clear = ClearOnDrop(&cache.updating);
                let value = fetch().map(Arc::new);
                if cache.store_refreshed(value, started, same) {
                    on_changed();
                }
            });
        if let Err(err) = spawned {
            log::error!("failed to spawn pane-proc-refresh thread: {err:#}");
            self.updating.store(false, Ordering::Release);
        }
    }

    /// 写回后台刷新结果，返回内容是否变化。刷新开始之后才落地的
    /// `fetch_now` 结果更新，保留它而丢弃本次结果。
    fn store_refreshed<S: FnOnce(&T, &T) -> bool>(
        &self,
        value: Option<Arc<T>>,
        started: Instant,
        same: S,
    ) -> bool {
        let mut slot = self.slot.lock();
        let previous = match slot.as_ref() {
            Some(current) if current.updated > started => return false,
            Some(current) => current.value.clone(),
            None => None,
        };
        let changed = match (&previous, &value) {
            (Some(old), Some(new)) => !same(old, new),
            (None, None) => false,
            _ => true,
        };
        slot.replace(Slot {
            value,
            updated: Instant::now(),
        });
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::sync::mpsc;

    const LONG_TTL: Duration = Duration::from_secs(3600);
    const WAIT: Duration = Duration::from_secs(5);

    fn same(a: &u32, b: &u32) -> bool {
        a == b
    }

    fn no_fetch() -> Option<u32> {
        panic!("fresh cache must not refetch")
    }

    /// 等后台刷新线程结束（`on_changed` 在清标记前调用，返回时已发生）。
    fn wait_idle(cache: &StaleWhileRevalidate<u32>) {
        let deadline = Instant::now() + WAIT;
        while cache.updating.load(Ordering::Acquire) {
            assert!(Instant::now() < deadline, "refresh did not finish");
            std::thread::yield_now();
        }
    }

    #[test]
    fn first_stale_read_returns_none_and_fills_in_background() {
        let cache = Arc::new(StaleWhileRevalidate::<u32>::new());
        let (tx, rx) = mpsc::channel();
        let value = cache.get_stale(LONG_TTL, || Some(7), same, move || tx.send(()).unwrap());
        assert!(value.is_none(), "no cache yet: must not block on a fetch");
        rx.recv_timeout(WAIT).expect("first fill notifies");
        wait_idle(&cache);
        let value = cache.get_stale(LONG_TTL, no_fetch, same, || {});
        assert_eq!(value.as_deref(), Some(&7));
    }

    #[test]
    fn expired_read_returns_stale_value_and_refreshes_single_flight() {
        let cache = Arc::new(StaleWhileRevalidate::<u32>::new());
        assert_eq!(cache.fetch_now(|| Some(1)).as_deref(), Some(&1));

        let (release_tx, release_rx) = mpsc::channel::<()>();
        let (changed_tx, changed_rx) = mpsc::channel();
        let fetches = Arc::new(AtomicUsize::new(0));

        let counter = Arc::clone(&fetches);
        let value = cache.get_stale(
            Duration::ZERO,
            move || {
                counter.fetch_add(1, Ordering::SeqCst);
                release_rx.recv().ok();
                Some(2)
            },
            same,
            move || changed_tx.send(()).unwrap(),
        );
        // 刷新被阻塞期间立即拿到旧值
        assert_eq!(value.as_deref(), Some(&1));

        // 刷新进行中再次过期读：仍返回旧值，且不再起第二个刷新
        let counter = Arc::clone(&fetches);
        let value = cache.get_stale(
            Duration::ZERO,
            move || {
                counter.fetch_add(1, Ordering::SeqCst);
                Some(99)
            },
            same,
            || {},
        );
        assert_eq!(value.as_deref(), Some(&1));

        release_tx.send(()).unwrap();
        changed_rx
            .recv_timeout(WAIT)
            .expect("changed value notifies");
        wait_idle(&cache);
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(fetches.load(Ordering::SeqCst), 1, "single-flight");
        let value = cache.get_stale(LONG_TTL, no_fetch, same, || {});
        assert_eq!(value.as_deref(), Some(&2));
    }

    #[test]
    fn unchanged_refresh_does_not_notify() {
        let cache = Arc::new(StaleWhileRevalidate::<u32>::new());
        cache.fetch_now(|| Some(5));
        let (tx, rx) = mpsc::channel();
        let value = cache.get_stale(
            Duration::ZERO,
            || Some(5),
            same,
            move || tx.send(()).unwrap(),
        );
        assert_eq!(value.as_deref(), Some(&5));
        wait_idle(&cache);
        assert!(rx.try_recv().is_err(), "same value must not notify");
    }

    #[test]
    fn fetch_now_is_not_overwritten_by_older_background_refresh() {
        let cache = Arc::new(StaleWhileRevalidate::<u32>::new());
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let (changed_tx, changed_rx) = mpsc::channel();
        cache.get_stale(
            LONG_TTL,
            move || {
                release_rx.recv().ok();
                Some(1)
            },
            same,
            move || changed_tx.send(()).unwrap(),
        );
        // 后台刷新开始后才落地的同步结果更新，不能被旧刷新覆盖
        std::thread::sleep(Duration::from_millis(10));
        assert_eq!(cache.fetch_now(|| Some(2)).as_deref(), Some(&2));
        release_tx.send(()).unwrap();
        wait_idle(&cache);
        assert!(changed_rx.try_recv().is_err());
        let value = cache.get_stale(LONG_TTL, no_fetch, same, || {});
        assert_eq!(value.as_deref(), Some(&2));
    }

    #[test]
    fn failed_fetch_is_cached_until_ttl() {
        let cache = Arc::new(StaleWhileRevalidate::<u32>::new());
        let fetches = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&fetches);
        let value = cache.get_stale(
            LONG_TTL,
            move || {
                counter.fetch_add(1, Ordering::SeqCst);
                None
            },
            same,
            || panic!("None -> None is not a change"),
        );
        assert!(value.is_none());
        wait_idle(&cache);
        // 失败结果同样占住 TTL，避免每次查询都起线程
        assert!(cache.get_stale(LONG_TTL, no_fetch, same, || {}).is_none());
        assert_eq!(fetches.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn panicking_refresh_releases_single_flight() {
        let cache = Arc::new(StaleWhileRevalidate::<u32>::new());
        cache.get_stale(LONG_TTL, || panic!("fetch failed"), same, || {});
        wait_idle(&cache);
        let (tx, rx) = mpsc::channel();
        cache.get_stale(LONG_TTL, || Some(3), same, move || tx.send(()).unwrap());
        rx.recv_timeout(WAIT).expect("refresh after a panicked one");
    }
}
