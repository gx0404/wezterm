-- Windows 字体文件探测：只在字体文件真的存在时才把它加进回退链，没装字体的机器
-- 不会多出「字体未找到」告警。纯函数：文件系统与环境变量由调用方注入，不 require('wezterm')，
-- 可脱离 wezterm 运行时单测（见 tests/pure_fn_test.lua）。

local gx_shell = require('utils.gx-shell')

local M = {}

---能否打开读取（与 utils/shells.lua 同法，不起子进程）。
---@param path string
---@return boolean
function M.readable(path)
   local file = io.open(path, 'rb')
   if file then
      file:close()
      return true
   end
   return false
end

---Windows 上可能放字体文件的目录（都带结尾反斜杠）：系统字体目录、用户字体目录
---（Windows 10 1809+ 的「为当前用户安装」）、GX Shell 安装包自带的 `<root>\fonts`
---（安装器把精选字重装在那里并注册给 DirectWrite，不在前两个目录里）。
---@param getenv fun(name: string): string|nil os.getenv；WezTerm 的版本遇到非 UTF-8 值会抛错，按未设置处理
---@param executable_dir string|nil wezterm.executable_dir
---@return string[]
function M.windows_font_dirs(getenv, executable_dir)
   local function env(name)
      local ok, value = pcall(getenv, name)
      if ok and type(value) == 'string' and value ~= '' then
         return value
      end
      return nil
   end

   local dirs = { (env('SystemRoot') or 'C:\\Windows') .. '\\Fonts\\' }
   local local_app_data = env('LOCALAPPDATA')
   if local_app_data then
      table.insert(dirs, local_app_data .. '\\Microsoft\\Windows\\Fonts\\')
   end
   local root = gx_shell.install_root(executable_dir)
   if root then
      table.insert(dirs, root .. '\\fonts\\')
   end
   return dirs
end

---任一目录里存在任一文件名即为真。
---@param dirs string[]
---@param names string[]
---@param exists? fun(path: string): boolean 缺省 M.readable
---@return boolean
function M.any_exists(dirs, names, exists)
   exists = exists or M.readable
   for _, dir in ipairs(dirs) do
      for _, name in ipairs(names) do
         if exists(dir .. name) then
            return true
         end
      end
   end
   return false
end

-- Noto Sans CJK：GX Shell 安装包随带 NotoSansCJK-Regular.ttc；单文件 OTF 版本也算。
M.NOTO_CJK_FILES = { 'NotoSansCJK-Regular.ttc', 'NotoSansCJKsc-Regular.otf' }

---界面字体回退链（命令面板、字符选择、窗格选择、标题栏/标签栏）：Segoe UI → 微软雅黑 UI
---→ Segoe UI Emoji，每项只在字体文件存在时加入；全都没有时返回空表（调用方不设置）。
---weight 缺省 'DemiBold'（浮层用半粗，没有半粗文件时退回 Regular）；标题栏/标签栏传
---'Regular'，非粗体更接近 Windows 11 的标题栏观感。
---@param dirs string[]
---@param exists? fun(path: string): boolean
---@param weight? 'Regular'|'DemiBold' Segoe UI 的字重
---@return (string|table)[]
function M.ui_font_chain(dirs, exists, weight)
   local chain = {}
   if weight ~= 'Regular' and M.any_exists(dirs, { 'segoeuisb.ttf' }, exists) then
      table.insert(chain, { family = 'Segoe UI', weight = 'DemiBold' })
   elseif M.any_exists(dirs, { 'segoeui.ttf' }, exists) then
      table.insert(chain, { family = 'Segoe UI', weight = 'Regular' })
   end
   if M.any_exists(dirs, { 'msyh.ttc', 'msyhl.ttc' }, exists) then
      table.insert(chain, 'Microsoft YaHei UI')
   end
   if M.any_exists(dirs, { 'seguiemj.ttf' }, exists) then
      table.insert(chain, 'Segoe UI Emoji')
   end
   return chain
end

return M
