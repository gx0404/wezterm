local wezterm = require('wezterm')
local tab_title = require('events.tab-title')
local still_fresh = require('utils.cache').still_fresh
local mocha = require('colors.custom').mocha

local M = {}
local last_status_by_window = {}
-- herdr 应用模式的 per-window 状态存在 wezterm.GLOBAL 的这个键下（按
-- tostring(window_id) 分桶）。不能用模块局部表：配置重载会新建 Lua 状态、
-- 局部表清空，而窗口的 config overrides 跨重载保留，两者会失配。
local HERDR_TAB_BAR_GLOBAL_KEY = 'herdr_app_mode_tab_bar'

-- update-status 在 GUI 线程上跑，且标题变化（agent spinner）时远比 2 秒频繁：
-- 前台进程探测在 Windows 上要全量快照进程树，每窗口至多每 2 秒一次；
-- battery_info 每次都枚举电池设备，缓存 60 秒。
local PROBE_INTERVAL_S = 2
local BATTERY_TTL_S = 60
local probe_by_window = {}
local battery_cache = nil

-- window:effective_config() 要把整份配置转成 Lua 表，很重；window_decorations 只会随配置
-- 重载（重载会新建 Lua 状态、这个缓存随之清空）变化，缓存一段时间足够。
local DECORATIONS_TTL_S = 30
local decorations_by_window = {}

---回收已关闭窗口的状态，表项不随窗口开关无限增长。
---wezterm.gui 只在 GUI 进程里有；取不到窗口列表时本轮不回收，下一轮再试。
local function prune_closed_windows()
   local gui = wezterm.gui
   if not (gui and gui.gui_windows) then
      return
   end
   local ok, windows = pcall(gui.gui_windows)
   if not ok then
      return
   end
   local alive = {}
   for _, gui_window in ipairs(windows) do
      alive[gui_window:window_id()] = true
   end
   for id in pairs(last_status_by_window) do
      if not alive[id] then
         last_status_by_window[id] = nil
         probe_by_window[tostring(id)] = nil
         decorations_by_window[tostring(id)] = nil
      end
   end
end

local colors = {
   blue = mocha.blue,
   peach = mocha.peach,
   yellow = mocha.yellow,
   mauve = mocha.mauve,
}

---左右状态块的一段。不设背景色、也不加粗：fancy 标签栏里整块状态的底色取自第一个格子的
---背景，不设就用 window_frame 的标题栏底色，和 + 按钮、标签栏底色自然一致；字体是
---Regular 的 Segoe UI（config/appearance.lua），加粗会换成 Bold 字重。
local function segment(icon, text, color)
   return {
      { Foreground = { Color = color } },
      { Text = string.format(' %s %s ', icon, text) },
   }
end

---@param now integer os.time()
local function battery_segment(now)
   if battery_cache and still_fresh(battery_cache.at, now, BATTERY_TTL_S) then
      return battery_cache.items
   end
   local items = {}
   for _, battery in ipairs(wezterm.battery_info()) do
      local percent = math.floor(battery.state_of_charge * 100 + 0.5)
      local icon = battery.state == 'Charging' and wezterm.nerdfonts.md_battery_charging
         or wezterm.nerdfonts.md_battery
      items = segment(icon, string.format('%d%%', percent), colors.yellow)
      break
   end
   battery_cache = { at = now, items = items }
   return items
end

local function render_status(window, date_format, now)
   local left = {}
   local workspace = window:active_workspace()
   local key_table = window:active_key_table()

   for _, item in ipairs(segment(wezterm.nerdfonts.cod_terminal, workspace, colors.blue)) do
      table.insert(left, item)
   end

   if key_table then
      for _, item in ipairs(segment(wezterm.nerdfonts.md_table_key, string.upper(key_table), colors.mauve)) do
         table.insert(left, item)
      end
   elseif window:leader_is_active() then
      for _, item in ipairs(segment(wezterm.nerdfonts.md_key, 'LEADER', colors.mauve)) do
         table.insert(left, item)
      end
   end

   local right = {}
   for _, item in ipairs(battery_segment(now)) do
      table.insert(right, item)
   end
   for _, item in ipairs(segment(wezterm.nerdfonts.fa_clock_o, wezterm.strftime(date_format), colors.peach)) do
      table.insert(right, item)
   end

   return wezterm.format(left), wezterm.format(right)
end

