local wezterm = require('wezterm')
local platform = require('utils.platform')
local backdrops = require('utils.backdrops')
local colors = require('colors.custom')
local font_files = require('utils.font-files')
local display = require('utils.display')

-- 各平台的渲染后端只在这里调整。GNOME X11 下优先稳定性；Windows 未经实机 A/B
-- （paint 日志）前同样保持 OpenGL。WebGPU 仅在隔离配置中做 A/B 测试。
---@type table<PlatformType, 'WebGpu' | 'OpenGL' | 'Software'>
local front_end = { linux = 'OpenGL', windows = 'OpenGL', mac = 'OpenGL' }

-- 界面字体（标题栏、命令面板、字符选择、窗格选择）只在 Windows 设置：Segoe UI 半粗 →
-- 微软雅黑 UI → Segoe UI Emoji，各项仅在字体文件存在时加入；一项都没有就保持上游默认。
-- 终端正文字体在 config/fonts.lua，与这里无关。
local ui_font = nil
if platform.is_win then
   local dirs = font_files.windows_font_dirs(os.getenv, wezterm.executable_dir)
   local chain = font_files.ui_font_chain(dirs)
   if #chain > 0 then
      ui_font = wezterm.font_with_fallback(chain)
   end
end

local options = {
   -- 帧率上限：60 兜底，活动屏幕刷新率更高时跟随它。首次加载配置时 GUI 还没起来、读不到
   -- 屏幕，配置重载后才生效；Windows 上另由 config/fluent.lua 的 max_fps_follows_display
   -- 让每个窗口按自己所在显示器的刷新率限速，这里的值只是读不到刷新率时的兜底。
   max_fps = display.max_fps(60, function()
      return wezterm.gui.screens().active.max_fps
   end),
   front_end = front_end[platform.os],
   underline_thickness = '1.5pt',

   -- animation_fps 是动画（视觉铃声淡入淡出等）的帧率。光标与闪烁文字的缓动都用 Constant：
   -- 非 Constant 的缓动会让空闲窗口按 animation_fps 持续重绘，Constant 只在亮灭切换时
   -- 各画一帧，所以把动画帧率提到 60 不增加空闲重绘。
   animation_fps = 60,
   cursor_blink_ease_in = 'Constant',
   cursor_blink_ease_out = 'Constant',
   text_blink_ease_in = 'Constant',
   text_blink_ease_out = 'Constant',
   text_blink_rapid_ease_in = 'Constant',
   text_blink_rapid_ease_out = 'Constant',
   default_cursor_style = 'BlinkingBlock',
   cursor_blink_rate = 650,

   -- color scheme：整套调色板注册成 'GX Mocha' 并设为默认方案；设置浮层「外观」里
   -- 选的方案写在 gui-settings.json 的 color_scheme，会覆盖这里的默认值。界面色
   -- （标签栏、滚动条、分割线等）单独放 colors，叠加在任何方案之上。
   color_schemes = { [colors.name] = colors.scheme },
   color_scheme = colors.name,
   colors = colors.chrome,

   -- background: pass in `true` if you want wezterm to start with focus mode on (no bg images)
   background = backdrops:initial_options(false),

   -- scrollbar
   enable_scroll_bar = true,

   -- tab bar
   enable_tab_bar = true,
   hide_tab_bar_if_only_one_tab = false,
   use_fancy_tab_bar = false,
   tab_max_width = 32,
   show_tab_index_in_tab_bar = false,
   switch_to_last_active_tab_when_closing_tab = true,

   -- command palette
   command_palette_fg_color = '#b4befe',
   command_palette_bg_color = '#11111b',
   command_palette_font_size = 12,
   command_palette_rows = 25,

   -- window
   window_padding = {
      left = 10,
      right = 10,
      top = 8,
      bottom = 8,
   },
   adjust_window_size_when_changing_font_size = false,
   -- 关闭窗口先确认：点 X / Alt+F4 不再直接杀掉运行中的 herdr、agent 或编译。
   -- 只剩空闲 Shell 的窗口仍一键关闭，名单见 config/general.lua 的
   -- skip_close_confirmation_for_processes_named。
   window_close_confirmation = 'AlwaysPrompt',
   window_frame = {
      active_titlebar_bg = '#090909',
      font = ui_font,
      font_size = ui_font and 10 or nil,
   },
   -- inactive_pane_hsb = {
   --    saturation = 0.9,
   --    brightness = 0.65,
   -- },
   inactive_pane_hsb = {
      saturation = 0.85,
      brightness = 0.72,
   },

   visual_bell = {
      fade_in_function = 'EaseIn',
      fade_in_duration_ms = 250,
      fade_out_function = 'EaseOut',
      fade_out_duration_ms = 250,
      target = 'CursorColor',
   },
}

if ui_font then
   options.command_palette_font = ui_font
   options.char_select_font = ui_font
   options.pane_select_font = ui_font
end

return options
