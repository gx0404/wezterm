------------------------------------------------------------------------------------------
-- Inspired by https://github.com/wez/wezterm/discussions/628#discussioncomment-1874614 --
------------------------------------------------------------------------------------------

local wezterm = require('wezterm')
local Cells = require('utils.cells')
local OptsValidator = require('utils.opts-validator')
local cache = require('utils.cache')

---
-- =======================================
-- Defining event setup options and schema
-- =======================================

---@alias Event.TabTitleOptions { unseen_icon: 'circle' | 'numbered_circle' | 'numbered_box', hide_active_tab_unseen: boolean }

---Setup options for the tab title
local EVENT_OPTS = {}

---@type OptsSchema
EVENT_OPTS.schema = {
   {
      name = 'unseen_icon',
      type = 'string',
      enum = { 'circle', 'numbered_circle', 'numbered_box' },
      default = 'circle',
   },
   {
      name = 'hide_active_tab_unseen',
      type = 'boolean',
      default = true,
   },
}
EVENT_OPTS.validator = OptsValidator:new(EVENT_OPTS.schema)

---
-- ===================
-- Constants and icons
-- ===================

local nf = wezterm.nerdfonts

local M = {}

local GLYPH_SCIRCLE_LEFT = nf.ple_left_half_circle_thick --[[  ]]
local GLYPH_SCIRCLE_RIGHT = nf.ple_right_half_circle_thick --[[  ]]
local GLYPH_CIRCLE = nf.fa_circle --[[  ]]
local GLYPH_ADMIN = nf.md_shield_half_full --[[ 󰞀 ]]
local GLYPH_LINUX = nf.cod_terminal_linux --[[  ]]
local GLYPH_DEBUG = nf.fa_bug --[[  ]]
-- local GLYPH_SEARCH = nf.fa_search --[[  ]]
local GLYPH_SEARCH = '🔭'

local GLYPH_UNSEEN_NUMBERED_BOX = {
   [1] = nf.md_numeric_1_box_multiple, --[[ 󰼏 ]]
   [2] = nf.md_numeric_2_box_multiple, --[[ 󰼐 ]]
   [3] = nf.md_numeric_3_box_multiple, --[[ 󰼑 ]]
   [4] = nf.md_numeric_4_box_multiple, --[[ 󰼒 ]]
   [5] = nf.md_numeric_5_box_multiple, --[[ 󰼓 ]]
   [6] = nf.md_numeric_6_box_multiple, --[[ 󰼔 ]]
   [7] = nf.md_numeric_7_box_multiple, --[[ 󰼕 ]]
   [8] = nf.md_numeric_8_box_multiple, --[[ 󰼖 ]]
   [9] = nf.md_numeric_9_box_multiple, --[[ 󰼗 ]]
   [10] = nf.md_numeric_9_plus_box_multiple, --[[ 󰼘 ]]
}

local GLYPH_UNSEEN_NUMBERED_CIRCLE = {
   [1] = nf.md_numeric_1_circle, --[[ 󰲠 ]]
   [2] = nf.md_numeric_2_circle, --[[ 󰲢 ]]
   [3] = nf.md_numeric_3_circle, --[[ 󰲤 ]]
   [4] = nf.md_numeric_4_circle, --[[ 󰲦 ]]
   [5] = nf.md_numeric_5_circle, --[[ 󰲨 ]]
   [6] = nf.md_numeric_6_circle, --[[ 󰲪 ]]
   [7] = nf.md_numeric_7_circle, --[[ 󰲬 ]]
   [8] = nf.md_numeric_8_circle, --[[ 󰲮 ]]
   [9] = nf.md_numeric_9_circle, --[[ 󰲰 ]]
   [10] = nf.md_numeric_9_plus_circle, --[[ 󰲲 ]]
}

local TITLE_INSET = {
   DEFAULT = 6,
   ICON = 8,
}

-- format-tab-title 每个 tab 每帧至少调用两次（量宽、绘制），而 foreground_process_name
-- 是惰性字段，每次读取都要向系统查询进程（Windows 上要快照进程树）。同一 tab 同一
-- 窗格的前台进程名最多每隔这么多秒查一次。
local PROBE_INTERVAL_S = 2

