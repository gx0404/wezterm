-- 纯函数用例：herdr 应用模式判定与状态转移 + tab 标题进程名清洗 + Config 字段守门
-- + 启动菜单不起子进程 + Windows/Linux 键位一致。
--
-- 仓库没有 lua5.4/busted 等独立解释器（机器上只装了 liblua 库，没有 CLI），
-- 这里借用 wezterm 自带的 mlua 运行时当解释器：wezterm --config-file 会把
-- 该文件当配置执行，执行到 error() 时会静默回退默认配置、不产生任何可见
-- 诊断（已用探针验证），所以断言结果一律用 print 显式输出，不依赖异常/
-- 退出码。跑法：
--
--   wezterm --config-file tests/pure_fn_test.lua show-keys 2>&1 | grep PURE_FN_TEST
--
-- 全部通过时最后一行是 `PURE_FN_TEST: ALL PASS (n cases)`；任何一条失败会
-- 单独打印 `PURE_FN_TEST FAIL: <case> ...`。

local wezterm = require('wezterm')
-- wezterm.config_dir 是 --config-file 指向文件所在目录，即 tests/；回退一级
-- 拼出 wezterm-config 根目录，与调用时的进程 cwd 无关（wezterm 沙箱化的
-- Lua 环境没有 debug 库，不能用 debug.getinfo 自行定位脚本路径）。
local config_root = wezterm.config_dir .. '/..'
package.path = config_root .. '/?.lua;' .. config_root .. '/?/init.lua;' .. package.path

local status = require('events.status')
local tab_title = require('events.tab-title')

local failures = 0
local total = 0

---@param name string
---@param actual any
---@param expected any
local function check(name, actual, expected)
   total = total + 1
   if actual ~= expected then
      failures = failures + 1
      print(string.format('PURE_FN_TEST FAIL: %s (got %s, want %s)', name, tostring(actual), tostring(expected)))
   end
end

-- should_hide_tab_bar：herdr_app_mode / tab 数量 / 前台进程名 三个入参的判定表
check('hide.single_tab_herdr', status.should_hide_tab_bar(true, 1, 'herdr'), true)
check('hide.multi_tab_herdr', status.should_hide_tab_bar(true, 2, 'herdr'), false)
check('hide.single_tab_other_process', status.should_hide_tab_bar(true, 1, 'bash'), false)
check('hide.app_mode_disabled', status.should_hide_tab_bar(false, 1, 'herdr'), false)
check('hide.empty_process_name', status.should_hide_tab_bar(true, 1, ''), false)
check('hide.uncleaned_process_name', status.should_hide_tab_bar(true, 1, 'herdr.exe'), false)
check('hide.zero_tabs', status.should_hide_tab_bar(true, 0, 'herdr'), false)
-- decorations 含 INTEGRATED_BUTTONS：集成标题栏按钮画在 tab bar 里，不能隐藏
check(
   'hide.integrated_buttons',
   status.should_hide_tab_bar(true, 1, 'herdr', 'RESIZE|INTEGRATED_BUTTONS'),
   false
)
check(
   'hide.integrated_buttons_only',
   status.should_hide_tab_bar(true, 1, 'herdr', 'INTEGRATED_BUTTONS'),
   false
)
check('hide.native_decorations', status.should_hide_tab_bar(true, 1, 'herdr', 'TITLE|RESIZE'), true)
check('hide.decorations_not_string', status.should_hide_tab_bar(true, 1, 'herdr', 42), true)
check(
   'hide.integrated_still_needs_herdr',
   status.should_hide_tab_bar(true, 1, 'bash', 'INTEGRATED_BUTTONS'),
   false
)
check('integrated.detect_nil', tab_title.has_integrated_buttons(nil), false)
check('integrated.detect_empty', tab_title.has_integrated_buttons(''), false)
check('integrated.detect_native', tab_title.has_integrated_buttons('TITLE|RESIZE'), false)
check('integrated.detect_yes', tab_title.has_integrated_buttons('RESIZE|INTEGRATED_BUTTONS'), true)
check('integrated.detect_not_string', tab_title.has_integrated_buttons({}), false)

-- next_tab_bar_state：herdr 应用模式的 tab bar 状态转移（纯函数）
do
   local next_state = status.next_tab_bar_state

   -- 新窗口首次见到且不需隐藏：记录状态，不写 overrides
   local st, ov = next_state(nil, false, nil)
   check('state.first_seen_visible.hidden', st.hidden, false)
   check('state.first_seen_visible.no_write', ov, nil)

   -- 新窗口启动 herdr：只写 enable_tab_bar=false，不引入 background 等其他键
   st, ov = next_state(nil, true, nil)
   check('state.hide.hidden', st.hidden, true)
   check('state.hide.had_prev', st.had_prev, false)
   check('state.hide.enable_tab_bar', ov and ov.enable_tab_bar, false)
   check('state.hide.no_background', ov and ov.background, nil)

   -- 重载后状态丢失（或部署前旧实现遗留的 false 覆盖）：按原值 nil 记录，
   -- 退出 herdr 时删掉 enable_tab_bar 回落 base，其余覆盖键原样保留
   local legacy = { enable_tab_bar = false, background = 'wallpaper-sentinel' }
   st, ov = next_state(nil, true, legacy)
   check('state.legacy.had_prev', st.had_prev, false)
   check('state.legacy.input_untouched', legacy.enable_tab_bar, false)
   st, ov = next_state(st, false, ov)
   check('state.legacy.restore.hidden', st.hidden, false)
   check('state.legacy.restore.enable_tab_bar_removed', ov and ov.enable_tab_bar, nil)
   check('state.legacy.restore.keeps_other_keys', ov and ov.background, 'wallpaper-sentinel')

   -- 手动切换后判定不变：不改写（原样返回 state，overrides 为 nil）
   local hidden_state = { hidden = true, had_prev = false }
   st, ov = next_state(hidden_state, true, { enable_tab_bar = true })
   check('state.manual_toggle.same_state', st, hidden_state)
   check('state.manual_toggle.no_write', ov, nil)

   -- 多 tab 时恢复显示（判定由 should_hide_tab_bar 给出）
   local multi_tab_hide = status.should_hide_tab_bar(true, 2, 'herdr')
   st, ov = next_state(hidden_state, multi_tab_hide, { enable_tab_bar = false })
   check('state.multi_tab.hidden', st.hidden, false)
   check('state.multi_tab.enable_tab_bar_removed', ov and ov.enable_tab_bar, nil)

   -- 启动 herdr 前已手动隐藏（覆盖 false）：退出后还原为手动值 false
   st, ov = next_state({ hidden = false, had_prev = false }, true, { enable_tab_bar = false })
   check('state.prev_false.had_prev', st.had_prev, true)
   check('state.prev_false.prev', st.prev, false)
   st, ov = next_state(st, false, ov)
   check('state.prev_false.restore', ov and ov.enable_tab_bar, false)

   -- 启动 herdr 前手动显示（覆盖 true）：退出后还原为 true
   st, ov = next_state({ hidden = false, had_prev = false }, true, { enable_tab_bar = true })
   st, ov = next_state(st, false, ov)
   check('state.prev_true.restore', ov and ov.enable_tab_bar, true)
end

