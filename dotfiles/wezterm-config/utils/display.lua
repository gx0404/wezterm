-- 显示器相关的纯函数。不 require('wezterm')：屏幕信息由调用方注入，
-- 可脱离 wezterm 运行时单测（见 tests/pure_fn_test.lua）。

local M = {}

-- config/src/config.rs::validate_max_fps 的合法上限，超出会让整份配置报错。
local MAX_FPS_LIMIT = 1000

---帧率上限：以 base 兜底，活动屏幕的刷新率不低于 base 时跟随它。读不到（首次加载配置时
---GUI 还没起来，wezterm.gui 为 nil 或 screens() 抛错）或值不合理时返回 base，配置
---重载后才拿到屏幕信息。
---@param base integer 兜底帧率
---@param read_screen_fps fun(): any 返回活动屏幕的 max_fps；可以抛错或返回 nil
---@return integer
function M.max_fps(base, read_screen_fps)
   local ok, fps = pcall(read_screen_fps)
   if ok and type(fps) == 'number' and fps >= base then
      return math.floor(math.min(fps, MAX_FPS_LIMIT))
   end
   return base
end

return M