local RENDER_VARIANTS = {
   { 'scircle_left', 'title', 'padding', 'scircle_right' },
   { 'scircle_left', 'title', 'unseen_output', 'padding', 'scircle_right' },
   { 'scircle_left', 'admin', 'title', 'padding', 'scircle_right' },
   { 'scircle_left', 'admin', 'title', 'unseen_output', 'padding', 'scircle_right' },
   { 'scircle_left', 'wsl', 'title', 'padding', 'scircle_right' },
   { 'scircle_left', 'wsl', 'title', 'unseen_output', 'padding', 'scircle_right' },
}


---@type table<string, Cells.SegmentColors>
-- stylua: ignore
local colors = {
   text_default          = { bg = '#313244', fg = '#CDD6F4' },
   text_hover            = { bg = '#45475A', fg = '#FFFFFF' },
   text_active           = { bg = '#89B4FA', fg = '#11111B' },

   unseen_output_default = { bg = '#313244', fg = '#FAB387' },
   unseen_output_hover   = { bg = '#45475A', fg = '#FAB387' },
   unseen_output_active  = { bg = '#89B4FA', fg = '#F38BA8' },

   scircle_default       = { bg = 'rgba(17, 17, 27, 0.88)', fg = '#313244' },
   scircle_hover         = { bg = 'rgba(17, 17, 27, 0.88)', fg = '#45475A' },
   scircle_active        = { bg = 'rgba(17, 17, 27, 0.88)', fg = '#89B4FA' },
}

---
-- ================
-- Helper functions
-- ================

---@param proc string
local function clean_process_name(proc)
   local a = string.gsub(proc, '(.*[/\\])(.*)', '%2')
   return a:gsub('%.exe$', '')
end

-- 导出：events/status.lua 的 herdr 应用模式判断复用同一套清洗口径，
-- 避免两处实现漂移；也供纯函数用例直接 require 测试。
M.clean_process_name = clean_process_name

---窗口装饰里是否带集成标题栏按钮。window_decorations 序列化成 'RESIZE|INTEGRATED_BUTTONS'
---这样的字符串（window_decorations 的 Rust 定义见 wezterm-input-types::WindowDecorations）。
---集成按钮画在标签栏里：标签栏一旦隐藏，窗口既没有最小化/最大化/关闭按钮也拖不动，
---所以 herdr 应用模式（events/status.lua）和手动切换都不能隐藏它。
---@param decorations any window:effective_config().window_decorations
---@return boolean
local function has_integrated_buttons(decorations)
   return type(decorations) == 'string' and decorations:find('INTEGRATED_BUTTONS', 1, true) ~= nil
end

M.has_integrated_buttons = has_integrated_buttons

---手动切换 tab bar 要写回的完整 overrides。纯函数：浅拷贝入参，只翻转
---enable_tab_bar 一个键，其余覆盖键原样保留；不再顺带写 background，避免把
---当时的壁纸固定成窗口覆盖（否则壁纸管理浮层选图重载后会被旧图顶回）。
---@param overrides table? 窗口当前的 config overrides（get_config_overrides()）
---@param effective_enable_tab_bar boolean 当前生效的 enable_tab_bar
---@return table
local function toggled_tab_bar_overrides(overrides, effective_enable_tab_bar)
   local new_overrides = {}
   for key, value in pairs(overrides or {}) do
      new_overrides[key] = value
   end
   new_overrides.enable_tab_bar = not effective_enable_tab_bar
   return new_overrides
end

M.toggled_tab_bar_overrides = toggled_tab_bar_overrides

---移除 Codex 等 TUI 写入窗口标题的 Braille spinner，保留稳定标题。
---@param title string?
local function stable_pane_title(title)
   title = title or ''
   -- 空标题（pane 刚创建、TUI 清空标题）时 utf8.codepoint(s, 1) 会抛
   -- "out of bounds"，连带 format-tab-title / format-window-title 整个失败。
   if title == '' then
      return title
   end
   local first = utf8.codepoint(title, 1)

   if first and first >= 0x2800 and first <= 0x28ff then
      local next_offset = utf8.offset(title, 2)
      title = next_offset and title:sub(next_offset) or ''
      title = title:gsub('^%s+', '')
   end

   return title
