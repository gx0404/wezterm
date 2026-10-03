local wezterm = require('wezterm')
local colors = require('colors.custom')
local gui_settings = require('utils.gui-settings')

-- Seeding random numbers before generating for use
-- Known issue with lua math library
-- see: https://stackoverflow.com/questions/20154991/generating-uniform-random-numbers-in-lua
math.randomseed(os.time())
math.random()
math.random()
math.random()

local GLOB_PATTERN = '*.{jpg,jpeg,png,gif,bmp,ico,tiff,pnm,dds,tga}'

---读 gui-settings.json 原文；文件不存在或读不了返回 nil。
---@return string?
local function read_sidecar()
   local f = io.open(gui_settings.path(wezterm.config_dir, wezterm.home_dir, os.getenv), 'r')
   if not f then
      return nil
   end
   local text = f:read('*a')
   f:close()
   return text
end

---内置配色方案的背景色；取不到返回 nil。get_builtin_schemes 每次要转换上千套方案，
---整份结果在本次配置加载内缓存。
local builtin_schemes = nil
---@param name string
---@return string?
local function builtin_background(name)
   if builtin_schemes == nil then
      builtin_schemes = wezterm.color.get_builtin_schemes()
   end
   local scheme = builtin_schemes[name]
   return scheme and scheme.background
end

---@class BackDrops
---@field current_idx number index of current image
---@field images string[] background images
---@field images_dir string directory of background images. Default is `wezterm.config_dir .. '/backdrops/'`
---@field mask_color string color of the mask layer above the image. Default is the background of the effective color scheme
---@field focus_color string background color when in focus mode. Default is the background of the effective color scheme
---@field focus_on boolean focus mode on or off
local BackDrops = {}
BackDrops.__index = BackDrops

---遮罩层/专注模式要跟随当前生效配色方案的背景：设置浮层「外观」选的方案写在
---gui-settings.json 的 color_scheme 键（轻量提取，同壁纸键）。没有该键或选的就是
---GX Mocha 时用 colors.scheme.background，否则查内置方案的背景；查不到（自定义/已
---删除的方案、接口出错）一律回退 Mocha 背景。纯函数：文件内容与查表函数由调用方注入，
---便于 tests/pure_fn_test.lua 单测。
---@param settings_text string? gui-settings.json 原文，读不到传 nil
---@param lookup fun(name: string): string? 内置方案名 -> 背景色，取不到返回 nil
---@return string
function BackDrops.scheme_background(settings_text, lookup)
   local name = settings_text and settings_text:match('"color_scheme"%s*:%s*"([^"]+)"')
   if name and name ~= colors.name then
      local ok, background = pcall(lookup, name)
      if ok and type(background) == 'string' and background ~= '' then
         return background
      end
   end
   return colors.scheme.background
end

--- Initialise backdrop controller
---@private
function BackDrops:init()
   local mask_color = BackDrops.scheme_background(read_sidecar(), builtin_background)
   local inital = {
      current_idx = 1,
      images = {},
      images_dir = wezterm.config_dir .. '/backdrops/',
      mask_color = mask_color,
      focus_color = mask_color,
      focus_on = false,
   }
   local backdrops = setmetatable(inital, self)
   return backdrops
end

---Override the default `images_dir`
---Default `images_dir` is `wezterm.config_dir .. '/backdrops/'`
---
--- INFO:
---  This function must be invoked before `set_images()`
---
---@param path string directory of background images
function BackDrops:set_images_dir(path)
   self.images_dir = path
   if not path:match('/$') then
      self.images_dir = path .. '/'
   end
   return self
end

---MUST BE RUN BEFORE ALL OTHER `BackDrops` functions
---Sets the `images` after instantiating `BackDrops`.
---
--- INFO:
---   During the initial load of the config, this function can only invoked in `wezterm.lua`.
---   WezTerm's fs utility `glob` (used in this function) works by running on a spawned child process.
---   This throws a coroutine error if the function is invoked in outside of `wezterm.lua` in the -
---   initial load of the Terminal config.
function BackDrops:set_images()
   local ok, images = pcall(wezterm.glob, self.images_dir .. GLOB_PATTERN)
   -- fork（WEZ-CFG-03）：glob 失败/目录不存在不拖垮配置加载；空表时
   -- `_create_opts` 退化为纯色遮罩层。
   self.images = (ok and images) or {}
   return self
end

---按文件名选择稳定的默认背景；找不到时回退到第一张。
---@param filename string
function BackDrops:set_default(filename)
   for idx, path in ipairs(self.images) do
      if path:match('([^/]+)$') == filename then
         self.current_idx = idx
         return self
      end
   end

   self.current_idx = 1
   wezterm.log_warn('Backdrop not found, using first image: ', filename)
   return self
end

---fork（批 13）：壁纸管理浮层把持久化选择写在 gui-settings.json 的
---wallpaper 键；启动/重载时优先按它覆盖默认（只认 basename 且必须在
---目录内——set_default 的查找天然挡掉目录外与缺失条目）。
function BackDrops:set_default_from_sidecar()
   local text = read_sidecar()
   if not text then
      return self
   end
   -- gui-settings.json 由 fork 的 store_key 原子写入（顶层键形状固定）；
   -- 轻量提取，不为一个键引入 JSON 解析器。
   local name = text:match('"wallpaper"%s*:%s*"([^"]+)"')
   if name and name:match('^[^/\\]+$') then
      self:set_default(name)
   end
   return self
