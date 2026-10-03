local wezterm = require('wezterm')
local platform = require('utils.platform')

-- 只有本分支（gx0404/wezterm）新构建才认得的配置键。
--
-- 旧的 wezterm 二进制遇到未知配置键会在启动时弹 Configuration Error 窗口，所以这些键
-- 不放进 config/*.lua 的静态表，而是由 wezterm.lua 在最后调用 apply()：逐键用严格模式的
-- config_builder 探测当前二进制认不认得，认得才写入，不认得就跳过并记一条日志。
-- 这样配置和二进制版本不一致（先升级了配置、二进制还是旧的）时仍能正常启动。
--
-- 以后新增 fork 配置键：先在 Rust 侧加字段，再把键登记到 declared() 里；不要直接写进
-- config/appearance.lua 等静态表。

local M = {}

---本分支新增、旧二进制不认得的配置键。top 是顶层键，colors 是 `colors` 表里的键
---（界面色，与 colors/custom.lua 的 chrome 叠加）。
---@param is_win boolean
---@return { top: table, colors: table }
local function declared(is_win)
   local top = {}
   local colors = {}

   -- 帧率上限跟随窗口所在显示器的刷新率（config.rs::max_fps_follows_display，目前只有
   -- Windows 实现，其他平台忽略，所以只在 Windows 写入）。max_fps 仍是读不到刷新率时的兜底。
   if is_win then
      top.max_fps_follows_display = true
   end

   return { top = top, colors = colors }
end

---用严格模式的 config_builder 探测当前二进制认不认得某个配置键（含值的类型）。
---构建器赋值时即时走一遍 Config::from_dynamic（未知键、类型不符都会抛错），副作用只落在
---这个用完即弃的构建器上。
---@param key string
---@param value any
---@return boolean
function M.supported(key, value)
   local ok = pcall(function()
      local builder = wezterm.config_builder()
      builder[key] = value
   end)
   return ok
end

---浅拷贝后叠加 extra 的键。
---@param base table?
---@param extra table
---@return table
local function merged(base, extra)
   local out = {}
   for key, value in pairs(base or {}) do
      out[key] = value
   end
   for key, value in pairs(extra) do
      out[key] = value
   end
   return out
end

---把当前二进制认得的 fork 新增键写进完整配置表（就地修改并返回）。
---顶层键已被其他模块设置时不覆盖（与 Config:append 对重复键的处理一致）。
---@param options table 已合并好的完整配置表
---@param opts? { supports?: fun(key: string, value: any): boolean, is_win?: boolean } 测试用注入点，缺省探测真实二进制 / 当前平台
---@return table options
function M.apply(options, opts)
   opts = opts or {}
   local supports = opts.supports or M.supported
   local is_win = opts.is_win
   if is_win == nil then
      is_win = platform.is_win
   end
   local entries = declared(is_win)
   local skipped = {}

   for key, value in pairs(entries.top) do
      if options[key] ~= nil then
         wezterm.log_warn('config.fluent: 配置键已被设置，不覆盖: ', key)
      elseif supports(key, value) then
         options[key] = value
      else
         table.insert(skipped, key)
      end
   end

   local accepted = {}
   for key, value in pairs(entries.colors) do
      if supports('colors', { [key] = value }) then
         accepted[key] = value
      else
         table.insert(skipped, 'colors.' .. key)
      end
   end
   if next(accepted) ~= nil then
      options.colors = merged(options.colors, accepted)
   end

   if #skipped > 0 then
      table.sort(skipped)
      wezterm.log_warn(
         'config.fluent: 当前 wezterm 不认得这些配置键，已跳过（需要本分支的新构建）: ',
         table.concat(skipped, ', ')
      )
   end
   return options
end

-- 导出：供 tests/pure_fn_test.lua 校验登记的键名与值类型（用 declared 的原样内容）。
M.declared = declared

return M