end

---@param process_name string
---@param base_title string
---@param max_width number
---@param inset number
local function create_title(process_name, base_title, max_width, inset)
   local title

   if process_name:len() > 0 then
      title = process_name .. ' ~ ' .. base_title
   else
      title = base_title
   end

   if base_title == 'Debug' then
      title = GLYPH_DEBUG .. ' DEBUG'
      inset = inset - 2
   end

   if base_title:match('^InputSelector:') ~= nil then
      title = base_title:gsub('InputSelector:', GLYPH_SEARCH)
      inset = inset - 2
   end

   local available = math.max(max_width - inset, 4)
   local width = wezterm.column_width(title)

   if width > available then
      title = wezterm.truncate_right(title, available - 1) .. '…'
      width = wezterm.column_width(title)
   end

   title = title .. string.rep(' ', math.max(available - width, 0))

   return title
end

---@param panes any[] WezTerm https://wezfurlong.org/wezterm/config/lua/pane/index.html
local function check_unseen_output(panes)
   local unseen_output = false
   local unseen_output_count = 0

   for i = 1, #panes, 1 do
      if panes[i].has_unseen_output then
         unseen_output = true
         if unseen_output_count >= 10 then
            unseen_output_count = 10
            break
         end
         unseen_output_count = unseen_output_count + 1
      end
   end

   return unseen_output, unseen_output_count
end

---
-- =================
-- Tab class and API
-- =================

---@class Tab
---@field title string
---@field cells FormatCells
---@field window_id number? 所属窗口，回收已关闭 tab 时只动同一窗口的条目
---@field probe_at integer? 上次查前台进程名的时刻（os.time()）
---@field probe_pane_id number? 上次查的是哪个窗格
---@field probe_name string 上次查到的（已清洗的）前台进程名
---@field is_wsl boolean
---@field is_admin boolean
---@field unseen_output boolean
---@field unseen_output_count number
---@field is_active boolean
local Tab = {}
Tab.__index = Tab

function Tab:new()
   local tab = {
      title = '',
      cells = Cells:new(),
      probe_name = '',
      is_wsl = false,
      is_admin = false,
      unseen_output = false,
      unseen_output_count = 0,
   }
   return setmetatable(tab, self)
end

---前台进程名（已清洗）；同一窗格在 PROBE_INTERVAL_S 秒内复用上次结果，
---换了窗格（active_pane 变了）立即重查。
---@param pane any WezTerm PaneInformation
---@param now integer os.time()
---@return string
function Tab:probe_process_name(pane, now)
   if
      self.probe_at
      and self.probe_pane_id == pane.pane_id
      and cache.still_fresh(self.probe_at, now, PROBE_INTERVAL_S)
   then
      return self.probe_name
   end
   self.probe_at = now
   self.probe_pane_id = pane.pane_id
   self.probe_name = clean_process_name(pane.foreground_process_name or '')
   return self.probe_name
end

---@param event_opts Event.TabTitleOptions
---@param tab any WezTerm TabInformation https://wezfurlong.org/wezterm/config/lua/TabInformation.html
---@param max_width number
---@param now? integer os.time()，缺省取当前时间
function Tab:set_info(event_opts, tab, max_width, now)
   -- active_pane 在窗口刚创建等边界情况下可能为 nil
   local pane = tab.active_pane
   local process_name = pane and self:probe_process_name(pane, now or os.time()) or ''
   local base_title = stable_pane_title(pane and pane.title)

   self.window_id = tab.window_id
   self.is_wsl = process_name:match('^wsl') ~= nil
   self.is_admin = (
      base_title:match('^Administrator: ') or base_title:match('(Admin)')
   ) ~= nil
   self.unseen_output = false
   self.unseen_output_count = 0

   if not event_opts.hide_active_tab_unseen or not tab.is_active then
      self.unseen_output, self.unseen_output_count = check_unseen_output(tab.panes)
   end

   local inset = (self.is_admin or self.is_wsl) and TITLE_INSET.ICON or TITLE_INSET.DEFAULT
   if self.unseen_output then
      inset = inset + 2
   end

   -- 手动重命名保存在 mux 的 tab 上（tab:set_title），跨配置重载保留；
   -- 非空时优先于「进程名 ~ 窗格标题」的自动标题。
   local manual_title = tab.tab_title
   if manual_title ~= nil and manual_title ~= '' then
      self.title = create_title('', manual_title, max_width, inset)
      return
   end
   self.title = create_title(process_name, base_title, max_width, inset)