end

---Override the default `focus_color`
---Default `focus_color` is the background of the effective color scheme
---@param focus_color string background color when in focus mode
function BackDrops:set_focus(focus_color)
   self.focus_color = focus_color
   return self
end

---Create the `background` options with the current image
---@private
---@return table
function BackDrops:_create_opts()
   -- fork（WEZ-CFG-03）：壁纸目录为空时回退纯色遮罩，不产生 File=nil 层
   if #self.images == 0 then
      return self:_create_focus_opts()
   end
   return {
      {
         source = { File = self.images[self.current_idx] },
         horizontal_align = 'Center',
      },
      {
         source = { Color = self.mask_color },
         height = '120%',
         width = '120%',
         vertical_offset = '-10%',
         horizontal_offset = '-10%',
         opacity = 0.92,
      },
   }
end

---Create the `background` options for focus mode
---@private
---@return table
function BackDrops:_create_focus_opts()
   return {
      {
         source = { Color = self.focus_color },
         height = '120%',
         width = '120%',
         vertical_offset = '-10%',
         horizontal_offset = '-10%',
         opacity = 1,
      },
   }
end

---Set the initial options for `background`
---@param focus_on boolean? focus mode on or off
function BackDrops:initial_options(focus_on)
   focus_on = focus_on or false
   assert(type(focus_on) == 'boolean', 'BackDrops:initial_options - Expected a boolean')

   self.focus_on = focus_on
   if focus_on then
      return self:_create_focus_opts()
   end

   return self:_create_opts()
end

---Override the current window options for background
---@private
---@param window any WezTerm Window see: https://wezfurlong.org/wezterm/config/lua/window/index.html
---@param background_opts table background option
function BackDrops:_set_opt(window, background_opts)
   window:set_config_overrides({
      background = background_opts,
      enable_tab_bar = window:effective_config().enable_tab_bar,
   })
end

---Override the current window options for background with focus color
---@private
---@param window any WezTerm Window see: https://wezfurlong.org/wezterm/config/lua/window/index.html
function BackDrops:_set_focus_opt(window)
   local opts = {
      background = {
         {
            source = { Color = self.focus_color },
            height = '120%',
            width = '120%',
            vertical_offset = '-10%',
            horizontal_offset = '-10%',
            opacity = 1,
         },
      },
      enable_tab_bar = window:effective_config().enable_tab_bar,
   }
   window:set_config_overrides(opts)
end

---Convert the `files` array to a table of `InputSelector` choices
---see: https://wezfurlong.org/wezterm/config/lua/keyassignment/InputSelector.html
function BackDrops:choices()
   local choices = {}
   for idx, file in ipairs(self.images) do
      table.insert(choices, {
         id = tostring(idx),
         label = file:match('([^/]+)$'),
      })
   end
   return choices
end

---Select a random background from the loaded `files`
---Pass in `Window` object to override the current window options
---@param window any? WezTerm `Window` see: https://wezfurlong.org/wezterm/config/lua/window/index.html
function BackDrops:random(window)
   -- fork（WEZ-CFG-03）：空目录时 math.random(0) 会抛错，提前返回
   if #self.images == 0 then
      return
   end
   self.current_idx = math.random(#self.images)

   if window ~= nil then
      self:_set_opt(window, self:_create_opts())
   end
end

---Cycle the loaded `files` and select the next background
---@param window any WezTerm `Window` see: https://wezfurlong.org/wezterm/config/lua/window/index.html
function BackDrops:cycle_forward(window)
   if #self.images == 0 then
      return
   end
   if self.current_idx == #self.images then
      self.current_idx = 1
   else
      self.current_idx = self.current_idx + 1
   end
   self:_set_opt(window, self:_create_opts())
end

---Cycle the loaded `files` and select the previous background
---@param window any WezTerm `Window` see: https://wezfurlong.org/wezterm/config/lua/window/index.html
function BackDrops:cycle_back(window)
   if #self.images == 0 then
      return
   end
   if self.current_idx == 1 then
      self.current_idx = #self.images
   else
      self.current_idx = self.current_idx - 1
   end
   self:_set_opt(window, self:_create_opts())
end

---Set a specific background from the `files` array
---@param window any WezTerm `Window` see: https://wezfurlong.org/wezterm/config/lua/window/index.html
---@param idx number index of the `files` array
function BackDrops:set_img(window, idx)
   if idx > #self.images or idx < 1 then
      wezterm.log_error('Index out of range')
      return
   end

   self.current_idx = idx
   self:_set_opt(window, self:_create_opts())
end

---Toggle the focus mode
---@param window any WezTerm `Window` see: https://wezfurlong.org/wezterm/config/lua/window/index.html
function BackDrops:toggle_focus(window)
   local background_opts

   if self.focus_on then
      background_opts = self:_create_opts()
      self.focus_on = false
   else
      background_opts = self:_create_focus_opts()
      self.focus_on = true
   end

   self:_set_opt(window, background_opts)
end

return BackDrops:init()
