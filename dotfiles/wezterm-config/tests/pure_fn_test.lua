-- 纯函数用例：herdr 应用模式判定 + tab 标题进程名清洗 + Config 字段守门。
--
-- 仓库没有 lua5.4/busted 等独立解释器（机器上只装了 liblua 库，没有 CLI），
-- 这里借用 wezterm 自带的 mlua 运行时当解释器：wezterm --config-file 会把
-- 该文件当配置执行，执行到 error() 时会静默回退默认配置、不产生任何可见
-- 诊断（已用探针验证），所以断言结果一律用 print 显式输出，不依赖异常/
-- 退出码。跑法：
--
--   wezterm --config-file tests/pure_fn_test.lua show-keys 2>&1 | grep PURE_FN_TEST
--
-- 全部通过时最后一行是 `PURE_FN_TEST: ALL PASS (n cases)`；任何一条失败会
-- 单独打印 `PURE_FN_TEST FAIL: <case> ...`。

local wezterm = require('wezterm')
-- wezterm.config_dir 是 --config-file 指向文件所在目录，即 tests/；回退一级
-- 拼出 wezterm-config 根目录，与调用时的进程 cwd 无关（wezterm 沙箱化的
-- Lua 环境没有 debug 库，不能用 debug.getinfo 自行定位脚本路径）。
local config_root = wezterm.config_dir .. '/..'
package.path = config_root .. '/?.lua;' .. config_root .. '/?/init.lua;' .. package.path

local status = require('events.status')
local tab_title = require('events.tab-title')

local failures = 0
local total = 0

---@param name string
---@param actual any
---@param expected any
local function check(name, actual, expected)
   total = total + 1
   if actual ~= expected then
      failures = failures + 1
      print(string.format('PURE_FN_TEST FAIL: %s (got %s, want %s)', name, tostring(actual), tostring(expected)))
   end
end

-- should_hide_tab_bar：herdr_app_mode / tab 数量 / 前台进程名 三个入参的判定表
check('hide.single_tab_herdr', status.should_hide_tab_bar(true, 1, 'herdr'), true)
check('hide.multi_tab_herdr', status.should_hide_tab_bar(true, 2, 'herdr'), false)
check('hide.single_tab_other_process', status.should_hide_tab_bar(true, 1, 'bash'), false)
check('hide.app_mode_disabled', status.should_hide_tab_bar(false, 1, 'herdr'), false)
check('hide.empty_process_name', status.should_hide_tab_bar(true, 1, ''), false)
check('hide.uncleaned_process_name', status.should_hide_tab_bar(true, 1, 'herdr.exe'), false)
check('hide.zero_tabs', status.should_hide_tab_bar(true, 0, 'herdr'), false)

-- clean_process_name（来自 events/tab-title.lua，herdr 应用模式判断复用同一口径）
check('clean.unix_path', tab_title.clean_process_name('/usr/bin/herdr'), 'herdr')
check('clean.windows_path_exe', tab_title.clean_process_name('C:\\Users\\x\\herdr.exe'), 'herdr')
check('clean.bare_name', tab_title.clean_process_name('herdr'), 'herdr')
check('clean.empty', tab_title.clean_process_name(''), '')

-- 防回归：wezterm.lua 里 append 进 Config 的每个模块，每个键都必须是合法的
-- wezterm Config 字段。纯表配置按 unknown_fields=Warn 转换，未知键不会报错，
-- 只会在下次 wezterm-gui 启动时弹 Configuration Error 窗口；这里借严格模式的
-- config_builder() 逐键赋值，未知键会被拒绝。模块清单直接从 wezterm.lua 的
-- `:append(require('...'))` 解析，新增模块自动纳入。
local function appended_config_modules()
   local file = io.open(config_root .. '/wezterm.lua', 'r')
   if not file then
      return {}
   end
   local source = file:read('a')
   file:close()
   local modules = {}
   for name in source:gmatch(":append%(%s*require%(%s*'([%w%._%-]+)'%s*%)%s*%)") do
      table.insert(modules, name)
   end
   return modules
end

local config_modules = appended_config_modules()
check('config_keys.modules_found', #config_modules > 0, true)
for _, module_name in ipairs(config_modules) do
   local loaded, options = pcall(require, module_name)
   check('config_keys.load.' .. module_name, loaded, true)
   if loaded and type(options) == 'table' then
      local builder = wezterm.config_builder()
      for key, value in pairs(options) do
         local accepted = pcall(function()
            builder[key] = value
         end)
         check('config_keys.' .. module_name .. '.' .. tostring(key), accepted, true)
      end
   end
end

if failures == 0 then
   print(string.format('PURE_FN_TEST: ALL PASS (%d cases)', total))
else
   print(string.format('PURE_FN_TEST: %d/%d FAILED', failures, total))
end

-- 保持这是一个能被 --config-file 加载的合法配置返回值。
return {}
