//! fork 新增：GUI/CLI 界面文案的 i18n 层。
//!
//! 设计：英文原文即 key。`tr()` 在当前语言为 zh-CN 时查 [`zh_cn`] 译表，
//! 未命中回退英文原文——上游新增文案自动降级，不缺字、不炸构建。
//! 语言状态是进程内全局原子（同 herdr）：配置加载/设置页应用时经
//! `apply_language` 落地；渲染路径每次调用 `tr()` 现取现用，切换当帧生效。
//! `WEZTERM_LANG` 环境变量优先级最高（运维钉死，配置重载不可翻转）。
//!
//! 边界：本模块只管「界面文案」； anyhow 错误链、日志、配置校验消息
//! 保持英文（它们是开发者/上游语义，不属于翻译面）。

mod zh_cn;

use std::borrow::Cow;
use std::sync::atomic::{AtomicU8, Ordering};
use wezterm_dynamic::{Error, FromDynamic, FromDynamicOptions, ToDynamic, Value};

/// Interface language, value of the `language` config option.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UiLanguage {
    /// Simplified Chinese (the default for this fork)
    #[default]
    ZhCn,
    /// English
    En,
}

impl UiLanguage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ZhCn => "zh-CN",
            Self::En => "en",
        }
    }

    /// Tolerant parser used by both the config option and the
    /// `WEZTERM_LANG` env override
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "zh-cn" | "zh_cn" | "zh-hans" | "zh" => Some(Self::ZhCn),
            "en" => Some(Self::En),
            _ => None,
        }
    }
}

impl std::fmt::Display for UiLanguage {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        fmt.write_str(self.as_str())
    }
}

impl FromDynamic for UiLanguage {
    fn from_dynamic(value: &Value, _options: FromDynamicOptions) -> Result<Self, Error> {
        match value {
            Value::String(s) => Self::parse(s).ok_or_else(|| {
                Error::Message(format!(
                    "invalid language {s:?}: expected \"zh-CN\" or \"en\""
                ))
            }),
            _ => Err(Error::Message(
                "language must be a string: \"zh-CN\" or \"en\"".into(),
            )),
        }
    }
}

impl ToDynamic for UiLanguage {
    fn to_dynamic(&self) -> Value {
        Value::String(self.as_str().to_string())
    }
}

/// Environment variable that pins the interface language, outranking
/// any configuration (operator override; config reloads cannot flip it).
pub const LANG_ENV_VAR: &str = "WEZTERM_LANG";

static LANG: AtomicU8 = AtomicU8::new(UiLanguage::ZhCn as u8);

/// Current process-wide interface language
pub fn lang() -> UiLanguage {
    match LANG.load(Ordering::Relaxed) {
        1 => UiLanguage::En,
        _ => UiLanguage::ZhCn,
    }
}

/// Set the interface language directly (used by the settings overlay for
/// immediate feedback; config loads go through [`apply_language`])
pub fn set_lang(lang: UiLanguage) {
    LANG.store(lang as u8, Ordering::Relaxed);
}

/// Resolve and install the effective language:
/// `WEZTERM_LANG` > the configured value. Called from the config load
/// funnel so every load/reload/override path converges here.
pub fn apply_language(configured: UiLanguage) {
    let pinned = std::env::var(LANG_ENV_VAR)
        .ok()
        .as_deref()
        .and_then(UiLanguage::parse);
    set_lang(pinned.unwrap_or(configured));
}

/// Translate a literal UI string (English text is the lookup key).
/// Falls back to the key itself when no translation is recorded.
pub fn tr(s: &'static str) -> Cow<'static, str> {
    match lang() {
        UiLanguage::En => Cow::Borrowed(s),
        UiLanguage::ZhCn => zh_cn::translate(s),
    }
}

/// Substitute `{name}` placeholders in a (possibly translated) template.
/// `format!` cannot take a runtime template, so translated strings with
/// parameters keep `{name}` placeholders and are filled by this helper.
pub fn fill(template: &str, args: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        match after.find('}') {
            Some(close) => {
                let name = &after[..close];
                match args.iter().find(|(k, _)| *k == name) {
                    Some((_, v)) => out.push_str(v),
                    // Unknown placeholder: keep it verbatim so gaps stay visible
                    None => {
                        out.push('{');
                        out.push_str(name);
                        out.push('}');
                    }
                }
                rest = &after[close + 1..];
            }
            None => {
                out.push_str(&rest[open..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_language_roundtrip() {
        assert_eq!(UiLanguage::parse("zh-CN"), Some(UiLanguage::ZhCn));
        assert_eq!(UiLanguage::parse("ZH_cn"), Some(UiLanguage::ZhCn));
        assert_eq!(UiLanguage::parse("zh"), Some(UiLanguage::ZhCn));
        assert_eq!(UiLanguage::parse("en"), Some(UiLanguage::En));
        assert_eq!(UiLanguage::parse("fr"), None);

        let v = UiLanguage::ZhCn.to_dynamic();
        assert_eq!(
            UiLanguage::from_dynamic(&v, FromDynamicOptions::default()).unwrap(),
            UiLanguage::ZhCn
        );
        assert!(UiLanguage::from_dynamic(
            &Value::String("klingon".into()),
            FromDynamicOptions::default()
        )
        .is_err());
    }

    #[test]
    fn zh_table_sorted_unique() {
        let keys: Vec<&str> = zh_cn::ZH_CN.iter().map(|(k, _)| *k).collect();
        let mut sorted = keys.clone();
        sorted.sort_unstable();
        assert_eq!(keys, sorted, "ZH_CN must stay sorted by key");
        sorted.dedup();
        assert_eq!(keys.len(), sorted.len(), "ZH_CN keys must be unique");
    }

    #[test]
    fn apply_language_env_pin_wins() {
        // nextest runs each test in its own process, so env mutation is safe
        std::env::remove_var(LANG_ENV_VAR);
        apply_language(UiLanguage::En);
        assert_eq!(lang(), UiLanguage::En);

        std::env::set_var(LANG_ENV_VAR, "zh-CN");
        apply_language(UiLanguage::En);
        assert_eq!(lang(), UiLanguage::ZhCn);

        // An unparseable pin is ignored rather than guessing
        std::env::set_var(LANG_ENV_VAR, "??");
        apply_language(UiLanguage::En);
        assert_eq!(lang(), UiLanguage::En);
        std::env::remove_var(LANG_ENV_VAR);
    }

    #[test]
    fn fill_substitutes_named_args() {
        assert_eq!(fill("{n} panes", &[("n", "3")]), "3 panes");
        assert_eq!(fill("无占位符", &[]), "无占位符");
        assert_eq!(fill("{a}-{b}", &[("a", "1"), ("b", "2")]), "1-2");
        // Unknown placeholders and stray braces survive verbatim
        assert_eq!(fill("{x}", &[("y", "1")]), "{x}");
        assert_eq!(fill("a { b", &[("b", "1")]), "a { b");
    }

    #[test]
    fn tr_falls_back_to_key() {
        // With an (initially) sparse table, unknown keys must fall back
        set_lang(UiLanguage::ZhCn);
        let s = tr("Definitely Not A Recorded String");
        assert_eq!(s, "Definitely Not A Recorded String");
    }
}
