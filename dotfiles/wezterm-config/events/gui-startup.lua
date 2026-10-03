local wezterm = require('wezterm')
local mux = wezterm.mux

local M = {}

-- 首窗占活动屏幕的比例并居中：默认的 80x24 在高分屏上小得出奇，在小屏上又可能撑满，
-- 按屏幕自适应比写死行列数稳妥。屏幕太小时不低于下面的最小尺寸（但不超过屏幕）。
local WINDOW_RATIO = 0.8
local MIN_WIDTH = 640
local MIN_HEIGHT = 400

---@param value any
---@return boolean
local function positive_number(value)
   return type(value) == 'number' and value > 0
end

---@param size number 屏幕的宽或高
---@param ratio number
---@param minimum number
---@return integer
local function scaled(size, ratio, minimum)
   return math.floor(math.max(size * ratio, math.min(size, minimum)))
end

---按屏幕算出居中的窗口几何（像素）。纯函数；屏幕信息不可用或不合理时返回 nil。
---@param screen {x: number?, y: number?, width: number?, height: number?}|nil wezterm.gui.screens().active
---@param ratio number 占屏幕宽高的比例
---@return {width: integer, height: integer, x: integer, y: integer}|nil
function M.centered_geometry(screen, ratio)
   if type(screen) ~= 'table' then
      return nil
   end
   if not (positive_number(screen.width) and positive_number(screen.height)) then
      return nil
   end
   local width = scaled(screen.width, ratio, MIN_WIDTH)
   local height = scaled(screen.height, ratio, MIN_HEIGHT)
   return {
      width = width,
      height = height,
      x = math.floor((screen.x or 0) + (screen.width - width) / 2),
      y = math.floor((screen.y or 0) + (screen.height - height) / 2),
   }
end

---估算一个终端格子的像素尺寸：pt → px 按屏幕有效 DPI 换算（缺省 96），等宽字体
---行高约 1.3em、字宽约 0.6em（JetBrainsMono 实测口径，偏差只影响首窗大小的百分之几）。
---@param font_size_pt number
---@param effective_dpi number|nil
---@return number cell_width_px, number cell_height_px
function M.estimate_cell_size(font_size_pt, effective_dpi)
   local dpi = positive_number(effective_dpi) and effective_dpi or 96
   local font_px = font_size_pt * dpi / 72
   return font_px * 0.6, font_px * 1.3
end

---把居中几何换成 `mux.spawn_window` 能直接消费的参数：行列数 + 相对活动屏幕的位置。
---不能在 gui-startup 里拿 `window:gui_window()`——GUI 窗口此时尚未创建，调用会阻塞主线程、
---窗口永远不出现（原配置把 `gui_window():maximize()` 注释掉也是同一原因）。纯函数。
---@param screen table|nil wezterm.gui.screens().active
---@param ratio number
---@param font_size_pt number
---@return {width: integer, height: integer, position: table}|nil
function M.spawn_geometry(screen, ratio, font_size_pt)
   local geometry = M.centered_geometry(screen, ratio)
   if not geometry or not positive_number(font_size_pt) then
      return nil
   end
   local cell_w, cell_h = M.estimate_cell_size(font_size_pt, screen.effective_dpi)
   return {
      width = math.max(math.floor(geometry.width / cell_w), 20),
      height = math.max(math.floor(geometry.height / cell_h), 5),
      position = {
         -- origin=ActiveScreen：坐标相对活动屏幕左上角，不再叠加 screen.x/y
         x = math.floor((screen.width - geometry.width) / 2),
         y = math.floor((screen.height - geometry.height) / 2),
         origin = 'ActiveScreen',
      },
   }
end

M.setup = function()
   wezterm.on('gui-startup', function(cmd)
      local spawn = cmd or {}
      -- 失败（如屏幕信息不可用、接口缺失）只记日志，回退默认行为
      local ok, err = pcall(function()
         local screens = wezterm.gui.screens()
         local font_size = require('config.fonts').font_size
         local geometry = M.spawn_geometry(screens and screens.active, WINDOW_RATIO, font_size)
         if geometry then
            spawn.width = spawn.width or geometry.width
            spawn.height = spawn.height or geometry.height
            spawn.position = spawn.position or geometry.position
         end
      end)
      if not ok then
         wezterm.log_warn('gui-startup: 按屏幕计算首窗几何失败，保持默认大小: ', err)
      end
      mux.spawn_window(spawn)
   end)
end

return M