---herdr 应用模式：是否应该隐藏宿主 tab bar。纯函数，不读取/修改任何
---window 或全局状态，只依据显式入参判断，可脱离 wezterm 运行时单独测试
---（见 tests/pure_fn_test.lua）。
---
---decorations 含 INTEGRATED_BUTTONS 时一律不隐藏：集成标题栏按钮画在 tab bar 里，隐藏后
---窗口没有关闭/最大化按钮也拖不动。force_with_integrated 为 true 时跳过这条保护，
---即使含集成按钮也按 herdr 单 tab 规则隐藏（用户明确选择隐藏 tab bar 时使用）。
---@param herdr_app_mode boolean
---@param tab_count number
---@param process_name string 已经过 clean_process_name 清洗的前台进程名
---@param decorations? string window_decorations 的字符串形式，如 'RESIZE|INTEGRATED_BUTTONS'；缺省按没有集成按钮处理
---@param force_with_integrated? boolean 为 true 时忽略集成按钮保护；缺省 false
---@return boolean
local function should_hide_tab_bar(herdr_app_mode, tab_count, process_name, decorations, force_with_integrated)
   if force_with_integrated ~= true and tab_title.has_integrated_buttons(decorations) then
      return false
   end
   return herdr_app_mode == true and tab_count == 1 and process_name == 'herdr'
end

---@class HerdrTabBarState
---@field hidden boolean 上次判定是否已由 herdr 应用模式隐藏 tab bar
---@field had_prev boolean 隐藏前窗口是否已有 enable_tab_bar 覆盖（如手动切换）
---@field prev boolean? had_prev 为 true 时，隐藏前的 enable_tab_bar 覆盖值

---herdr 应用模式的 tab bar 状态转移。纯函数：不读写 window / GLOBAL，入参
---overrides 也不修改（按需浅拷贝），可脱离 wezterm 运行时单独测试。
---
---只在「是否应隐藏」的判定翻转时才返回要写回的 overrides；判定不变时返回
---nil（no-op），因此手动 `tabs.toggle-tab-bar` 切出的状态会一直保留，直到
---前台进程 / tab 数量变化触发下一次翻转。写回时只改 enable_tab_bar 一个键，
---其余覆盖键（壁纸等）原样保留，不把当时的 base 配置固定成窗口覆盖。
---
---state 为 nil 表示本进程内首次见到该窗口：此时若要隐藏，一律按「隐藏前
---没有覆盖」记录，恢复时删掉 enable_tab_bar 回落 base 配置。这样部署本修复
---前旧实现遗留的 enable_tab_bar=false 覆盖，在退出 herdr 后也能自愈。
---@param state HerdrTabBarState?
---@param hide boolean 本轮判定是否应隐藏
---@param overrides table? 窗口当前的 config overrides（get_config_overrides()）
---@return HerdrTabBarState new_state 判定不变且 state 非 nil 时原样返回 state
---@return table? new_overrides 要写回的完整 overrides；nil 表示本轮不写
local function next_tab_bar_state(state, hide, overrides)
   local was_hidden = (state ~= nil and state.hidden) or false
   if was_hidden == hide then
      return state or { hidden = false, had_prev = false }, nil
   end

   local new_overrides = {}
   for key, value in pairs(overrides or {}) do
      new_overrides[key] = value
   end

   if hide then
      local new_state = { hidden = true, had_prev = false }
      if state ~= nil and new_overrides.enable_tab_bar ~= nil then
         new_state.had_prev = true
         new_state.prev = new_overrides.enable_tab_bar
      end
      new_overrides.enable_tab_bar = false
      return new_state, new_overrides
   end

   if state.had_prev then
      new_overrides.enable_tab_bar = state.prev
   else
      new_overrides.enable_tab_bar = nil
   end
   return { hidden = false, had_prev = false }, new_overrides
end

---从 wezterm.GLOBAL 读出某窗口的状态，转成普通 Lua 表（GLOBAL 返回的是
---共享 userdata 代理，不宜直接交给纯函数或长期持有）。
---@param key string tostring(window_id)
---@return HerdrTabBarState?
local function load_tab_bar_state(key)
   local all = wezterm.GLOBAL[HERDR_TAB_BAR_GLOBAL_KEY]
   local stored = all and all[key]
   if stored == nil then
      return nil
   end
   return { hidden = stored.hidden == true, had_prev = stored.had_prev == true, prev = stored.prev }
end

---@param key string tostring(window_id)
---@param state HerdrTabBarState
local function store_tab_bar_state(key, state)
   if wezterm.GLOBAL[HERDR_TAB_BAR_GLOBAL_KEY] == nil then
      wezterm.GLOBAL[HERDR_TAB_BAR_GLOBAL_KEY] = {}
   end
   wezterm.GLOBAL[HERDR_TAB_BAR_GLOBAL_KEY][key] = state
end

