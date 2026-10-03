local wezterm = require('wezterm')
local colors = require('colors.custom')
local gui_settings = require('utils.gui-settings')
local platform = require('utils.platform')

-- Seeding random numbers before generating for use
-- Known issue with lua math library
-- see: https://stackoverflow.com/questions/20154991/generating-uniform-random-numbers-in-lua
math.randomseed(os.time())
math.random()
math.random()
math.random()

local GLOB_PATTERN = '*.{jpg,jpeg,png,gif,bmp,ico,tiff,pnm,dds,tga}'

-- 设置浮层「外观」里的窗口材质（gui-settings.json 的 window_material 键）：
--   wallpaper  壁纸 + 遮罩（默认）
--   solid      纯色背景
--   mica       云母：只放一层半透明的方案底色，透出系统 Mica 材质（Windows 11）
--   acrylic    亚克力：同上，透出系统 Acrylic 模糊（Windows）
local MATERIALS = { wallpaper = true, solid = true, mica = true, acrylic = true }
local DEFAULT_MATERIAL = 'wallpaper'

-- 系统材质要窗口本身半透明才看得见：云母底色层不透明度 0.3，亚克力 0.75（亚克力自带模糊，
-- 可以更实一些）。
local MATERIAL_OPACITY = { mica = 0.3, acrylic = 0.75 }
local MATERIAL_BACKDROP = { mica = 'Mica', acrylic = 'Acrylic' }

---整窗的纯色层。尺寸偏移留着 120% 的余量：壁纸遮罩层沿用同一写法，保证盖住整窗。
---@param color string
---@param opacity number
---@return table
local function color_layer(color, opacity)
   return {
      source = { Color = color },
      height = '120%',
      width = '120%',
      vertical_offset = '-10%',
      horizontal_offset = '-10%',
      opacity = opacity,
   }
end

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
---@field material 'wallpaper'|'solid'|'mica'|'acrylic' window material, from `window_material` in gui-settings.json
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

---窗口材质取自 gui-settings.json 的 window_material 键（轻量提取，同壁纸键）；没有该键、
---值不在 wallpaper|solid|mica|acrylic 内都按 wallpaper。纯函数，便于单测。
---@param settings_text string? gui-settings.json 原文，读不到传 nil
---@return 'wallpaper'|'solid'|'mica'|'acrylic'
function BackDrops.material_from_settings(settings_text)
   local name = settings_text and settings_text:match('"window_material"%s*:%s*"([^"]*)"')
   if name and MATERIALS[name] then
      return name
   end
   return DEFAULT_MATERIAL
end

---云母/亚克力只在 Windows 上有系统材质：其他平台把它们降级成壁纸，免得得到一个没有
---模糊底的半透明窗口。Windows 上再看系统支不支持（Windows 10 没有云母，1803 之前也没有
---亚克力）：support 是 wezterm.gui.system_backdrop_support() 的结果，选的材质在其中不为
---true 就回退纯色（同样是为了不留一个没有模糊底的半透明窗口）；support 为 nil（旧二进制
---没有该接口、或不在 GUI 里）时不判断，保持原样。纯函数。
---@param material string
---@param is_win boolean
---@param support table? { mica = boolean, acrylic = boolean, tabbed = boolean }
---@return 'wallpaper'|'solid'|'mica'|'acrylic'
function BackDrops.effective_material(material, is_win, support)
   if not is_win and MATERIAL_OPACITY[material] then
      return DEFAULT_MATERIAL
   end
   if type(support) == 'table' and MATERIAL_OPACITY[material] and support[material] ~= true then
      return 'solid'
   end
   if MATERIALS[material] then
      return material
   end
   return DEFAULT_MATERIAL
end

---系统支持哪些背景材质：wezterm.gui.system_backdrop_support()。wezterm.gui 只在 GUI 进程里有，
---旧二进制没有这个函数；取不到（含调用出错、返回值不是表）一律返回 nil，effective_material
---据此保持原样。该接口只查系统版本、不取配置锁，配置求值期调用是安全的。
---@return table?
function BackDrops.system_backdrop_support()
   local ok, support = pcall(function()
      return wezterm.gui.system_backdrop_support()
   end)
   if ok and type(support) == 'table' then
      return support
   end
   return nil
end

---按窗口材质生成 `background` 层栈，以及材质要求一起设置的窗口配置键。纯函数。
---  wallpaper  壁纸层 + 方案底色遮罩（0.92）；没有壁纸文件时退化成一层纯色
---  solid      一层不透明的方案底色
---  mica       一层 0.3 不透明度的方案底色；附加 win32_system_backdrop = 'Mica'、
---             window_background_opacity = 0.3
---  acrylic    同上，不透明度 0.75；win32_system_backdrop = 'Acrylic'
---附加键必须和层栈一起生效（系统材质要窗口半透明才露得出来），由 config/appearance.lua
---在 Windows 上合并进配置。
---@param material string wallpaper|solid|mica|acrylic，其他值按 wallpaper
---@param color string 方案底色（遮罩层/纯色层的颜色）
---@param image_path string? 当前壁纸文件；nil 表示没有壁纸
---@return table layers
---@return table extra 需要一起设置的窗口配置键，没有时为空表
function BackDrops.material_layers(material, color, image_path)
   local opacity = MATERIAL_OPACITY[material]
   if opacity then
      return { color_layer(color, opacity) }, {
         win32_system_backdrop = MATERIAL_BACKDROP[material],
         window_background_opacity = opacity,
      }
   end
   if material == 'solid' or not image_path then
      return { color_layer(color, 1) }, {}
   end
   return {
      {
         source = { File = image_path },
         horizontal_align = 'Center',
      },
      color_layer(color, 0.92),
   }, {}
end

--- Initialise backdrop controller
---@private
function BackDrops:init()
   local settings_text = read_sidecar()
   local mask_color = BackDrops.scheme_background(settings_text, builtin_background)
   local inital = {
      current_idx = 1,
      images = {},
      images_dir = wezterm.config_dir .. '/backdrops/',
      mask_color = mask_color,
      focus_color = mask_color,
      focus_on = false,
      material = BackDrops.effective_material(
         BackDrops.material_from_settings(settings_text),
         platform.is_win,
         BackDrops.system_backdrop_support()
      ),
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
   if #self.images == 0 and self.material == 'wallpaper' then
      return self:_create_focus_opts()
   end
   local layers =
      BackDrops.material_layers(self.material, self.mask_color, self.images[self.current_idx])
   return layers
end

---当前窗口材质要求一起设置的窗口配置键（云母/亚克力：win32_system_backdrop 与
---window_background_opacity；壁纸/纯色为空表）。config/appearance.lua 在 Windows 上
---把它们合并进配置；非 Windows 上云母/亚克力在 init 时已降级成壁纸、系统不支持的材质
---已回退纯色，这里为空。
---@return table
function BackDrops:window_options()
   local _, extra = BackDrops.material_layers(self.material, self.mask_color, nil)
   return extra
end

---Create the `background` options for focus mode
---@private
---@return table
function BackDrops:_create_focus_opts()
   return { color_layer(self.focus_color, 1) }
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
      background = self:_create_focus_opts(),
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