-- apply_herdr_app_mode 端到端：假 window/pane 驱动，状态走真实 wezterm.GLOBAL；
-- 清掉 package.loaded 重新 require 模拟配置重载（模块局部状态丢失、GLOBAL 保留）。
do
   local function fake_window(id)
      local window = { id = id, overrides = nil, writes = 0, tab_count = 1 }
      function window:window_id()
         return self.id
      end
      function window:mux_window()
         local tabs = {}
         for i = 1, self.tab_count do
            tabs[i] = i
         end
         return {
            tabs = function()
               return tabs
            end,
         }
      end
      function window:get_config_overrides()
         if self.overrides == nil then
            return nil
         end
         local copy = {}
         for k, v in pairs(self.overrides) do
            copy[k] = v
         end
         return copy
      end
      function window:set_config_overrides(value)
         self.overrides = value
         self.writes = self.writes + 1
      end
      return window
   end

   local function fake_pane(process)
      return {
         get_foreground_process_name = function()
            return process
         end,
      }
   end

   local function tab_bar_override(window)
      return window.overrides and window.overrides.enable_tab_bar
   end

   local function reload_status()
      package.loaded['events.status'] = nil
      return require('events.status')
   end

   -- 每次调用推进 10 秒，越过前台进程探测的 2 秒节流
   local clock = 0
   local function later()
      clock = clock + 10
      return clock
   end

   -- 取测试进程内不会与真实窗口冲突的大 id
   local window = fake_window(900001)
   local herdr = fake_pane('/usr/local/bin/herdr')
   local shell = fake_pane('/usr/bin/zsh')

   status.apply_herdr_app_mode(window, shell, true, later())
   check('apply.visible_no_write', window.writes, 0)

   status.apply_herdr_app_mode(window, herdr, true, later())
   check('apply.hide.writes', window.writes, 1)
   check('apply.hide.enable_tab_bar', tab_bar_override(window), false)
   check('apply.hide.no_background', window.overrides and window.overrides.background, nil)

   status.apply_herdr_app_mode(window, herdr, true, later())
   check('apply.steady_no_rewrite', window.writes, 1)

   local reloaded = reload_status()
   check('apply.reload.fresh_module', reloaded ~= status, true)
   reloaded.apply_herdr_app_mode(window, herdr, true, later())
   check('apply.reload.no_rewrite', window.writes, 1)

   reloaded.apply_herdr_app_mode(window, shell, true, later())
   check('apply.reload.restore.writes', window.writes, 2)
   check('apply.reload.restore.enable_tab_bar', tab_bar_override(window), nil)

   -- 手动隐藏后启动 herdr，中途重载，再退出：GLOBAL 里的 prev=false 生效
   -- （也覆盖 false 值经 GLOBAL 往返不被读成 nil）
   local manual = fake_window(900002)
   manual.overrides = { enable_tab_bar = false }
   reloaded.apply_herdr_app_mode(manual, shell, true, later())
   reloaded.apply_herdr_app_mode(manual, herdr, true, later())
   reloaded = reload_status()
   reloaded.apply_herdr_app_mode(manual, shell, true, later())
   check('apply.manual_prev_false.restore', tab_bar_override(manual), false)

   -- 同上但手动值为 true：能区分「按 GLOBAL 记录还原」与「重载丢状态后卡在 false」
   local manual_shown = fake_window(900004)
   manual_shown.overrides = { enable_tab_bar = true }
   reloaded.apply_herdr_app_mode(manual_shown, shell, true, later())
   reloaded.apply_herdr_app_mode(manual_shown, herdr, true, later())
   check('apply.manual_prev_true.hidden', tab_bar_override(manual_shown), false)
   reloaded = reload_status()
   reloaded.apply_herdr_app_mode(manual_shown, shell, true, later())
   check('apply.manual_prev_true.restore', tab_bar_override(manual_shown), true)

   -- 多 tab：隐藏后开第二个 tab 即恢复
   local multi = fake_window(900003)
   reloaded.apply_herdr_app_mode(multi, herdr, true, later())
   multi.tab_count = 2
   reloaded.apply_herdr_app_mode(multi, herdr, true, later())
   check('apply.multi_tab.restore', tab_bar_override(multi), nil)
   check('apply.multi_tab.writes', multi.writes, 2)

   -- 前台进程探测：同一窗口 2 秒内复用结果，多 tab 或关闭应用模式时不探测
   local probes = 0
   local counting = {
      get_foreground_process_name = function()
         probes = probes + 1
         return '/usr/local/bin/herdr'
      end,
   }
   local throttled = fake_window(900005)
   reloaded.apply_herdr_app_mode(throttled, counting, true, 1000)
   reloaded.apply_herdr_app_mode(throttled, counting, true, 1001)
   check('probe.throttled', probes, 1)
   check('probe.throttled.hidden', tab_bar_override(throttled), false)
   reloaded.apply_herdr_app_mode(throttled, counting, true, 1002)
   check('probe.after_interval', probes, 2)
   throttled.tab_count = 2
   reloaded.apply_herdr_app_mode(throttled, counting, true, 1010)
   check('probe.skipped_multi_tab', probes, 2)
   check('probe.multi_tab.restore', tab_bar_override(throttled), nil)
   throttled.tab_count = 1
   reloaded.apply_herdr_app_mode(throttled, counting, false, 1020)
   check('probe.skipped_app_mode_off', probes, 2)
   -- 时钟往回拨（上次探测记在 1002）：按过期处理，重新探测
   reloaded.apply_herdr_app_mode(throttled, counting, true, 990)
   check('probe.clock_stepped_back', probes, 3)

   -- 集成标题栏按钮（window_decorations 含 INTEGRATED_BUTTONS）：tab bar 就是标题栏，单 tab
   -- 前台是 herdr 也不隐藏；原生装饰照常隐藏。窗口装饰按窗口缓存 30 秒，且前台不是 herdr
   -- （不可能隐藏）时根本不取（effective_config 要转换整份配置，很重）。
   local function with_decorations(target, decorations)
      local reads = { count = 0 }
      function target:effective_config()
         reads.count = reads.count + 1
         return { window_decorations = decorations }
      end
      return reads
   end
   local integrated = fake_window(900006)
   local integrated_reads = with_decorations(integrated, 'RESIZE|INTEGRATED_BUTTONS')
   reloaded.apply_herdr_app_mode(integrated, shell, true, 2000)
   check('integrated.not_read_for_shell', integrated_reads.count, 0)
   reloaded.apply_herdr_app_mode(integrated, herdr, true, 2010)
   reloaded.apply_herdr_app_mode(integrated, herdr, true, 2020)
   check('integrated.never_hidden', integrated.writes, 0)
   check('integrated.keeps_tab_bar', tab_bar_override(integrated), nil)
   check('integrated.read_cached', integrated_reads.count, 1)
   reloaded.apply_herdr_app_mode(integrated, herdr, true, 2050)
   check('integrated.read_again_after_ttl', integrated_reads.count, 2)

   local native = fake_window(900007)
   with_decorations(native, 'TITLE|RESIZE')
   reloaded.apply_herdr_app_mode(native, herdr, true, 3000)
   check('native.hidden', tab_bar_override(native), false)

   -- 取不到窗口装饰（effective_config 抛错）：按没有集成按钮处理，不缓存失败
   local broken = fake_window(900008)
   function broken:effective_config()
      error('boom')
   end
   reloaded.apply_herdr_app_mode(broken, herdr, true, 4000)
   check('decorations_unreadable.hides', tab_bar_override(broken), false)

   -- 之前（无集成按钮的旧配置）隐藏过的窗口，重载成集成按钮配置后要把 tab bar 放回来
   local migrated = fake_window(900009)
   reloaded.apply_herdr_app_mode(migrated, herdr, true, 5000)
   check('integrated.migrated_hidden_before', tab_bar_override(migrated), false)
   with_decorations(migrated, 'RESIZE|INTEGRATED_BUTTONS')
   reloaded.apply_herdr_app_mode(migrated, herdr, true, 5100)
   check('integrated.migrated_restored', tab_bar_override(migrated), nil)
end

-- update-status：battery_info 在缓存期内只查一次
do
   local battery_calls = 0
   local real_battery_info, real_on = wezterm.battery_info, wezterm.on
   local handler = nil
   wezterm.battery_info = function()
      battery_calls = battery_calls + 1
      return { { state_of_charge = 0.5, state = 'Charging' } }
   end
   wezterm.on = function(name, callback)
      if name == 'update-status' then
         handler = callback
      end
   end
   package.loaded['events.status'] = nil
   require('events.status').setup({ herdr_app_mode = false })
   wezterm.on = real_on

   local window = {}
   function window:active_workspace()
      return 'default'
   end
   function window:active_key_table()
      return nil
   end
   function window:leader_is_active()
      return false
   end
   function window:window_id()
      return 900010
   end
   function window:mux_window()
      return {
         tabs = function()
            return { 1 }
         end,
      }
   end
   function window:set_left_status(_) end
   function window:set_right_status(value)
      self.right = value
   end
   handler(window, nil)
   handler(window, nil)
   wezterm.battery_info = real_battery_info
   check('battery.cached', battery_calls, 1)
   check('battery.rendered', window.right ~= nil and window.right:find('50%%') ~= nil, true)
