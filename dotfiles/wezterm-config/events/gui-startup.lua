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

---把窗口调成屏幕的 WINDOW_RATIO 并居中。拿不到 GUI 窗口（如不在活动工作区）或屏幕信息
---不可用时什么都不做，保持默认大小；返回是否已调整。
---@param window any WezTerm MuxWindow
---@param active_screen table|nil wezterm.gui.screens().active
---@return boolean
function M.fit_to_screen(window, active_screen)
   local geometry = M.centered_geometry(active_screen, WINDOW_RATIO)
   if not geometry then
      return false
   end
   local gui_window = window:gui_window()
   if not gui_window then
      return false
   end
   gui_window:set_inner_size(geometry.width, geometry.height)
   gui_window:set_position(geometry.x, geometry.y)
   return true
end

M.setup = function()
   wezterm.on('gui-startup', function(cmd)
      local _, _, window = mux.spawn_window(cmd or {})
      -- window:gui_window():maximize()

      -- 失败（如 Wayland 不允许应用摆放窗口、接口缺失）只记日志，回退默认行为
      local ok, err = pcall(function()
         M.fit_to_screen(window, wezterm.gui.screens().active)
      end)
      if not ok then
         wezterm.log_warn('gui-startup: 按屏幕调整首窗失败，保持默认大小: ', err)
      end
   end)
end

return M