---窗口前台进程名（已清洗），同一窗口 PROBE_INTERVAL_S 秒内复用上次结果。
---@param key string tostring(window_id)
---@param pane any WezTerm Pane
---@param now integer os.time()
---@return string
local function foreground_process_name(key, pane, now)
   local cached = probe_by_window[key]
   if cached and still_fresh(cached.at, now, PROBE_INTERVAL_S) then
      return cached.name
   end
   local name = tab_title.clean_process_name(pane:get_foreground_process_name() or '')
   probe_by_window[key] = { at = now, name = name }
   return name
end

---窗口生效的 window_decorations（字符串），同一窗口 DECORATIONS_TTL_S 秒内复用上次结果。
---读不到（取配置失败）时返回空串且不缓存，按没有集成按钮处理。
---@param key string tostring(window_id)
---@param window any WezTerm GuiWindow
---@param now integer os.time()
---@return string
local function window_decorations(key, window, now)
   local cached = decorations_by_window[key]
   if cached and still_fresh(cached.at, now, DECORATIONS_TTL_S) then
      return cached.value
   end
   local ok, value = pcall(function()
      return window:effective_config().window_decorations
   end)
   if not ok or type(value) ~= 'string' then
      return ''
   end
   decorations_by_window[key] = { at = now, value = value }
   return value
end

---按 herdr 应用模式决定是否隐藏 tab bar；状态转移见 next_tab_bar_state。
---只在单 tab（隐藏的前提）时探测前台进程，且只在前台是 herdr（可能隐藏）时才取窗口装饰。
---@param window any WezTerm GuiWindow
---@param pane any? WezTerm Pane，可能为 nil（窗口刚创建等边界情况）
---@param herdr_app_mode boolean
---@param now? integer os.time()，缺省取当前时间
---@param force_with_integrated? boolean 透传给 should_hide_tab_bar；缺省 false
local function apply_herdr_app_mode(window, pane, herdr_app_mode, now, force_with_integrated)
   now = now or os.time()
   local key = tostring(window:window_id())
   local tab_count = #window:mux_window():tabs()
   local process_name = ''
   if pane and herdr_app_mode and tab_count == 1 then
      process_name = foreground_process_name(key, pane, now)
   end
   local decorations = ''
   if process_name == 'herdr' then
      decorations = window_decorations(key, window, now)
   end

   local hide = should_hide_tab_bar(herdr_app_mode, tab_count, process_name, decorations, force_with_integrated)
   local state = load_tab_bar_state(key)
   -- 判定不变时不必取 overrides（每次 update-status 都会走到这里）。
   local overrides = nil
   if ((state ~= nil and state.hidden) or false) ~= hide then
      overrides = window:get_config_overrides()
   end

   local new_state, new_overrides = next_tab_bar_state(state, hide, overrides)
   if new_state ~= state then
      store_tab_bar_state(key, new_state)
   end
   if new_overrides ~= nil then
      window:set_config_overrides(new_overrides)
   end
end

---@param opts? { date_format?: string, herdr_app_mode?: boolean, integrated_hides_tab_bar?: boolean } Default: {date_format = '%a %H:%M', herdr_app_mode = true, integrated_hides_tab_bar = false}；integrated_hides_tab_bar 为 true 时，即使 window_decorations 含 INTEGRATED_BUTTONS 也按 herdr 单 tab 规则隐藏 tab bar
M.setup = function(opts)
   local date_format = (opts and opts.date_format) or '%a %H:%M'
   local herdr_app_mode = true
   if opts and opts.herdr_app_mode ~= nil then
      herdr_app_mode = opts.herdr_app_mode
   end

   local force_with_integrated = opts ~= nil and opts.integrated_hides_tab_bar == true

   wezterm.on('update-status', function(window, pane)
      local now = os.time()
      local left, right = render_status(window, date_format, now)
      local window_id = window:window_id()
      prune_closed_windows()
      local previous = last_status_by_window[window_id]

      if not previous or previous.left ~= left then
         window:set_left_status(left)
      end
      if not previous or previous.right ~= right then
         window:set_right_status(right)
      end

      last_status_by_window[window_id] = { left = left, right = right }

      apply_herdr_app_mode(window, pane, herdr_app_mode, now, force_with_integrated)
   end)
end

-- 导出：供 tests/pure_fn_test.lua 单独驱动断言，无需起 GUI 窗口。
M.tracked_window_count = function()
   local count = 0
   for _ in pairs(last_status_by_window) do
      count = count + 1
   end
   return count
end
M.should_hide_tab_bar = should_hide_tab_bar
M.next_tab_bar_state = next_tab_bar_state
M.apply_herdr_app_mode = apply_herdr_app_mode

return M