end

function Tab:create_cells()
   local attr = self.cells.attr
   self.cells
      :add_segment('scircle_left', GLYPH_SCIRCLE_LEFT)
      :add_segment('admin', ' ' .. GLYPH_ADMIN)
      :add_segment('wsl', ' ' .. GLYPH_LINUX)
      :add_segment('title', ' ', nil, attr(attr.intensity('Bold')))
      :add_segment('unseen_output', ' ' .. GLYPH_CIRCLE)
      :add_segment('padding', ' ')
      :add_segment('scircle_right', GLYPH_SCIRCLE_RIGHT)
end

---@param event_opts Event.TabTitleOptions
---@param is_active boolean
---@param hover boolean
function Tab:update_cells(event_opts, is_active, hover)
   local tab_state = 'default'
   if is_active then
      tab_state = 'active'
   elseif hover then
      tab_state = 'hover'
   end

   self.cells:update_segment_text('title', ' ' .. self.title)

   if event_opts.unseen_icon == 'numbered_box' and self.unseen_output then
      self.cells:update_segment_text(
         'unseen_output',
         ' ' .. GLYPH_UNSEEN_NUMBERED_BOX[self.unseen_output_count]
      )
   end
   if event_opts.unseen_icon == 'numbered_circle' and self.unseen_output then
      self.cells:update_segment_text(
         'unseen_output',
         ' ' .. GLYPH_UNSEEN_NUMBERED_CIRCLE[self.unseen_output_count]
      )
   end

   self.cells
      :update_segment_colors('scircle_left', colors['scircle_' .. tab_state])
      :update_segment_colors('admin', colors['text_' .. tab_state])
      :update_segment_colors('wsl', colors['text_' .. tab_state])
      :update_segment_colors('title', colors['text_' .. tab_state])
      :update_segment_colors('unseen_output', colors['unseen_output_' .. tab_state])
      :update_segment_colors('padding', colors['text_' .. tab_state])
      :update_segment_colors('scircle_right', colors['scircle_' .. tab_state])
end