end

-- utils/cache.lua::still_fresh：有效期内为真，到期或时钟往回拨为假
do
   local still_fresh = require('utils.cache').still_fresh
   check('cache.fresh', still_fresh(100, 101, 2), true)
   check('cache.same_second', still_fresh(100, 100, 2), true)
   check('cache.expired_at_ttl', still_fresh(100, 102, 2), false)
   check('cache.expired', still_fresh(100, 200, 2), false)
   check('cache.clock_back', still_fresh(100, 99, 2), false)
end

-- update-status：已关闭窗口的状态按 gui_windows() 回收，不随窗口开关无限增长
do
   local real_gui, real_on, real_battery = wezterm.gui, wezterm.on, wezterm.battery_info
   local handler = nil
   wezterm.on = function(name, callback)
      if name == 'update-status' then
         handler = callback
      end
   end
   wezterm.battery_info = function()
      return {}
   end
   package.loaded['events.status'] = nil
   local recycling = require('events.status')
   recycling.setup({ herdr_app_mode = false })
   wezterm.on = real_on

   local function status_window(id)
      local window = { id = id }
      function window:active_workspace()
         return 'default'
      end
      function window:active_key_table()
         return nil
      end
      function window:leader_is_active()
         return false
      end
      function window:window_id()
         return self.id
      end
      function window:mux_window()
         return {
            tabs = function()
               return { 1 }
            end,
         }
      end
      function window:set_left_status(_) end
      function window:set_right_status(_) end
      return window
   end
   local function gui_window(id)
      return {
         window_id = function()
            return id
         end,
      }
   end

   local first, second = status_window(900021), status_window(900022)
   wezterm.gui = {
      gui_windows = function()
         return { gui_window(900021), gui_window(900022) }
      end,
   }
   handler(first, nil)
   handler(second, nil)
   check('status_recycle.both_tracked', recycling.tracked_window_count(), 2)
   wezterm.gui = {
      gui_windows = function()
         return { gui_window(900021) }
      end,
   }
   handler(first, nil)
   check('status_recycle.closed_removed', recycling.tracked_window_count(), 1)
   -- 取不到窗口列表时本轮不回收（也不抛错）
   wezterm.gui = {
      gui_windows = function()
         error('no gui')
      end,
   }
   handler(second, nil)
   check('status_recycle.error_keeps_state', recycling.tracked_window_count(), 2)
   wezterm.gui = nil
   handler(first, nil)
   check('status_recycle.no_gui_keeps_state', recycling.tracked_window_count(), 2)
   wezterm.gui, wezterm.battery_info = real_gui, real_battery
   package.loaded['events.status'] = nil
end

-- tab 标题（events/tab-title.lua）：首帧即完整、手动重命名走 mux 的 tab 标题、
-- 前台进程名限频、已关闭 tab 的状态按窗口回收。
do
   package.loaded['events.tab-title'] = nil
   local titles = require('events.tab-title')
   local opts = { unseen_icon = 'numbered_box', hide_active_tab_unseen = true }

   local function flatten(items)
      local parts = {}
      for _, item in ipairs(items) do
         if type(item) == 'table' and item.Text then
            table.insert(parts, item.Text)
         end
      end
      return table.concat(parts)
   end

   -- 假 tab：foreground_process_name 像真实字段一样惰性取值，并记下读取次数
   local function fake_tab(id, window_id, process, title)
      local reads = { count = 0 }
      local pane = { pane_id = id * 10, title = title, has_unseen_output = false }
      setmetatable(pane, {
         __index = function(_, key)
            if key == 'foreground_process_name' then
               reads.count = reads.count + 1
               return process
            end
         end,
      })
      local tab = {
         tab_id = id,
         window_id = window_id,
         is_active = false,
         tab_title = '',
         active_pane = pane,
         panes = { pane },
      }
      return tab, reads
   end

   -- 首次渲染就是完整标题（此前首帧只有空壳，标题被截成「pw…」）
   local tab, reads = fake_tab(1, 700, '/usr/bin/pwsh', 'build')
   local text = flatten(titles.render_tab(opts, tab, { tab }, false, 32, 1000))
   check('tab_title.first_frame_full', text:find('pwsh ~ build', 1, true) ~= nil, true)
   check('tab_title.first_frame_not_truncated', text:find('…', 1, true), nil)
   check('tab_title.tracked_after_first', titles.tracked_tab_count(), 1)

   -- 手动标题（tab.tab_title）优先于自动标题；清空后恢复自动标题
   tab.tab_title = 'my tab'
   text = flatten(titles.render_tab(opts, tab, { tab }, false, 32, 1000))
   check('tab_title.manual_shown', text:find('my tab', 1, true) ~= nil, true)
   check('tab_title.manual_hides_auto', text:find('pwsh ~', 1, true), nil)
   tab.tab_title = ''
   text = flatten(titles.render_tab(opts, tab, { tab }, false, 32, 1000))
   check('tab_title.reset_restores_auto', text:find('pwsh ~ build', 1, true) ~= nil, true)

   -- 前台进程名：同一窗格 2 秒内只读一次；到期、时钟回拨、换窗格都重读
   check('tab_title.probe.first_frames', reads.count, 1)
   titles.render_tab(opts, tab, { tab }, true, 32, 1001)
   check('tab_title.probe.throttled', reads.count, 1)
   titles.render_tab(opts, tab, { tab }, false, 32, 1002)
   check('tab_title.probe.after_interval', reads.count, 2)
   titles.render_tab(opts, tab, { tab }, false, 32, 990)
   check('tab_title.probe.clock_back', reads.count, 3)
   tab.active_pane.pane_id = 99
   titles.render_tab(opts, tab, { tab }, false, 32, 990)
   check('tab_title.probe.pane_changed', reads.count, 4)

   -- 未读输出：非活动 tab 显示编号，活动 tab 按 hide_active_tab_unseen 隐藏
   local busy = fake_tab(2, 700, 'zsh', 'logs')
   busy.panes = {
      { has_unseen_output = true },
      { has_unseen_output = true },
      { has_unseen_output = false },
   }
   local box_two = wezterm.nerdfonts.md_numeric_2_box_multiple
   text = flatten(titles.render_tab(opts, busy, { tab, busy }, false, 32, 1000))
   check('tab_title.unseen_count_shown', text:find(box_two, 1, true) ~= nil, true)
   busy.is_active = true
   text = flatten(titles.render_tab(opts, busy, { tab, busy }, false, 32, 1000))
   check('tab_title.unseen_hidden_when_active', text:find(box_two, 1, true), nil)

   -- 回收：只清同一窗口里已不在 tabs 的 tab，别的窗口的状态不受影响
   local extra = fake_tab(3, 700, 'zsh', 'a')
   local other, other_reads = fake_tab(9, 800, 'zsh', 'b')
   titles.render_tab(opts, extra, { tab, busy, extra }, false, 32, 1000)
   titles.render_tab(opts, other, { other }, false, 32, 1000)
   check('tab_title.recycle.before', titles.tracked_tab_count(), 4)
   -- 窗口 700 关掉了 tab 2、3：只剩 tab 1
   titles.render_tab(opts, tab, { tab }, false, 32, 1000)
   check('tab_title.recycle.closed_removed', titles.tracked_tab_count(), 2)
   titles.render_tab(opts, other, { other }, false, 32, 1000)
   check('tab_title.recycle.other_window_kept', other_reads.count, 1)
   -- tabs 为 nil（异常调用）也不抛错、不清掉当前 tab
   check(
      'tab_title.recycle.nil_tabs',
      pcall(titles.render_tab, opts, other, nil, false, 32, 1000),
      true
   )
   check('tab_title.recycle.nil_tabs_kept', titles.tracked_tab_count(), 2)

   -- active_pane 为 nil（窗口刚创建）：不抛错，标题为空
   local bare = { tab_id = 4, window_id = 700, is_active = false, tab_title = '', panes = {} }
   check(
      'tab_title.nil_active_pane',
      pcall(titles.render_tab, opts, bare, { bare }, false, 32, 1000),
      true
   )

   -- 手动重命名/重置：写进 mux 的 tab 标题（tab:set_title），空串即恢复自动标题
   local function fake_window(active)
      return {
         active_tab = function()
            return active
         end,
      }
   end
   local mux_tab = {
      set_title = function(self, value)
         self.title = value
      end,
   }
   titles.apply_manual_title(fake_window(mux_tab), 'release')
   check('tab_title.rename.set_title', mux_tab.title, 'release')
   titles.apply_manual_title(fake_window(mux_tab), '')
   check('tab_title.reset.set_title_empty', mux_tab.title, '')
   mux_tab.title = 'kept'
   titles.apply_manual_title(fake_window(mux_tab), nil)
   check('tab_title.rename.cancelled_keeps_title', mux_tab.title, 'kept')
   check(
      'tab_title.rename.no_active_tab',
      pcall(titles.apply_manual_title, fake_window(nil), 'x'),
      true
   )
