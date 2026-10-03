-- 带时效的缓存判断。纯函数：不 require('wezterm')，时钟由调用方注入（os.time()），
-- 可脱离 wezterm 运行时单测。

local M = {}

---缓存是否仍在有效期内；时钟被往回拨（age 为负）按过期处理。
---@param at integer 缓存写入时刻（os.time()）
---@param now integer 当前时刻（os.time()）
---@param ttl integer 有效期（秒）
---@return boolean
function M.still_fresh(at, now, ttl)
   local age = now - at
   return age >= 0 and age < ttl
end

return M
