local wezterm = require('wezterm')

-- fork（WEZ-CFG-03）：插件 require 失败（离线机器/插件目录损坏）不该
-- 拖垮整份配置加载；失败时返回 nil，bindings.lua 的插件键位跳过注册。
local function try_require(url)
   local ok, plugin = pcall(wezterm.plugin.require, url)
   if ok then
      return plugin
   end
   wezterm.log_error('plugin load failed (skipping its keybindings): ' .. url)
   return nil
end

-- resurrect: 会话保存与恢复
local resurrect = try_require('https://github.com/MLFlexer/resurrect.wezterm')

-- smart_workspace_switcher: 智能 workspace 切换
local workspace_switcher = try_require('https://github.com/MLFlexer/smart_workspace_switcher.wezterm')
if workspace_switcher then
   workspace_switcher.apply_to_config = workspace_switcher.apply_to_config or function() end
end

return {
   resurrect = resurrect,
   workspace_switcher = workspace_switcher,
}