end

-- clean_process_name（来自 events/tab-title.lua，herdr 应用模式判断复用同一口径）
check('clean.unix_path', tab_title.clean_process_name('/usr/bin/herdr'), 'herdr')
check('clean.windows_path_exe', tab_title.clean_process_name('C:\\Users\\x\\herdr.exe'), 'herdr')
check('clean.bare_name', tab_title.clean_process_name('herdr'), 'herdr')
check('clean.empty', tab_title.clean_process_name(''), '')

-- toggled_tab_bar_overrides（events/tab-title.lua 手动切换 tab bar）
do
   local toggled = tab_title.toggled_tab_bar_overrides
   local input = { background = 'wallpaper-sentinel', enable_tab_bar = false }
   local out = toggled(input, false)
   check('toggle.show', out.enable_tab_bar, true)
   check('toggle.keeps_other_keys', out.background, 'wallpaper-sentinel')
   check('toggle.input_untouched', input.enable_tab_bar, false)
   out = toggled(nil, true)
   check('toggle.hide_from_nil', out.enable_tab_bar, false)
   check('toggle.no_background', out.background, nil)
end

-- 手动切换 tab bar 的事件处理（tabs.toggle-tab-bar）：集成标题栏按钮模式下标签栏就是
-- 标题栏，不允许隐藏；原生装饰照常切换；已经隐藏的窗口仍可切回显示。
do
   local real_on = wezterm.on
   local handlers = {}
   wezterm.on = function(name, callback)
      handlers[name] = callback
   end
   package.loaded['events.tab-title'] = nil
   require('events.tab-title').setup({})
   wezterm.on = real_on
   package.loaded['events.tab-title'] = tab_title

   local function toggle_window(enable_tab_bar, decorations)
      local window = { overrides = nil, writes = 0 }
      function window:effective_config()
         return { enable_tab_bar = enable_tab_bar, window_decorations = decorations }
      end
      function window:get_config_overrides()
         return self.overrides
      end
      function window:set_config_overrides(value)
         self.overrides = value
         self.writes = self.writes + 1
      end
      return window
   end
   local toggle = handlers['tabs.toggle-tab-bar']
   check('toggle_event.registered', type(toggle), 'function')
   if toggle then
      local native = toggle_window(true, 'TITLE|RESIZE')
      toggle(native, nil)
      check(
         'toggle_event.native_hides',
         native.overrides and native.overrides.enable_tab_bar,
         false
      )
      local integrated = toggle_window(true, 'RESIZE|INTEGRATED_BUTTONS')
      toggle(integrated, nil)
      check('toggle_event.integrated_keeps', integrated.writes, 0)
      local hidden = toggle_window(false, 'RESIZE|INTEGRATED_BUTTONS')
      toggle(hidden, nil)
      check(
         'toggle_event.integrated_can_show',
         hidden.overrides and hidden.overrides.enable_tab_bar,
         true
      )
   end
end

