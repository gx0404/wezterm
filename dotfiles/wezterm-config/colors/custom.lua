-- A slightly altered version of catppucchin mocha
-- stylua: ignore
local mocha = {
   rosewater = '#f5e0dc',
   flamingo  = '#f2cdcd',
   pink      = '#f5c2e7',
   mauve     = '#cba6f7',
   red       = '#f38ba8',
   maroon    = '#eba0ac',
   peach     = '#fab387',
   yellow    = '#f9e2af',
   green     = '#a6e3a1',
   teal      = '#94e2d5',
   sky       = '#89dceb',
   sapphire  = '#74c7ec',
   blue      = '#89b4fa',
   lavender  = '#b4befe',
   text      = '#FFFFFF',
   subtext1  = '#bac2de',
   subtext0  = '#a6adc8',
   overlay2  = '#9399b2',
   overlay1  = '#7f849c',
   overlay0  = '#6c7086',
   surface2  = '#585b70',
   surface1  = '#45475a',
   surface0  = '#313244',
   base      = '#1f1f28',
   mantle    = '#181825',
   crust     = '#11111b',
}

-- 拆成两份，让设置浮层「外观」里的配色切换真正生效：
--   scheme：整套终端调色板，注册为名为 'GX Mocha' 的配色方案（config/appearance.lua 的
--           color_schemes）并设为默认 color_scheme；设置页写进 gui-settings.json 的
--           color_scheme 会覆盖它，前景/背景/ANSI 16 色随之整套换掉。
--   chrome：标签栏、滚动条、分割线等界面色，作为 `colors` 叠加在任何方案之上，
--           切换方案时保持 Mocha 风格的界面外观。
-- ANSI 16 色用 Catppuccin Mocha 官方色板（此前是 Windows Terminal Campbell，
-- 蓝/紫在 #1f1f28 背景上对比度只有约 2:1，与整体配色割裂）；背景保持 mocha.base。
local scheme = {
   foreground = mocha.text,
   background = mocha.base,
   cursor_bg = mocha.rosewater,
   cursor_border = mocha.rosewater,
   cursor_fg = mocha.crust,
   selection_bg = mocha.surface2,
   selection_fg = mocha.text,
   ansi = {
      mocha.surface1, -- black
      mocha.red,
      mocha.green,
      mocha.yellow,
      mocha.blue,
      mocha.pink, -- magenta
      mocha.teal, -- cyan
      mocha.subtext1, -- white
   },
   brights = {
      mocha.surface2, -- black
      mocha.red,
      mocha.green,
      mocha.yellow,
      mocha.blue,
      mocha.pink, -- magenta
      mocha.teal, -- cyan
      mocha.subtext0, -- white
   },
   indexed = {
      [16] = mocha.peach,
      [17] = mocha.rosewater,
   },
}

local chrome = {
   -- fancy 标签栏（config/appearance.lua 的 use_fancy_tab_bar）：栏底色来自 window_frame 的
   -- 标题栏底色（crust），这里的 background 只在经典标签栏下生效，取同色保持一致。
   -- 活动标签与终端背景同色（base），和内容区连成一片；非活动标签融进栏底（crust），悬停
   -- 时浮起一层 surface0；+ 按钮与栏底同色，悬停同样浮起。标签标题与图标的文字色
   -- （events/tab-title.lua）也读这里的三态 fg_color。
   tab_bar = {
      background = mocha.crust,
      active_tab = {
         bg_color = mocha.base,
         fg_color = mocha.text,
      },
      inactive_tab = {
         bg_color = mocha.crust,
         fg_color = mocha.subtext0,
      },
      inactive_tab_hover = {
         bg_color = mocha.surface0,
         fg_color = mocha.text,
      },
      inactive_tab_edge = mocha.surface0,
      new_tab = {
         bg_color = mocha.crust,
         fg_color = mocha.subtext0,
      },
      new_tab_hover = {
         bg_color = mocha.surface0,
         fg_color = mocha.text,
      },
   },
   visual_bell = mocha.red,
   scrollbar_thumb = mocha.surface2,
   split = mocha.overlay0,
   compose_cursor = mocha.flamingo,
}

return {
   -- scheme 注册到 color_schemes 时用的名字；config/appearance.lua 与
   -- utils/backdrops.lua（判断设置浮层是否选了别的方案）共用这一处。
   name = 'GX Mocha',
   scheme = scheme,
   chrome = chrome,
   mocha = mocha,
}