---@return FormatItem[] (ref: https://wezfurlong.org/wezterm/config/lua/wezterm/format.html)
function Tab:render()
   local variant_idx = self.is_admin and 3 or 1
   if self.is_wsl then
      variant_idx = 5
   end

   if self.unseen_output then
      variant_idx = variant_idx + 1
   end
   return self.cells:render(RENDER_VARIANTS[variant_idx])
end

---@type table<number, Tab> tab_id -> 状态；窗口里的 tab 关闭后由 prune_closed_tabs 回收
local tab_list = {}

---回收已关闭 tab 的状态。tabs 是 format-tab-title 传入的、该窗口当前的全部 tab；
---tab_list 跨窗口共用，所以只回收属于同一窗口、却已不在 tabs 里的条目——动了其他
---窗口的条目，各窗口会轮流把对方的状态清掉，缓存形同虚设。
---@param window_id number? 当前 tab 所在窗口
---@param tabs any[] TabInformation 列表
---@param current_tab_id number 当前正在渲染的 tab（兜底保证它不会被清掉）
local function prune_closed_tabs(window_id, tabs, current_tab_id)
   local alive = { [current_tab_id] = true }
   for _, other in ipairs(tabs or {}) do
      alive[other.tab_id] = true
   end
   for id, state in pairs(tab_list) do
      if state.window_id == window_id and not alive[id] then
         tab_list[id] = nil
      end
   end
end

---format-tab-title 的核心：首次见到的 tab 与已有的 tab 走同一条路径（建好格子后照样
---更新标题与颜色），首帧就是完整标题，不会先画出一个被截断的空壳。
---@param event_opts Event.TabTitleOptions
---@param tab any WezTerm TabInformation
---@param tabs any[] 该窗口的全部 TabInformation
---@param hover boolean
---@param max_width number
---@param now? integer os.time()，缺省取当前时间
---@return FormatItem[]
local function render_tab(event_opts, tab, tabs, hover, max_width, now)
   prune_closed_tabs(tab.window_id, tabs, tab.tab_id)

   local state = tab_list[tab.tab_id]
   if not state then
      state = Tab:new()
      state:create_cells()
      tab_list[tab.tab_id] = state
   end
   state:set_info(event_opts, tab, max_width, now)
   state:update_cells(event_opts, tab.is_active, hover)
   return state:render()
end

---手动重命名：写进 mux 的 tab 标题（跨配置重载保留）；空串等于恢复自动标题。
---@param window any WezTerm Window
---@param line string?
local function apply_manual_title(window, line)
   if line == nil then
      return
   end
   local tab = window:active_tab()
   if tab then
      tab:set_title(line)
   end
end

M.render_tab = render_tab
M.apply_manual_title = apply_manual_title

-- 导出：供 tests/pure_fn_test.lua 观察回收结果。
---@return integer
M.tracked_tab_count = function()
   local count = 0
   for _ in pairs(tab_list) do
      count = count + 1
   end
   return count
end

---@param opts? Event.TabTitleOptions Default: {unseen_icon = 'circle', hide_active_tab_unseen = true}
M.setup = function(opts)
   local valid_opts, err = EVENT_OPTS.validator:validate(opts or {})

   if err then
      wezterm.log_error(err)
   end

   -- CUSTOM EVENT
   -- Event listener to manually update the tab name
   -- Tab name stays until `tabs.reset-tab-title` is triggered
   wezterm.on('tabs.manual-update-tab-title', function(window, pane)
      window:perform_action(
         wezterm.action.PromptInputLine({
            description = wezterm.format({
               { Foreground = { Color = '#FFFFFF' } },
               { Attribute = { Intensity = 'Bold' } },
               { Text = 'Enter new name for tab' },
            }),
            action = wezterm.action_callback(function(inner_window, _pane, line)
               apply_manual_title(inner_window, line)
            end),
         }),
         pane
      )
   end)

   -- CUSTOM EVENT
   -- Event listener to restore the automatic tab name
   wezterm.on('tabs.reset-tab-title', function(window, _pane)
      apply_manual_title(window, '')
   end)

   -- CUSTOM EVENT
   -- 手动切换 tab bar（与 events/status.lua 的 herdr 应用模式共用 enable_tab_bar 覆盖）。
   -- 集成标题栏按钮模式下标签栏就是标题栏，不允许隐藏。
   wezterm.on('tabs.toggle-tab-bar', function(window, _pane)
      local config = window:effective_config()
      if config.enable_tab_bar and has_integrated_buttons(config.window_decorations) then
         return
      end
      window:set_config_overrides(
         toggled_tab_bar_overrides(window:get_config_overrides(), config.enable_tab_bar)
      )
   end)

   -- 固定系统窗口标题；即使 pane 的 OSC title spinner 在变化，返回值也保持稳定。
   wezterm.on('format-window-title', function(tab, _pane, tabs, _panes, _config)
      local title = stable_pane_title(tab.active_pane.title)
      if title == '' then
         title = clean_process_name(tab.active_pane.foreground_process_name)
      end

      local zoomed = tab.active_pane.is_zoomed and '[Z] ' or ''
      local index = #tabs > 1 and string.format('[%d/%d] ', tab.tab_index + 1, #tabs) or ''
      return zoomed .. index .. title
   end)

   -- BUILTIN EVENT
   wezterm.on('format-tab-title', function(tab, tabs, _panes, _config, hover, max_width)
      return render_tab(valid_opts, tab, tabs, hover, max_width)
   end)
end

return M