-- 配色拆分（colors/custom.lua）：scheme 是整套调色板（注册为默认方案，设置浮层选别的方案
-- 才能整套换掉），chrome 只放界面色；ANSI 用 Catppuccin Mocha 官方 16 色。
do
   local colors = require('colors.custom')
   check('colors.name', colors.name, 'GX Mocha')
   check('colors.has_mocha', type(colors.mocha), 'table')
   check('colors.scheme.background', colors.scheme.background, '#1f1f28')
   check('colors.scheme.ansi_count', #colors.scheme.ansi, 8)
   check('colors.scheme.brights_count', #colors.scheme.brights, 8)
   check('colors.scheme.ansi_black', colors.scheme.ansi[1], '#45475a')
   check('colors.scheme.ansi_blue', colors.scheme.ansi[5], '#89b4fa')
   check('colors.scheme.ansi_magenta', colors.scheme.ansi[6], '#f5c2e7')
   check('colors.scheme.ansi_white', colors.scheme.ansi[8], '#bac2de')
   check('colors.scheme.brights_black', colors.scheme.brights[1], '#585b70')
   check('colors.scheme.brights_white', colors.scheme.brights[8], '#a6adc8')
   check('colors.scheme.indexed', colors.scheme.indexed[16], colors.mocha.peach)
   -- chrome 不得带调色板键，否则会盖掉设置浮层选中的方案
   for _, key in ipairs({ 'foreground', 'background', 'ansi', 'brights', 'indexed' }) do
      check('colors.chrome.no_' .. key, colors.chrome[key], nil)
   end
   check('colors.chrome.tab_bar', type(colors.chrome.tab_bar), 'table')
   check('colors.chrome.split', colors.chrome.split, colors.mocha.overlay0)

   local appearance = require('config.appearance')
   check('appearance.color_scheme', appearance.color_scheme, colors.name)
   check('appearance.color_schemes', appearance.color_schemes[colors.name], colors.scheme)
   check('appearance.colors_is_chrome', appearance.colors, colors.chrome)
   check('appearance.close_confirmation', appearance.window_close_confirmation, 'AlwaysPrompt')

   -- 遮罩层/专注模式背景跟随 gui-settings.json 里设置浮层选中的方案
   local backdrops = require('utils.backdrops')
   local scheme_background = backdrops.scheme_background
   local builtin = { Dracula = '#282a36', Broken = 42, Empty = '' }
   local function lookup(name)
      return builtin[name]
   end
   local default_bg = colors.scheme.background
   local cases = {
      { 'no_key', '{"wallpaper": "a.png"}', default_bg },
      { 'gx_scheme', '{"color_scheme": "GX Mocha"}', default_bg },
      { 'builtin', '{"color_scheme": "Dracula"}', '#282a36' },
      { 'compact_json', '{\n  "wallpaper": "a.png",\n  "color_scheme":"Dracula"\n}', '#282a36' },
      { 'unknown_scheme', '{"color_scheme": "Nope"}', default_bg },
      { 'bad_type', '{"color_scheme": "Broken"}', default_bg },
      { 'empty_color', '{"color_scheme": "Empty"}', default_bg },
   }
   check('backdrop.no_sidecar', scheme_background(nil, lookup), default_bg)
   for _, case in ipairs(cases) do
      check('backdrop.' .. case[1], scheme_background(case[2], lookup), case[3])
   end
   local dracula = '{"color_scheme": "Dracula"}'
   check('backdrop.lookup_error', scheme_background(dracula, error), default_bg)
   -- 测试环境的配置目录（tests/）旁没有 gui-settings.json，实例走默认方案背景
   check('backdrop.instance_focus', backdrops.focus_color, colors.scheme.background)
   local saved_images = backdrops.images
   backdrops.images = { 'wallpaper.png' }
   local layers = backdrops:_create_opts()
   backdrops.images = saved_images
   check('backdrop.mask_layer', layers[2] and layers[2].source.Color, backdrops.mask_color)
end

-- GX Shell 安装包内置 shell 探测（utils/gx-shell.lua）
do
   local gx_shell = require('utils.gx-shell')
   local win_dir = 'C:\\Users\\x\\AppData\\Local\\Programs\\GXShell\\wezterm'
   local win_root = 'C:\\Users\\x\\AppData\\Local\\Programs\\GXShell'
   check('gx.root.windows', gx_shell.install_root(win_dir), win_root)
   check('gx.root.trailing_separator', gx_shell.install_root(win_dir .. '\\'), win_root)
   check('gx.root.linux', gx_shell.install_root('/usr/lib/wezterm-gx'), '/usr/lib')
   check('gx.root.no_parent', gx_shell.install_root('wezterm'), nil)
   check('gx.root.nil', gx_shell.install_root(nil), nil)

   local present = {}
   local function exists(path)
      return present[path] == true
   end
   check('gx.detect.standalone_windows', gx_shell.detect(win_dir, true, exists), nil)
   present[win_root .. '\\bin\\gx-zsh.exe'] = true
   check('gx.detect.needs_herdr_too', gx_shell.detect(win_dir, true, exists), nil)
   present[win_root .. '\\bin\\herdr.exe'] = true
   local entries = gx_shell.detect(win_dir, true, exists)
   check('gx.detect.windows.gx_zsh', entries and entries.gx_zsh, win_root .. '\\bin\\gx-zsh.exe')
   check('gx.detect.windows.herdr', entries and entries.herdr, win_root .. '\\bin\\herdr.exe')

   present['/usr/lib/ohmyzsh-gx/bin/gx-zsh'] = true
   present['/usr/lib/ohmyzsh-gx/bin/herdr'] = true
   entries = gx_shell.detect('/usr/lib/wezterm-gx', false, exists)
   check('gx.detect.deb.gx_zsh', entries and entries.gx_zsh, '/usr/lib/ohmyzsh-gx/bin/gx-zsh')
   check('gx.detect.deb.herdr', entries and entries.herdr, '/usr/lib/ohmyzsh-gx/bin/herdr')
   check('gx.detect.dev_build', gx_shell.detect('/src/wezterm/target/release', false, exists), nil)
end

-- 启动菜单（config/launch.lua）在 require 里求值：不能起子进程，每项固定 domain，
-- 除 herdr 外都带 GX_SHELL_ID；WSL 列表（utils/wsl.lua）在重试间隔内只取一次。
do
   local spawned, wsl_calls = 0, 0
   local real_run, real_background = wezterm.run_child_process, wezterm.background_child_process
   local real_wsl = wezterm.default_wsl_domains
   wezterm.run_child_process = function()
      spawned = spawned + 1
      return false, '', ''
   end
   wezterm.background_child_process = function()
      spawned = spawned + 1
   end
   wezterm.default_wsl_domains = function()
      wsl_calls = wsl_calls + 1
      return {}
   end
   wezterm.GLOBAL.gx_wsl = nil
   local loaded, launch
   for _ = 1, 2 do
      package.loaded['config.domains'] = nil
      package.loaded['config.launch'] = nil
      loaded, launch = pcall(require, 'config.launch')
   end
   wezterm.run_child_process, wezterm.background_child_process = real_run, real_background
   wezterm.default_wsl_domains = real_wsl

   check('launch.loads', loaded, true)
   check('launch.no_child_process', spawned, 0)
   check('launch.wsl_listed_once', wsl_calls, require('utils.platform').is_win and 1 or 0)
   check('domains.always_table', type(require('config.domains').wsl_domains), 'table')
   if loaded then
      check('launch.has_entries', #launch.launch_menu > 0, true)
      for idx, entry in ipairs(launch.launch_menu) do
         check('launch.domain.' .. idx, type(entry.domain) == 'table' and type(entry.domain.DomainName), 'string')
         local tagged = entry.set_environment_variables ~= nil and entry.set_environment_variables.GX_SHELL_ID ~= nil
         check('launch.tag.' .. idx, tagged, entry.label ~= 'herdr')
      end
      check('launch.default_marked', require('utils.shells').default_index(launch) ~= nil, true)
   end
end

-- 插件目录名与 lua-api-crates/plugin 的 compute_repo_dir 一致（含其 Rust 测试向量）。
do
   local plugins = require('config.plugins')
   check('plugin_dir.plain', plugins.compute_repo_dir('foo'), 'foo')
   check(
      'plugin_dir.path',
      plugins.compute_repo_dir('githubsDscom/wezterm/wezterm-plugins'),
      'githubsDscomsZsweztermsZswezterm-plugins'
   )
   check('plugin_dir.port', plugins.compute_repo_dir('localhost:8080/repo'), 'localhostsCs8080sZsrepo')
   check(
      'plugin_dir.trailing_slash',
      plugins.compute_repo_dir('https://github.com/a/b/'),
      'httpssCssZssZsgithubsDscomsZsasZsb'
   )
   check('plugin_dir.other', plugins.compute_repo_dir('a b'), 'au32b')
   check(
      'plugin_dir.resurrect',
      plugins.compute_repo_dir('https://github.com/MLFlexer/resurrect.wezterm'),
      'httpssCssZssZsgithubsDscomsZsMLFlexersZsresurrectsDswezterm'
   )
end

-- 新标签按钮右键列表：launch_menu 各项按自己的 domain 启动，默认 Shell 标注一次，
-- 末项打开设置页的默认 Shell 分区。
do
   package.loaded['events.new-tab-button'] = nil
   local loaded, new_tab = pcall(require, 'events.new-tab-button')
   check('new_tab.loads', loaded, true)
   if loaded then
      local choices, choices_data = new_tab.build_choices({}, {})
      local marked = 0
      for _, choice in ipairs(choices) do
         if choice.label:find('（默认）', 1, true) then
            marked = marked + 1
         end
      end
      check('new_tab.default_marked_once', marked, 1)
      check('new_tab.settings_last', choices[#choices].id, 'open-settings')
      check('new_tab.settings_label', choices[#choices].label:find('设为默认 Shell…', 1, true) ~= nil, true)
      check('new_tab.entries', #choices_data, #require('config.launch').launch_menu)
      for idx, data in ipairs(choices_data) do
         check('new_tab.domain.' .. idx, type(data.domain), 'table')
      end
   end
   -- 末项用的按键动作要随 WezTerm GX 本体一起发布（0.2.0 新增）；旧二进制上这一项会失败。
   check('new_tab.settings_action_known', pcall(function()
      return wezterm.action.ShowDefaultShellSettings
   end), true)
end

-- gui-settings.json 与 Rust（config/src/gui_settings.rs::settings_file_in_dir）同一位置：
-- 配置目录不是 HOME 时在它旁边，~/.wezterm.lua 时退回 XDG 配置目录。
do
   local gui_settings = require('utils.gui-settings')
   check(
      'settings.next_to_config',
      gui_settings.path(wezterm.config_dir, wezterm.home_dir, os.getenv),
      wezterm.config_dir .. '/gui-settings.json'
   )
   local function no_env()
      return nil
   end
   check(
      'settings.home_config',
      gui_settings.path(wezterm.home_dir, wezterm.home_dir, no_env),
      wezterm.home_dir .. '/.config/wezterm/gui-settings.json'
   )
end

-- Windows 免确认关闭名单：保留上游默认项，补上带 .exe 的 shell；herdr 仍需确认。
do
   local function load_general(os_name)
      local saved = package.loaded['utils.platform']
      package.loaded['utils.platform'] = { os = os_name, is_win = os_name == 'windows', is_linux = os_name == 'linux' }
      package.loaded['config.general'] = nil
      local general = require('config.general')
      package.loaded['config.general'] = nil
      package.loaded['utils.platform'] = saved
      return general
   end
   local names = {}
   for _, name in ipairs(load_general('windows').skip_close_confirmation_for_processes_named or {}) do
      names[name] = true
   end
   local expected = {
      'bash', 'sh', 'zsh', 'fish', 'tmux', 'nu', 'nu.exe', 'cmd.exe', 'pwsh.exe', 'powershell.exe',
      'zsh.exe', 'bash.exe', 'sh.exe', 'fish.exe', 'gx-zsh.exe', 'env.exe',
      -- WSL 桥接进程：窗格里只剩它们时视为空闲
      'wsl.exe', 'wslhost.exe',
      -- GX Zsh 的 Powerlevel10k 在 zsh 下常驻（安装包 lib/gitstatus 里的文件名，无扩展名）
      'gitstatusd-msys_nt-10.0-x86_64',
   }
   for _, name in ipairs(expected) do
      check('skip_close.windows.' .. name, names[name], true)
   end
   check('skip_close.windows.herdr', names['herdr.exe'], nil)
   check('skip_close.linux_upstream_default', load_general('linux').skip_close_confirmation_for_processes_named, nil)
end

-- Windows 字体文件探测（utils/font-files.lua）、终端字体回退链（config/fonts.lua）与
-- 界面字体（config/appearance.lua）：回退项只在字体文件存在时加入。
do
   local font_files = require('utils.font-files')
   local function env_of(values)
      return function(name)
         return values[name]
      end
   end
   local gx_exe_dir = 'C:\\Users\\x\\AppData\\Local\\Programs\\GXShell\\wezterm'
   local dirs = font_files.windows_font_dirs(
      env_of({ SystemRoot = 'D:\\Win', LOCALAPPDATA = 'C:\\Users\\x\\AppData\\Local' }),
      gx_exe_dir
   )
   check('font_dirs.count', #dirs, 3)
   check('font_dirs.system', dirs[1], 'D:\\Win\\Fonts\\')
   check('font_dirs.user', dirs[2], 'C:\\Users\\x\\AppData\\Local\\Microsoft\\Windows\\Fonts\\')
   check('font_dirs.gx', dirs[3], 'C:\\Users\\x\\AppData\\Local\\Programs\\GXShell\\fonts\\')
   dirs = font_files.windows_font_dirs(env_of({}), nil)
   check('font_dirs.default_count', #dirs, 1)
   check('font_dirs.default_system', dirs[1], 'C:\\Windows\\Fonts\\')
   dirs = font_files.windows_font_dirs(error, nil)
   check('font_dirs.getenv_error', #dirs, 1)

   local present = {}
   local function exists(path)
      return present[path] == true
   end
   local dir_a, dir_b = 'C:\\Windows\\Fonts\\', 'C:\\Users\\x\\Fonts\\'
   local function any_exists(name)
      return font_files.any_exists({ dir_a, dir_b }, { name }, exists)
   end
   check('font_files.none', any_exists('msyh.ttc'), false)
   present[dir_b .. 'msyh.ttc'] = true
   check('font_files.second_dir', any_exists('msyh.ttc'), true)
   check('font_files.other_name', any_exists('a.ttf'), false)

   local function chain_of(files)
      present = {}
      for _, name in ipairs(files) do
         present[dir_a .. name] = true
      end
      return font_files.ui_font_chain({ dir_a }, exists)
   end
   local chain = chain_of({ 'segoeuisb.ttf', 'msyh.ttc', 'seguiemj.ttf' })
   check('ui_chain.full_count', #chain, 3)
   check('ui_chain.semibold', chain[1].family .. ':' .. chain[1].weight, 'Segoe UI:DemiBold')
   check('ui_chain.yahei_ui', chain[2], 'Microsoft YaHei UI')
   check('ui_chain.emoji', chain[3], 'Segoe UI Emoji')
   chain = chain_of({ 'segoeui.ttf' })
   check('ui_chain.regular_only', #chain == 1 and chain[1].weight, 'Regular')
   check('ui_chain.empty', #chain_of({}), 0)
   -- 标题栏/标签栏要非粗体：即使有半粗文件也取 Regular，后备项不变；只有半粗文件时不加 Segoe UI
   present = {
      [dir_a .. 'segoeuisb.ttf'] = true,
      [dir_a .. 'segoeui.ttf'] = true,
      [dir_a .. 'msyh.ttc'] = true,
   }
   chain = font_files.ui_font_chain({ dir_a }, exists, 'Regular')
   check(
      'ui_chain.regular_requested',
      chain[1].family .. ':' .. chain[1].weight,
      'Segoe UI:Regular'
   )
   check('ui_chain.regular_keeps_fallbacks', chain[2], 'Microsoft YaHei UI')
   check(
      'ui_chain.demibold_default',
      font_files.ui_font_chain({ dir_a }, exists)[1].weight,
      'DemiBold'
   )
   present = { [dir_a .. 'segoeuisb.ttf'] = true }
   check(
      'ui_chain.regular_needs_regular_file',
      #font_files.ui_font_chain({ dir_a }, exists, 'Regular'),
      0
   )

   -- 把平台伪装成 Windows、字体文件系统换成 files 清单，加载一个配置模块
   local function load_windows(module_name, files)
      local saved_platform, saved_readable = package.loaded['utils.platform'], font_files.readable
      package.loaded['utils.platform'] =
         { os = 'windows', is_win = true, is_linux = false, is_mac = false }
      font_files.readable = function(path)
         return files[path] == true
      end
      package.loaded[module_name] = nil
      local loaded, options = pcall(require, module_name)
      package.loaded[module_name] = nil
      package.loaded['utils.platform'] = saved_platform
      font_files.readable = saved_readable
      check('windows_fonts.load.' .. module_name, loaded, true)
      return loaded and options or { font = { font = {} }, window_frame = {} }
   end
   local function families_of(options)
      local families = {}
      for _, entry in ipairs(options.font.font) do
         table.insert(families, entry.family)
      end
      return table.concat(families, '|')
   end
   local sys = (os.getenv('SystemRoot') or 'C:\\Windows') .. '\\Fonts\\'
   local function in_sys(...)
      local files = {}
      for _, name in ipairs({ ... }) do
         files[sys .. name] = true
      end
      return files
   end

   -- 终端字体回退链：JetBrainsMono NF → Noto Sans CJK SC → Segoe UI Emoji；
   -- 微软雅黑只在找不到 Noto Sans CJK 文件时补进来（排在 emoji 前）
   local noto, yahei, emoji = 'NotoSansCJK-Regular.ttc', 'msyh.ttc', 'seguiemj.ttf'
   local base = 'JetBrainsMono NF|Noto Sans CJK SC'
   local fonts = load_windows('config.fonts', in_sys(yahei, emoji))
   check('fonts.win_without_noto', families_of(fonts), base .. '|Microsoft YaHei|Segoe UI Emoji')
   fonts = load_windows('config.fonts', in_sys(noto, yahei, emoji))
   check('fonts.win_with_noto', families_of(fonts), base .. '|Segoe UI Emoji')
   check('fonts.win_no_system_fonts', families_of(load_windows('config.fonts', {})), base)
   check('fonts.emoji_presentation', fonts.font.font[3].assume_emoji_presentation, true)
   -- 与默认值相同的 freetype 目标不再显式设置
   check('fonts.no_freetype_load_target', fonts.freetype_load_target, nil)
   check('fonts.no_freetype_render_target', fonts.freetype_render_target, nil)

   -- 界面字体：Windows 上有 Segoe UI 等字体文件才设置，一项都没有就保持上游默认
   local ui = load_windows('config.appearance', in_sys('segoeuisb.ttf', 'segoeui.ttf', yahei, emoji))
   check('appearance_fonts.frame_font_first', ui.window_frame.font.font[1].family, 'Segoe UI')
   check('appearance_fonts.frame_font_size', ui.window_frame.font_size, 10)
   -- 标题栏/标签栏非粗体，浮层半粗
   check('appearance_fonts.frame_font_regular', ui.window_frame.font.font[1].weight, 'Regular')
   check(
      'appearance_fonts.palette_font_demibold',
      ui.command_palette_font.font[1].weight,
      'DemiBold'
   )
   check('appearance_fonts.palette_font', ui.command_palette_font ~= nil, true)
   check('appearance_fonts.char_select_font', ui.char_select_font ~= nil, true)
   check('appearance_fonts.pane_select_font', ui.pane_select_font ~= nil, true)
   ui = load_windows('config.appearance', {})
   check('appearance_fonts.none_frame_font', ui.window_frame.font, nil)
   check('appearance_fonts.none_frame_font_size', ui.window_frame.font_size, nil)
   check('appearance_fonts.none_palette_font', ui.command_palette_font, nil)
end

-- 窗口外观（config/appearance.lua）：Windows 用集成标题栏按钮 + fancy tab bar，标题栏/标签栏
-- 底色与细边框取 Mocha；其他平台保持系统装饰。
do
   local colors = require('colors.custom')
   local mocha = colors.mocha
   local font_files = require('utils.font-files')
   local function load_appearance(os_name)
      local saved_platform, saved_readable = package.loaded['utils.platform'], font_files.readable
      package.loaded['utils.platform'] = {
         os = os_name,
         is_win = os_name == 'windows',
         is_linux = os_name == 'linux',
         is_mac = os_name == 'mac',
      }
      font_files.readable = function()
         return false
      end
      package.loaded['config.appearance'] = nil
      local loaded, options = pcall(require, 'config.appearance')
      package.loaded['config.appearance'] = nil
      package.loaded['utils.platform'] = saved_platform
      font_files.readable = saved_readable
      check('window_chrome.load.' .. os_name, loaded, true)
      return loaded and options or { window_frame = {} }
   end

   local win = load_appearance('windows')
   check('window_chrome.win.decorations', win.window_decorations, 'INTEGRATED_BUTTONS|RESIZE')
   check('window_chrome.win.button_style', win.integrated_title_button_style, 'Windows')
   check('window_chrome.win.button_alignment', win.integrated_title_button_alignment, 'Right')
   check('window_chrome.win.button_color', win.integrated_title_button_color, 'auto')
   local linux = load_appearance('linux')
   check('window_chrome.linux.decorations', linux.window_decorations, nil)
   check('window_chrome.linux.button_style', linux.integrated_title_button_style, nil)
   check('window_chrome.linux.button_alignment', linux.integrated_title_button_alignment, nil)
   check('window_chrome.linux.button_color', linux.integrated_title_button_color, nil)

   for _, options in ipairs({ win, linux }) do
      local frame = options.window_frame
      check('window_chrome.fancy_tab_bar', options.use_fancy_tab_bar, true)
      check('window_chrome.titlebar_bg', frame.active_titlebar_bg, mocha.crust)
      check('window_chrome.inactive_titlebar_bg', frame.inactive_titlebar_bg, mocha.crust)
      check('window_chrome.titlebar_fg', frame.active_titlebar_fg, mocha.text)
      check('window_chrome.inactive_titlebar_fg', frame.inactive_titlebar_fg, mocha.overlay0)
      for _, key in ipairs({
         'border_left_width',
         'border_right_width',
         'border_top_height',
         'border_bottom_height',
      }) do
         check('window_chrome.' .. key, frame[key], '1px')
      end
      for _, key in ipairs({
         'border_left_color',
         'border_right_color',
         'border_top_color',
         'border_bottom_color',
      }) do
         check('window_chrome.' .. key, frame[key], mocha.surface1)
      end
   end
end

-- 首窗按屏幕自适应（events/gui-startup.lua）：约 80% 并居中；屏幕信息不可用时保持默认大小。
do
   package.loaded['events.gui-startup'] = nil
   local startup = require('events.gui-startup')
   local geometry = startup.centered_geometry
   local function describe(g)
      return g and string.format('%dx%d@%d,%d', g.width, g.height, g.x, g.y) or 'nil'
   end

   local cases = {
      { 'qhd', { x = 0, y = 0, width = 2560, height = 1440 }, 0.8, '2048x1152@256,144' },
      -- 副屏在主屏左侧（负坐标）：居中要加上屏幕自己的原点
      { 'offset', { x = -1920, y = 0, width = 1920, height = 1080 }, 0.8, '1536x864@-1728,108' },
      { 'missing_origin', { width = 1000, height = 1000 }, 0.5, '640x500@180,250' },
      -- 小屏：不低于最小尺寸，但不超过屏幕
      { 'small_screen', { x = 0, y = 0, width = 500, height = 300 }, 0.8, '500x300@0,0' },
      { 'min_size', { x = 0, y = 0, width = 700, height = 600 }, 0.5, '640x400@30,100' },
      { 'nil_screen', nil, 0.8, 'nil' },
      { 'zero_size', { x = 0, y = 0, width = 0, height = 1080 }, 0.8, 'nil' },
      { 'bad_type', { width = '1920', height = 1080 }, 0.8, 'nil' },
   }
   for _, case in ipairs(cases) do
      check('startup.' .. case[1], describe(geometry(case[2], case[3])), case[4])
   end

   local function fake_window(gui_window)
      return {
         gui_window = function()
            return gui_window
         end,
      }
   end
   local calls = {}
   local gui_window = {
      set_inner_size = function(_, w, h)
         table.insert(calls, string.format('size %d %d', w, h))
      end,
      set_position = function(_, x, y)
         table.insert(calls, string.format('pos %d %d', x, y))
      end,
   }
   local screen = { x = 0, y = 0, width = 2560, height = 1440 }
   check('startup.fit_ok', startup.fit_to_screen(fake_window(gui_window), screen), true)
   check('startup.fit_calls', table.concat(calls, ';'), 'size 2048 1152;pos 256 144')
   -- 拿不到 GUI 窗口（不在活动工作区）或屏幕信息：什么都不做
   check('startup.fit_no_gui_window', startup.fit_to_screen(fake_window(nil), screen), false)
   check('startup.fit_no_screen', startup.fit_to_screen(fake_window(gui_window), nil), false)
   check('startup.fit_untouched', #calls, 2)
end

-- 帧率与动画（utils/display.lua、config/appearance.lua）：max_fps 60 兜底、活动屏幕刷新率
-- 更高时跟随；动画帧率 60，光标与闪烁文字缓动都是 Constant（空闲零重绘）。
do
   local max_fps = require('utils.display').max_fps
   local function reads(value)
      return function()
         return value
      end
   end
   check('display.follows_higher', max_fps(60, reads(165)), 165)
   check('display.equal_base', max_fps(60, reads(60)), 60)
   check('display.below_base', max_fps(60, reads(30)), 60)
   check('display.unknown_rate', max_fps(60, reads(nil)), 60)
   check('display.not_a_number', max_fps(60, reads('144')), 60)
   check('display.read_error', max_fps(60, error), 60)
   check('display.nan', max_fps(60, reads(0 / 0)), 60)
   check('display.float_floored', max_fps(60, reads(143.9)), 143)
   -- 超过 Rust 侧 validate_max_fps 的上限 1000 会让整份配置报错
   check('display.clamped', max_fps(60, reads(5000)), 1000)
   check('display.infinite_clamped', max_fps(60, reads(math.huge)), 1000)

   local appearance = require('config.appearance')
   -- 测试环境读不到屏幕（或屏幕刷新率不高于 60），退回 60 兜底
   check('appearance.max_fps_floor', appearance.max_fps >= 60, true)
   check('appearance.animation_fps', appearance.animation_fps, 60)
   for _, key in ipairs({
      'cursor_blink_ease_in',
      'cursor_blink_ease_out',
      'text_blink_ease_in',
      'text_blink_ease_out',
      'text_blink_rapid_ease_in',
      'text_blink_rapid_ease_out',
   }) do
      check('appearance.' .. key, appearance[key], 'Constant')
   end
end

-- fork 新增配置键（config/fluent.lua）：逐键探测当前二进制认不认得，认得才写入；
-- 顶层键已被设置时不覆盖。键名与值类型的严格校验只在认得新键的构建上做（旧二进制跳过）。
do
   local fluent = require('config.fluent')
   local function accept_all()
      return true
   end
   local function reject_all()
      return false
   end

   local out = fluent.apply({ max_fps = 60 }, { supports = accept_all, is_win = true })
   check('fluent.win_follows_display', out.max_fps_follows_display, true)
   check('fluent.keeps_existing_keys', out.max_fps, 60)
   out = fluent.apply({}, { supports = accept_all, is_win = false })
   check('fluent.non_win_no_follows_display', out.max_fps_follows_display, nil)
   out = fluent.apply({}, { supports = reject_all, is_win = true })
   check('fluent.rejected_is_skipped', out.max_fps_follows_display, nil)
   out = fluent.apply({ max_fps_follows_display = false }, { supports = accept_all, is_win = true })
   check('fluent.no_override', out.max_fps_follows_display, false)

   -- 真实探测：老键认得，未知键与类型不符不认得
   check('fluent.probe.known_key', fluent.supported('max_fps', 60), true)
   check('fluent.probe.unknown_key', fluent.supported('no_such_config_key', true), false)
   check('fluent.probe.bad_type', fluent.supported('max_fps', 'fast'), false)

   -- 哨兵键：认得它就是本分支的新构建，登记的键必须全部合法
   local new_build = fluent.supported('max_fps_follows_display', true)
   if new_build then
      for key, value in pairs(fluent.declared(true).top) do
         check('fluent.declared.top.' .. key, fluent.supported(key, value), true)
      end
      for key, value in pairs(fluent.declared(true).colors) do
         check(
            'fluent.declared.colors.' .. key,
            fluent.supported('colors', { [key] = value }),
            true
         )
      end
   else
      print(
         'PURE_FN_TEST NOTE: 二进制不认得 fork 新增配置键，跳过 fluent 键名校验（需新构建复核）'
      )
   end
   out = fluent.apply({}, { is_win = true })
   check('fluent.apply_matches_binary', out.max_fps_follows_display, new_build or nil)
end

-- 键位：Windows 与 Linux 同一套（除 Linux 专属的截图/AI 图片粘贴），不占裸 Alt，
-- 不绑 Ctrl+B/C/V，翻页要 Shift。
do
   local function load_bindings(os_name)
      local saved_platform, saved_plugins = package.loaded['utils.platform'], package.loaded['config.plugins']
      package.loaded['utils.platform'] = {
         os = os_name,
         is_win = os_name == 'windows',
         is_linux = os_name == 'linux',
         is_mac = os_name == 'mac',
      }
      package.loaded['config.plugins'] = {
         resurrect_available = true,
         resurrect = function()
            return nil
         end,
      }
      package.loaded['config.bindings'] = nil
      local loaded, bindings = pcall(require, 'config.bindings')
      package.loaded['config.bindings'] = nil
      package.loaded['utils.platform'] = saved_platform
      package.loaded['config.plugins'] = saved_plugins
      check('bindings.load.' .. os_name, loaded, true)
      return loaded and bindings or { keys = {} }
   end

   local function normalized_mods(mods)
      local parts = {}
      for part in (mods or 'NONE'):gmatch('[^|%s]+') do
         table.insert(parts, part)
      end
      table.sort(parts)
      return table.concat(parts, '|')
   end

   local function index(bindings, os_name)
      local set = {}
      for _, binding in ipairs(bindings.keys) do
         local id = binding.key .. ' ' .. normalized_mods(binding.mods)
         check('bindings.unique.' .. os_name .. '.' .. id, set[id], nil)
         set[id] = binding
      end
      return set
   end

   local linux_only = { ['S ALT|SHIFT'] = true, ['V ALT|SHIFT'] = true }
   local sets = {}
   for _, os_name in ipairs({ 'windows', 'linux' }) do
      local set = index(load_bindings(os_name), os_name)
      sets[os_name] = set
      for id, binding in pairs(set) do
         local mods = normalized_mods(binding.mods)
         if mods == 'ALT' or mods == 'ALT|SHIFT' then
            check('bindings.bare_alt.' .. os_name .. '.' .. id, os_name == 'linux' and linux_only[id], true)
         end
         if mods == 'CTRL' then
            local key = binding.key:lower()
            check('bindings.ctrl_' .. key .. '.' .. os_name, key == 'b' or key == 'c' or key == 'v', false)
         end
      end
      check('bindings.page_up_shift.' .. os_name, set['PageUp SHIFT'] ~= nil, true)
      check('bindings.page_down_shift.' .. os_name, set['PageDown SHIFT'] ~= nil, true)
      check('bindings.page_up_plain.' .. os_name, set['PageUp NONE'], nil)
      check('bindings.page_down_plain.' .. os_name, set['PageDown NONE'], nil)
      local close_pane = set['w CTRL|SHIFT']
      check('bindings.close_pane.' .. os_name, close_pane and close_pane.action.CloseCurrentPane.confirm, true)
      local close_tab = set['w ALT|CTRL|SHIFT']
      check('bindings.close_tab.' .. os_name, close_tab and close_tab.action.CloseCurrentTab.confirm, true)
      local shifted = { '{', '}', '|', ')', '(' }
      for _, key in ipairs(shifted) do
         check('bindings.shifted.' .. os_name .. '.' .. key, set[key .. ' CTRL|SHIFT'] ~= nil, true)
      end
      check('bindings.shifted_rev.' .. os_name, set['{ ALT|CTRL|SHIFT'] ~= nil, true)
      -- Ctrl+_（Ctrl+Shift+-）是 readline/zsh/emacs/nano 的撤销，必须交给应用；
      -- Ctrl++（Ctrl+Shift+=）是字号放大；窗口缩放挂 leader 层。
      for _, id in ipairs({ '_ CTRL|SHIFT', '- CTRL|SHIFT', '= CTRL|SHIFT' }) do
         check('bindings.free.' .. os_name .. '.' .. id, set[id], nil)
      end
      local ctrl_plus = set['+ CTRL|SHIFT']
      check('bindings.ctrl_plus.' .. os_name, ctrl_plus and ctrl_plus.action, 'IncreaseFontSize')
      check('bindings.window_shrink.' .. os_name, set['- LEADER'] ~= nil, true)
      check('bindings.window_grow.' .. os_name, set['= LEADER'] ~= nil, true)
      for index_key = 1, 9 do
         check('bindings.leader_tab.' .. os_name .. '.' .. index_key, set[index_key .. ' LEADER'] ~= nil, true)
      end
      check('bindings.ctrl_page_up.' .. os_name, set['PageUp CTRL'] ~= nil, true)
      check('bindings.font_reset.' .. os_name, set['0 CTRL'] ~= nil, true)
   end
   for id in pairs(sets.linux) do
      check('bindings.windows_has.' .. id, sets.windows[id] ~= nil or linux_only[id] == true, true)
   end
   for id in pairs(sets.windows) do
      check('bindings.linux_has.' .. id, sets.linux[id] ~= nil, true)
   end

   local mac = index(load_bindings('mac'), 'mac')
   check('bindings.mac.close_pane', mac['w SUPER'] ~= nil, true)
   check('bindings.mac.page_up', mac['PageUp NONE'] ~= nil, true)
   check('bindings.mac.line_start', mac['LeftArrow SUPER'] ~= nil, true)
   check('bindings.mac.window_grow', mac['= LEADER'] ~= nil, true)
end

-- 防回归：wezterm.lua 里 append 进 Config 的每个模块，每个键都必须是合法的
-- wezterm Config 字段。纯表配置按 unknown_fields=Warn 转换，未知键不会报错，
-- 只会在下次 wezterm-gui 启动时弹 Configuration Error 窗口；这里借严格模式的
-- config_builder() 逐键赋值，未知键会被拒绝。模块清单直接从 wezterm.lua 的
-- `:append(require('...'))` 解析，新增模块自动纳入。
local function appended_config_modules()
   local file = io.open(config_root .. '/wezterm.lua', 'r')
   if not file then
      return {}
   end
   local source = file:read('a')
   file:close()
   local modules = {}
   for name in source:gmatch(":append%(%s*require%(%s*'([%w%._%-]+)'%s*%)%s*%)") do
      table.insert(modules, name)
   end
   return modules
end

local config_modules = appended_config_modules()
check('config_keys.modules_found', #config_modules > 0, true)
for _, module_name in ipairs(config_modules) do
   local loaded, options = pcall(require, module_name)
   check('config_keys.load.' .. module_name, loaded, true)
   if loaded and type(options) == 'table' then
      local builder = wezterm.config_builder()
      for key, value in pairs(options) do
         local accepted = pcall(function()
            builder[key] = value
         end)
         check('config_keys.' .. module_name .. '.' .. tostring(key), accepted, true)
      end
   end
end

-- resurrect 只在会话保存/恢复键触发时加载（加载会用 git 打开全部插件仓库）。
check('plugins.resurrect_lazy', package.loaded['httpssCssZssZsgithubsDscomsZsMLFlexersZsresurrectsDswezterm'], nil)

if failures == 0 then
   print(string.format('PURE_FN_TEST: ALL PASS (%d cases)', total))
else
   print(string.format('PURE_FN_TEST: %d/%d FAILED', failures, total))
end

-- 保持这是一个能被 --config-file 加载的合法配置返回值。
return {}
