-- Run with lua5.4 from wezterm/; the wezterm module, environment and filesystem are stubbed.
package.path = 'dotfiles/wezterm-config/?.lua;dotfiles/wezterm-config/?/init.lua;' .. package.path

local shells = require('utils.shells')
local real_exists = shells.exists

local cases = 0
local function check(name, actual, expected)
   cases = cases + 1
   if actual ~= expected then
      error(string.format('%s: got %s, want %s', name, tostring(actual), tostring(expected)), 2)
   end
end

local function ids(list)
   local out = {}
   for _, entry in ipairs(list) do
      table.insert(out, entry.id)
   end
   return table.concat(out, ',')
end

local function by_id(list, id)
   for _, entry in ipairs(list) do
      if entry.id == id then
         return entry
      end
   end
   return nil
end

local function fake_fs(paths)
   local set = {}
   for _, path in ipairs(paths) do
      set[path] = true
   end
   return function(path)
      return set[path] == true
   end
end

local function fake_env(values)
   return function(name)
      return values[name]
   end
end

-- exists(): only Store app-execution aliases under WindowsApps count without a successful open
-- (they fail with EINVAL but still start); elsewhere a failed open means "not there".
do
   local real_open = io.open
   local store = 'C:\\Users\\x\\AppData\\Local\\Microsoft\\WindowsApps\\'
   local results = {
      [store .. 'pwsh.exe'] = 22,
      [store .. 'missing.exe'] = 2,
      ['C:/Users/x/AppData/Local/Microsoft/WindowsApps/wsl.exe'] = 22,
      ['C:\\apps\\pwsh.exe'] = 22,
      ['E:\\bin\\pwsh.exe'] = 22,
      ['C:\\apps\\missing.exe'] = 2,
      ['/usr/bin/zsh/zsh'] = 20,
      ['C:\\apps\\locked.exe'] = 13,
   }
   io.open = function(path, mode)
      if path == 'C:\\apps\\real.exe' then
         return { close = function() end }
      end
      local code = results[path]
      if code then
         return nil, path .. ': error', code
      end
      return real_open(path, mode)
   end
   check('exists.opened', shells.exists('C:\\apps\\real.exe'), true)
   check('exists.store_alias_einval', shells.exists(store .. 'pwsh.exe'), true)
   check('exists.store_alias_slashes', shells.exists('C:/Users/x/AppData/Local/Microsoft/WindowsApps/wsl.exe'), true)
   check('exists.store_missing', shells.exists(store .. 'missing.exe'), false)
   check('exists.einval_elsewhere', shells.exists('C:\\apps\\pwsh.exe'), false)
   check('exists.empty_drive', shells.exists('E:\\bin\\pwsh.exe'), false)
   check('exists.enoent', shells.exists('C:\\apps\\missing.exe'), false)
   check('exists.enotdir', shells.exists('/usr/bin/zsh/zsh'), false)
   check('exists.eacces_elsewhere', shells.exists('C:\\apps\\locked.exe'), false)
   io.open = real_open
end

-- PATH parsing: ';' on Windows, case-insensitive de-dup, quotes, relative and invalid entries dropped.
do
   local dirs = shells.path_dirs(
      'C:\\A;c:\\a\\;"C:\\Program Files\\B";relative\\dir;;C:\\bad<name;\\\\server\\share\\bin;C:/Mixed/Slash/;D:',
      true
   )
   check('path.win', table.concat(dirs, '|'), 'C:\\A|C:\\Program Files\\B|\\\\server\\share\\bin|C:\\Mixed\\Slash')
   dirs = shells.path_dirs('/usr/bin:/bin/:relative:/usr/bin::', false)
   check('path.linux', table.concat(dirs, '|'), '/usr/bin|/bin')
   check('path.nil', #shells.path_dirs(nil, true), 0)
end

check('wsl_bash.system32', shells.is_wsl_bash('C:\\WINDOWS\\System32\\bash.exe'), true)
check('wsl_bash.windowsapps', shells.is_wsl_bash('C:\\Users\\x\\AppData\\Local\\Microsoft\\WindowsApps\\bash.exe'), true)
check('wsl_bash.git', shells.is_wsl_bash('C:\\Program Files\\Git\\bin\\bash.exe'), false)

local GX = { root = 'C:\\GX', gx_zsh = 'C:\\GX\\bin\\gx-zsh.exe', herdr = 'C:\\GX\\bin\\herdr.exe' }
local APPS = 'C:\\Users\\x\\AppData\\Local\\Microsoft\\WindowsApps'
local WIN_ENV = {
   PATH = table.concat({
      'C:\\WINDOWS\\system32',
      APPS,
      'C:\\GX\\runtime\\msys64\\usr\\bin',
      'C:\\Program Files\\Git\\cmd',
      'D:\\msys64\\usr\\bin',
      'C:\\tools\\nu\\bin',
   }, ';'),
   SystemRoot = 'C:\\WINDOWS',
   SystemDrive = 'C:',
   ComSpec = 'C:\\WINDOWS\\system32\\cmd.exe',
   ProgramFiles = 'C:\\Program Files',
   LOCALAPPDATA = 'C:\\Users\\x\\AppData\\Local',
   USERPROFILE = 'C:\\Users\\x',
}
local WIN_FILES = {
   'C:\\WINDOWS\\system32\\bash.exe',
   'C:\\WINDOWS\\system32\\cmd.exe',
   'C:\\WINDOWS\\System32\\WindowsPowerShell\\v1.0\\powershell.exe',
   APPS .. '\\pwsh.exe',
   APPS .. '\\bash.exe',
   'C:\\GX\\bin\\gx-zsh.exe',
   'C:\\GX\\bin\\herdr.exe',
   'C:\\GX\\runtime\\msys64\\msys2_shell.cmd',
   'C:\\GX\\runtime\\msys64\\usr\\bin\\env.exe',
   'C:\\GX\\runtime\\msys64\\usr\\bin\\bash.exe',
   'C:\\GX\\runtime\\msys64\\usr\\bin\\git.exe',
   'C:\\Program Files\\Git\\cmd\\git.exe',
   'C:\\Program Files\\Git\\bin\\bash.exe',
   'D:\\msys64\\msys2_shell.cmd',
   'D:\\msys64\\usr\\bin\\env.exe',
   'D:\\msys64\\usr\\bin\\bash.exe',
   'C:\\tools\\nu\\bin\\nu.exe',
}

-- Windows detection: PATH first, GX runtime and WSL bash rejected, WSL from domains.lua.
do
   local list = shells.detect({
      os = 'windows',
      getenv = fake_env(WIN_ENV),
      exists = fake_fs(WIN_FILES),
      gx = GX,
      wsl = { { name = 'WSL:Ubuntu', distribution = 'Ubuntu' }, { name = 'WSL:Debian' } },
   })
   check('win.ids', ids(list), 'gx-zsh,pwsh,powershell,cmd,git-bash,msys2-ucrt64,nu,wsl:Ubuntu,wsl:Debian')
   check('win.gx.args', by_id(list, 'gx-zsh').args[1], GX.gx_zsh)
   check('win.gx.herdr_exe', by_id(list, 'gx-zsh').exe, 'C:\\GX\\runtime\\msys64\\usr\\bin\\zsh.exe')
   check('win.pwsh.alias_on_path', by_id(list, 'pwsh').exe, APPS .. '\\pwsh.exe')
   check('win.pwsh.nologo', by_id(list, 'pwsh').args[2], '-NoLogo')
   check('win.powershell', by_id(list, 'powershell').exe, 'C:\\WINDOWS\\System32\\WindowsPowerShell\\v1.0\\powershell.exe')
   check('win.cmd.comspec', by_id(list, 'cmd').exe, 'C:\\WINDOWS\\system32\\cmd.exe')
   local git = by_id(list, 'git-bash')
   check('win.git.from_path', git.exe, 'C:\\Program Files\\Git\\bin\\bash.exe')
   check('win.git.args', table.concat(git.args, ' ', 2), '--login -i')
   local msys = by_id(list, 'msys2-ucrt64')
   check('win.msys.not_gx_runtime', msys.args[1], 'D:\\msys64\\usr\\bin\\env.exe')
   check('win.msys.args', table.concat(msys.args, ' ', 2), 'MSYSTEM=UCRT64 CHERE_INVOKING=1 /usr/bin/bash --login -i')
   check('win.msys.herdr_exe', msys.exe, 'D:\\msys64\\usr\\bin\\bash.exe')
   check('win.nu', by_id(list, 'nu').exe, 'C:\\tools\\nu\\bin\\nu.exe')
   local wsl = by_id(list, 'wsl:Ubuntu')
   check('win.wsl.domain', wsl.domain, 'WSL:Ubuntu')
   check('win.wsl.no_args', wsl.args, nil)
   check('win.wsl.herdr_exe', wsl.exe, 'C:\\WINDOWS\\System32\\wsl.exe')
   check('win.wsl.name_only', by_id(list, 'wsl:Debian').domain, 'WSL:Debian')
   check('win.local_domain', by_id(list, 'cmd').domain, 'local')
end

-- Only WSL's bash.exe (System32 / WindowsApps) and the GX runtime on PATH: no Git Bash, no MSYS2.
do
   local list = shells.detect({
      os = 'windows',
      getenv = fake_env({
         PATH = 'C:\\WINDOWS\\System32;C:\\WINDOWS\\System32\\usr\\bin;' .. APPS .. ';C:\\GX\\runtime\\msys64\\usr\\bin',
         SystemRoot = 'C:\\WINDOWS',
      }),
      exists = fake_fs({
         'C:\\WINDOWS\\System32\\bash.exe',
         'C:\\WINDOWS\\System32\\usr\\bin\\bash.exe',
         'C:\\WINDOWS\\System32\\msys2_shell.cmd',
         'C:\\WINDOWS\\System32\\usr\\bin\\env.exe',
         APPS .. '\\bash.exe',
         'C:\\GX\\bin\\gx-zsh.exe',
         'C:\\GX\\runtime\\msys64\\msys2_shell.cmd',
         'C:\\GX\\runtime\\msys64\\usr\\bin\\env.exe',
         'C:\\GX\\runtime\\msys64\\usr\\bin\\bash.exe',
         'C:\\WINDOWS\\System32\\cmd.exe',
      }),
      wsl = {},
   })
   check('reject.ids', ids(list), 'cmd')
end

-- Known install locations without PATH entries; no WSL distros.
do
   local list = shells.detect({
      os = 'windows',
      getenv = fake_env({
         SystemRoot = 'C:\\Windows',
         ProgramFiles = 'C:\\Program Files',
         USERPROFILE = 'C:\\Users\\x',
         SystemDrive = 'C:',
      }),
      exists = fake_fs({
         'C:\\Program Files\\PowerShell\\7\\pwsh.exe',
         'C:\\Windows\\System32\\cmd.exe',
         'C:\\Users\\x\\scoop\\apps\\git\\current\\bin\\bash.exe',
         'C:\\Users\\x\\scoop\\apps\\git\\current\\git-bash.exe',
         'C:\\msys64\\msys2_shell.cmd',
         'C:\\msys64\\usr\\bin\\env.exe',
         'C:\\msys64\\usr\\bin\\bash.exe',
      }),
      wsl = {},
   })
   check('known.ids', ids(list), 'pwsh,cmd,git-bash,msys2-ucrt64')
   check('known.pwsh', by_id(list, 'pwsh').exe, 'C:\\Program Files\\PowerShell\\7\\pwsh.exe')
   check('known.cmd_without_comspec', by_id(list, 'cmd').exe, 'C:\\Windows\\System32\\cmd.exe')
   check('known.scoop_git', by_id(list, 'git-bash').exe, 'C:\\Users\\x\\scoop\\apps\\git\\current\\bin\\bash.exe')
   check('known.msys', by_id(list, 'msys2-ucrt64').exe, 'C:\\msys64\\usr\\bin\\bash.exe')
end

-- Linux detection.
do
   local env = fake_env({ PATH = '/usr/local/bin:/usr/bin:/bin' })
   local exists = fake_fs({ '/usr/bin/zsh', '/bin/bash', '/usr/bin/bash' })
   local list = shells.detect({ os = 'linux', getenv = env, exists = exists })
   check('linux.ids', ids(list), 'zsh,bash')
   check('linux.zsh.args', table.concat(by_id(list, 'zsh').args, ' '), '/usr/bin/zsh -l')
   check('linux.bash.path_order', by_id(list, 'bash').exe, '/usr/bin/bash')
   local gx = { root = '/usr/lib', gx_zsh = '/usr/lib/ohmyzsh-gx/bin/gx-zsh', herdr = '/usr/lib/ohmyzsh-gx/bin/herdr' }
   list = shells.detect({ os = 'linux', getenv = env, exists = exists, gx = gx })
   check('linux.gx.ids', ids(list), 'gx-zsh,zsh,bash')
   check('linux.gx.herdr_exe', by_id(list, 'gx-zsh').exe, '/usr/lib/ohmyzsh-gx/libexec/zsh/zsh')
end

-- gui-settings.json choice parsing.
do
   local parsed = {
      ['{"default_shell":"pwsh"}'] = { default_shell = 'pwsh' },
      ['{"default_shell":3}'] = { default_shell = 3 },
      ['{"default_shell":""}'] = { default_shell = '' },
      ['{"wallpaper":"a.png"}'] = { wallpaper = 'a.png' },
      ['[1]'] = { 1 },
      ['"text"'] = 'text',
   }
   local function json_parse(text)
      local value = parsed[text]
      if value == nil then
         error('malformed JSON')
      end
      return value
   end
   check('choice.valid', shells.read_choice('{"default_shell":"pwsh"}', json_parse), 'pwsh')
   check('choice.malformed', shells.read_choice('{"default_shell":', json_parse), nil)
   check('choice.missing_file', shells.read_choice(nil, json_parse), nil)
   check('choice.missing_key', shells.read_choice('{"wallpaper":"a.png"}', json_parse), nil)
   check('choice.non_string', shells.read_choice('{"default_shell":3}', json_parse), nil)
   check('choice.empty', shells.read_choice('{"default_shell":""}', json_parse), nil)
   check('choice.array', shells.read_choice('[1]', json_parse), nil)
   check('choice.scalar', shells.read_choice('"text"', json_parse), nil)
end

-- Fallback order and launch options.
do
   local win = shells.detect({
      os = 'windows',
      getenv = fake_env(WIN_ENV),
      exists = fake_fs(WIN_FILES),
      gx = GX,
      wsl = { { name = 'WSL:Ubuntu', distribution = 'Ubuntu' } },
   })
   local shell, fell_back = shells.resolve(win, nil, 'windows')
   check('resolve.absent_bundled', shell.id, 'gx-zsh')
   check('resolve.absent_no_warning', fell_back, false)
   shell, fell_back = shells.resolve(win, 'git-bash', 'windows')
   check('resolve.valid', shell.id, 'git-bash')
   check('resolve.valid_no_warning', fell_back, false)
   shell, fell_back = shells.resolve(win, 'wsl:Missing', 'windows')
   check('resolve.unknown', shell.id, 'gx-zsh')
   check('resolve.unknown_warns', fell_back, true)

   local standalone = shells.detect({ os = 'windows', getenv = fake_env(WIN_ENV), exists = fake_fs(WIN_FILES) })
   check('resolve.standalone_pwsh', shells.resolve(standalone, nil, 'windows').id, 'pwsh')
   check('resolve.gx_choice_standalone', shells.resolve(standalone, 'gx-zsh', 'windows').id, 'pwsh')
   table.remove(standalone, 1)
   check('resolve.standalone_powershell', shells.resolve(standalone, nil, 'windows').id, 'powershell')
   check('resolve.linux_zsh', shells.resolve({ { id = 'zsh' }, { id = 'bash' } }, nil, 'linux').id, 'zsh')
   check('resolve.linux_first', shells.resolve({ { id = 'bash' } }, 'zsh', 'linux').id, 'bash')
   check('resolve.empty', shells.resolve({}, nil, 'windows'), nil)

   local options = shells.launch_options(win, shells.resolve(win, nil, 'windows'), GX, 'windows')
   check('options.first_gx', options.launch_menu[1].label, 'GX Zsh')
   check('options.herdr_second', options.launch_menu[2].label, 'herdr')
   check('options.herdr_untagged', options.launch_menu[2].set_environment_variables, nil)
   check('options.herdr_args', options.launch_menu[2].args[1], GX.herdr)
   for idx, entry in ipairs(options.launch_menu) do
      check('options.domain.' .. idx, type(entry.domain) == 'table' and type(entry.domain.DomainName), 'string')
      if entry.label ~= 'herdr' then
         check('options.tag.' .. idx, type(entry.set_environment_variables.GX_SHELL_ID), 'string')
      end
   end
   check('options.default_prog', options.default_prog[1], GX.gx_zsh)
   check('options.no_default_domain', options.default_domain, nil)
   check('options.default_index', shells.default_index(options), 1)
   local extra = {}
   for key in pairs(options) do
      if key ~= 'launch_menu' and key ~= 'default_prog' and key ~= 'default_domain' then
         table.insert(extra, key)
      end
   end
   check('options.only_config_keys', table.concat(extra, ','), '')

   options = shells.launch_options(win, shells.resolve(win, 'wsl:Ubuntu', 'windows'), GX, 'windows')
   check('options.wsl.default_domain', options.default_domain, 'WSL:Ubuntu')
   check('options.wsl.local_default_prog', options.default_prog[1], GX.gx_zsh)
   check('options.wsl.default_index', options.launch_menu[shells.default_index(options)].label, 'WSL: Ubuntu')

   options = shells.launch_options(win, shells.resolve(win, 'cmd', 'windows'), GX, 'windows')
   check('options.cmd.default_index', options.launch_menu[shells.default_index(options)].label, 'Command Prompt')
end

check('outcome.ok', shells.herdr_outcome(true, '', ''), 'ok')
local CUSTOM = 'Oh My Zsh GX: herdr uses a custom configuration (%s); default_shell was not changed\n'
check('outcome.custom_file', shells.herdr_outcome(false, '', CUSTOM:format('C:\\p\\herdr\\config.toml')), 'custom')
check('outcome.custom_env', shells.herdr_outcome(false, '', CUSTOM:format('HERDR_CONFIG_PATH=D:\\h.toml')), 'custom')
check('outcome.phrase_on_stdout_only', shells.herdr_outcome(false, CUSTOM:format('x'), ''), 'error')
check('outcome.other_wording', shells.herdr_outcome(false, '', 'herdr config is not managed; not changed\n'), 'error')
check('outcome.env_name_only', shells.herdr_outcome(false, '', 'failed to read HERDR_CONFIG_PATH\n'), 'error')
check('outcome.error', shells.herdr_outcome(false, '', 'Oh My Zsh GX: Access is denied. (os error 5)'), 'error')
check('outcome.usage', shells.herdr_outcome(false, '', 'Oh My Zsh GX: Usage: herdr --gx-set-default-shell <path>'), 'error')

-- gui-settings.json location matches config/src/gui_settings.rs::settings_file_in_dir.
do
   local path = require('utils.gui-settings').path
   local none = fake_env({})
   check('settings.next_to_config', path('/home/u/.config/wezterm', '/home/u', none), '/home/u/.config/wezterm/gui-settings.json')
   check('settings.config_file_flag', path('/tmp/iso', '/home/u', none), '/tmp/iso/gui-settings.json')
   check('settings.bare_config_file', path('', '/home/u', none), 'gui-settings.json')
   check('settings.home_config', path('/home/u', '/home/u', none), '/home/u/.config/wezterm/gui-settings.json')
   check('settings.home_trailing_slash', path('/home/u/', '/home/u', none), '/home/u/.config/wezterm/gui-settings.json')
   check('settings.home_xdg', path('/home/u', '/home/u', fake_env({ XDG_CONFIG_HOME = '/x' })), '/x/wezterm/gui-settings.json')
   check(
      'settings.home_env_is_home',
      path('/home/u', '/home/u', fake_env({ WEZTERM_CONFIG_DIR = '/home/u/', XDG_CONFIG_HOME = '/x' })),
      '/x/wezterm/gui-settings.json'
   )
   check(
      'settings.home_env_elsewhere',
      path('/home/u', '/home/u', fake_env({ WEZTERM_CONFIG_DIR = '/elsewhere', XDG_CONFIG_HOME = '/x' })),
      '/elsewhere/gui-settings.json'
   )
   check('settings.env_ignored_outside_home', path('/cfg', '/home/u', fake_env({ WEZTERM_CONFIG_DIR = '/e' })), '/cfg/gui-settings.json')
   check(
      'settings.windows_home',
      path('c:/Users/x/', 'C:\\Users\\x', none),
      'C:\\Users\\x/.config/wezterm/gui-settings.json'
   )
   check(
      'settings.windows_config',
      path('C:\\Users\\x\\.config\\wezterm', 'C:\\Users\\x', none),
      'C:\\Users\\x\\.config\\wezterm/gui-settings.json'
   )
end

-- The first tagged launch_menu row is the effective default whenever no valid choice is saved
-- (the settings overlay ticks it in that case); new-tab-button marks the same row.
do
   local function first_tagged(options)
      for idx, entry in ipairs(options.launch_menu) do
         if entry.set_environment_variables then
            return idx, entry.set_environment_variables.GX_SHELL_ID
         end
      end
      return nil
   end
   local no_pwsh = {}
   for _, file in ipairs(WIN_FILES) do
      if not file:find('pwsh', 1, true) then
         table.insert(no_pwsh, file)
      end
   end
   local only_cmd_env = { SystemRoot = 'C:\\WINDOWS', ComSpec = 'C:\\WINDOWS\\system32\\cmd.exe' }
   local scenarios = {
      { 'bundled', 'windows', { getenv = fake_env(WIN_ENV), exists = fake_fs(WIN_FILES), gx = GX }, 'gx-zsh' },
      { 'standalone', 'windows', { getenv = fake_env(WIN_ENV), exists = fake_fs(WIN_FILES) }, 'pwsh' },
      { 'no_pwsh', 'windows', { getenv = fake_env(WIN_ENV), exists = fake_fs(no_pwsh) }, 'powershell' },
      { 'only_cmd', 'windows', { getenv = fake_env(only_cmd_env), exists = fake_fs({ 'C:\\WINDOWS\\system32\\cmd.exe' }) }, 'cmd' },
      { 'linux', 'linux', { getenv = fake_env({ PATH = '/usr/bin' }), exists = fake_fs({ '/usr/bin/zsh', '/usr/bin/bash' }) }, 'zsh' },
      { 'linux_no_zsh', 'linux', { getenv = fake_env({ PATH = '/usr/bin' }), exists = fake_fs({ '/usr/bin/bash' }) }, 'bash' },
      { 'mac', 'mac', {}, 'fish' },
   }
   for _, scenario in ipairs(scenarios) do
      local name, os_name, ctx, want = scenario[1], scenario[2], scenario[3], scenario[4]
      ctx.os = os_name
      local list = shells.detect(ctx)
      for _, choice in ipairs({ false, 'not-installed' }) do
         local label = 'first_row.' .. name .. (choice and '.unknown_choice' or '')
         local default = shells.resolve(list, choice or nil, os_name)
         local options = shells.launch_options(list, default, ctx.gx, os_name)
         local idx, id = first_tagged(options)
         check(label .. '.effective', default.id, want)
         check(label .. '.first_row', id, want)
         check(label .. '.marked', shells.default_index(options), idx)
      end
   end
end

-- config/launch.lua and config/domains.lua with a stubbed wezterm module.
local function deep_copy(value)
   if type(value) ~= 'table' then
      return value
   end
   local out = {}
   for key, item in pairs(value) do
      out[key] = deep_copy(item)
   end
   return out
end

local settings_text = nil
local settings_path = nil
local settings_json = {
   ['{"default_shell":"git-bash"}'] = { default_shell = 'git-bash' },
   ['{"default_shell":"wsl:Ubuntu"}'] = { default_shell = 'wsl:Ubuntu' },
   ['{"default_shell":"msys2-ucrt64"}'] = { default_shell = 'msys2-ucrt64' },
   ['{"default_shell":"zsh"}'] = { default_shell = 'zsh' },
   ['{"default_shell":"bash"}'] = { default_shell = 'bash' },
   ['{"default_shell":"nope"}'] = { default_shell = 'nope' },
}
local real_open, real_getenv = io.open, os.getenv
io.open = function(path, mode)
   if path:match('gui%-settings%.json$') then
      settings_path = path
      if settings_text == nil then
         return nil, path .. ': No such file or directory', 2
      end
      return {
         read = function()
            return settings_text
         end,
         close = function() end,
      }
   end
   return real_open(path, mode)
end

-- Encoded values outlive a stub like real JSON strings in wezterm.GLOBAL do.
local encoded = {}

local function make_wezterm(target_triple, executable_dir, global, config_dir)
   local stub = {
      target_triple = target_triple,
      executable_dir = executable_dir,
      config_dir = config_dir or 'C:\\Users\\x\\.config\\wezterm',
      home_dir = 'C:\\Users\\x',
      GLOBAL = global,
      handlers = {},
      warnings = {},
      spawned = {},
      wsl_calls = 0,
      wsl_result = nil,
      child_result = { true, '', '' },
      action = {
         ShowDefaultShellSettings = 'ShowDefaultShellSettings',
         InputSelector = function(args)
            return { InputSelector = args }
         end,
         SpawnCommandInNewTab = function(command)
            return { SpawnCommandInNewTab = command }
         end,
      },
      nerdfonts = setmetatable({}, {
         __index = function()
            return '*'
         end,
      }),
   }
   function stub.action_callback(callback)
      return callback
   end
   function stub.log_info() end
   function stub.format(items)
      local text = {}
      for _, item in ipairs(items) do
         if type(item) == 'table' and item.Text then
            table.insert(text, item.Text)
         end
      end
      return table.concat(text)
   end
   function stub.json_encode(value)
      table.insert(encoded, deep_copy(value))
      return 'json#' .. #encoded
   end
   function stub.json_parse(text)
      local index = tonumber(text:match('^json#(%d+)$'))
      if index then
         return deep_copy(encoded[index])
      end
      local value = settings_json[text]
      if value == nil then
         error('malformed JSON')
      end
      return value
   end
   function stub.default_wsl_domains()
      stub.wsl_calls = stub.wsl_calls + 1
      if stub.wsl_result == 'raise' then
         error('wsl.exe failed')
      end
      return stub.wsl_result or { { name = 'WSL:Ubuntu', distribution = 'Ubuntu', default_cwd = '~' } }
   end
   function stub.on(name, callback)
      stub.handlers[name] = callback
   end
   function stub.log_warn(message)
      table.insert(stub.warnings, message)
   end
   function stub.run_child_process(args)
      table.insert(stub.spawned, args)
      if stub.child_result == 'raise' then
         error('program not found')
      end
      return table.unpack(stub.child_result)
   end
   return stub
end

local function load_launch(wezterm, env, files)
   package.loaded.wezterm = wezterm
   for _, name in ipairs({ 'utils.platform', 'utils.wsl', 'config.domains', 'config.launch', 'events.new-tab-button' }) do
      package.loaded[name] = nil
   end
   shells.exists = fake_fs(files)
   os.getenv = fake_env(env)
   local options = require('config.launch')
   shells.exists = real_exists
   os.getenv = real_getenv
   return options
end

local function fake_window()
   local window = { toasts = {} }
   function window:toast_notification(title, message)
      table.insert(self.toasts, { title = title, message = message })
   end
   return window
end

-- utils/wsl.lua: a found distro list is kept for the GUI process; an empty or failed listing is
-- retried after RETRY_S (300 s), and when the clock steps back.
do
   local wezterm = make_wezterm('x86_64-pc-windows-msvc', 'C:\\GX\\wezterm', {})
   package.loaded.wezterm = wezterm
   package.loaded['utils.wsl'] = nil
   local wsl = require('utils.wsl')
   wezterm.wsl_result = {}
   check('wsl.empty', #wsl.domains(1000), 0)
   check('wsl.empty.calls', wezterm.wsl_calls, 1)
   wsl.domains(1299)
   check('wsl.empty.cached_briefly', wezterm.wsl_calls, 1)
   wezterm.wsl_result = 'raise'
   check('wsl.failed', #wsl.domains(1300), 0)
   check('wsl.empty.retried', wezterm.wsl_calls, 2)
   wsl.domains(1400)
   check('wsl.failed.cached_briefly', wezterm.wsl_calls, 2)
   wezterm.wsl_result = nil
   check('wsl.clock_back.retried', wsl.domains(900)[1].name, 'WSL:Ubuntu')
   check('wsl.clock_back.calls', wezterm.wsl_calls, 3)
   check('wsl.found.plain_table', type(wsl.domains(100900)[1]), 'table')
   check('wsl.found.kept', wezterm.wsl_calls, 3)
   package.loaded['utils.wsl'] = nil
end

do
   local global = {}
   local wezterm = make_wezterm('x86_64-pc-windows-msvc', 'C:\\GX\\wezterm', global)
   local options = load_launch(wezterm, WIN_ENV, WIN_FILES)
   check('launch.no_child_process', #wezterm.spawned, 0)
   check('launch.wsl_listed_once', wezterm.wsl_calls, 1)
   check('launch.default_prog', options.default_prog[1], GX.gx_zsh)
   check('launch.wsl_entry', options.launch_menu[#options.launch_menu].domain.DomainName, 'WSL:Ubuntu')
   for idx, entry in ipairs(options.launch_menu) do
      check('launch.domain.' .. idx, type(entry.domain.DomainName), 'string')
      check('launch.tag.' .. idx, entry.set_environment_variables == nil, entry.label == 'herdr')
   end
   local domains = require('config.domains')
   check('domains.plain_table', type(domains.wsl_domains[1]), 'table')
   check('domains.name', domains.wsl_domains[1].name, 'WSL:Ubuntu')

   -- Reloads reuse the GLOBAL cache: wsl.exe runs once per GUI process.
   wezterm = make_wezterm('x86_64-pc-windows-msvc', 'C:\\GX\\wezterm', global)
   load_launch(wezterm, WIN_ENV, WIN_FILES)
   check('domains.cached_across_reload', wezterm.wsl_calls, 0)

   settings_text = '{"default_shell":"git-bash"}'
   options = load_launch(wezterm, WIN_ENV, WIN_FILES)
   check('launch.choice.default_prog', options.default_prog[1], 'C:\\Program Files\\Git\\bin\\bash.exe')
   check('launch.settings_path', settings_path, 'C:\\Users\\x\\.config\\wezterm/gui-settings.json')

   -- ~/.wezterm.lua: the config dir is HOME, so the sidecar is the one WezTerm itself uses.
   local home_config = make_wezterm('x86_64-pc-windows-msvc', 'C:\\GX\\wezterm', global, 'C:\\Users\\x')
   options = load_launch(home_config, WIN_ENV, WIN_FILES)
   check('launch.home_config.settings_path', settings_path, 'C:\\Users\\x/.config/wezterm/gui-settings.json')
   check('launch.home_config.choice', options.default_prog[1], 'C:\\Program Files\\Git\\bin\\bash.exe')
   local xdg_env = {}
   for key, value in pairs(WIN_ENV) do
      xdg_env[key] = value
   end
   xdg_env.XDG_CONFIG_HOME = 'D:\\xdg'
   load_launch(home_config, xdg_env, WIN_FILES)
   check('launch.home_config.xdg', settings_path, 'D:\\xdg/wezterm/gui-settings.json')
   settings_path = nil
   home_config.handlers['gx-default-shell-changed'](fake_window(), nil)
   check('launch.home_config.event_reads_same_file', settings_path, 'D:\\xdg/wezterm/gui-settings.json')

   settings_text = '{"default_shell":"nope"}'
   options = load_launch(wezterm, WIN_ENV, WIN_FILES)
   check('launch.fallback.default_prog', options.default_prog[1], GX.gx_zsh)
   check('launch.fallback.warns', #wezterm.warnings, 1)

   settings_text = '{"default_shell":'
   options = load_launch(wezterm, WIN_ENV, WIN_FILES)
   check('launch.malformed.default_prog', options.default_prog[1], GX.gx_zsh)

   settings_text = '{"default_shell":"wsl:Ubuntu"}'
   options = load_launch(wezterm, WIN_ENV, WIN_FILES)
   check('launch.wsl.default_domain', options.default_domain, 'WSL:Ubuntu')

   -- gx-default-shell-changed: map the choice to herdr's absolute shell and say honestly what
   -- herdr panes get (herdr takes one executable: no WSL distro, no UCRT64 environment).
   local handler = wezterm.handlers['gx-default-shell-changed']
   check('event.registered', type(handler), 'function')
   local cases_by_choice = {
      { '{"default_shell":"git-bash"}', 'C:\\Program Files\\Git\\bin\\bash.exe', 'herdr 新窗格将使用 Git Bash' },
      { nil, 'C:\\GX\\runtime\\msys64\\usr\\bin\\zsh.exe', 'herdr 新窗格将使用 GX Zsh' },
      {
         '{"default_shell":"wsl:Ubuntu"}',
         'C:\\WINDOWS\\System32\\wsl.exe',
         'herdr 新窗格将使用 WSL 默认发行版（herdr 不能指定发行版）',
      },
      {
         '{"default_shell":"msys2-ucrt64"}',
         'D:\\msys64\\usr\\bin\\bash.exe',
         'herdr 新窗格将使用 MSYS2 bash（MSYS 环境，不是 UCRT64）',
      },
   }
   for _, case in ipairs(cases_by_choice) do
      settings_text = case[1]
      wezterm.spawned = {}
      local window = fake_window()
      handler(window, nil)
      local args = wezterm.spawned[1]
      check('event.herdr.' .. case[2], table.concat(args, ' '), 'C:\\GX\\bin\\herdr.exe --gx-set-default-shell ' .. case[2])
      check('event.ok_toast.' .. case[2], window.toasts[1].title, 'GX Shell')
      check('event.ok_message.' .. case[2], window.toasts[1].message, case[3])
   end

   settings_text = nil
   wezterm.child_result = { false, '', CUSTOM:format('C:\\p\\herdr\\config.toml') }
   local window = fake_window()
   handler(window, nil)
   check('event.custom', window.toasts[1].message, 'herdr 使用自定义 Shell 配置，未修改')

   wezterm.child_result = { false, '', 'Access is denied.\r\n' }
   window = fake_window()
   handler(window, nil)
   check('event.error.title', window.toasts[1].title, 'herdr 默认 Shell 同步失败')
   check('event.error.message', window.toasts[1].message, 'Access is denied.')

   wezterm.child_result = 'raise'
   window = fake_window()
   handler(window, nil)
   check('event.raise.title', window.toasts[1].title, 'herdr 默认 Shell 同步失败')
   check('event.raise.message', window.toasts[1].message:find('program not found', 1, true) ~= nil, true)

   -- Standalone WezTerm: no bundle, no herdr call.
   wezterm = make_wezterm('x86_64-pc-windows-msvc', 'C:\\Program Files\\WezTerm', global)
   options = load_launch(wezterm, WIN_ENV, WIN_FILES)
   check('standalone.default_prog', options.default_prog[1], APPS .. '\\pwsh.exe')
   check('standalone.no_herdr_entry', options.launch_menu[2].label, 'PowerShell 5.1')
   window = fake_window()
   wezterm.handlers['gx-default-shell-changed'](window, nil)
   check('standalone.no_herdr_call', #wezterm.spawned, 0)
   check('standalone.no_toast', #window.toasts, 0)
end

do
   local wezterm = make_wezterm('x86_64-unknown-linux-gnu', '/usr/lib/wezterm-gx', {})
   settings_text = nil
   local options = load_launch(wezterm, { PATH = '/usr/bin:/bin' }, {
      '/usr/lib/ohmyzsh-gx/bin/gx-zsh',
      '/usr/lib/ohmyzsh-gx/bin/herdr',
      '/usr/bin/zsh',
      '/bin/bash',
   })
   check('linux.launch.no_wsl', wezterm.wsl_calls, 0)
   check('linux.launch.labels', options.launch_menu[1].label .. ',' .. options.launch_menu[2].label, 'GX Zsh,herdr')
   check('linux.launch.default_prog', options.default_prog[1], '/usr/lib/ohmyzsh-gx/bin/gx-zsh')
   local handler = wezterm.handlers['gx-default-shell-changed']
   handler(fake_window(), nil)
   check('linux.event.herdr', wezterm.spawned[1][3], '/usr/lib/ohmyzsh-gx/libexec/zsh/zsh')

   -- The herdr server runs with the GX Zsh environment, so a system zsh there loads the GX
   -- profile anyway: herdr is pointed at GX Zsh and the toast says so.
   settings_text = '{"default_shell":"zsh"}'
   local window = fake_window()
   handler(window, nil)
   check('linux.event.system_zsh.herdr', wezterm.spawned[2][3], '/usr/lib/ohmyzsh-gx/libexec/zsh/zsh')
   check('linux.event.system_zsh.toast', window.toasts[1].message, 'herdr 新窗格将使用 GX Zsh（系统 zsh 在 herdr 里也会加载 GX 配置）')
   settings_text = '{"default_shell":"bash"}'
   window = fake_window()
   handler(window, nil)
   check('linux.event.bash.herdr', wezterm.spawned[3][3], '/bin/bash')
   check('linux.event.bash.toast', window.toasts[1].message, 'herdr 新窗格将使用 Bash')
   settings_text = nil
end

-- Right click on the `+` button: every launch_menu row with its own domain, the effective default
-- marked once, then the settings entry; also when only standalone shells exist.
do
   local no_pwsh = {}
   for _, file in ipairs(WIN_FILES) do
      if not file:find('pwsh', 1, true) then
         table.insert(no_pwsh, file)
      end
   end
   local scenarios = {
      { 'bundled', 'C:\\GX\\wezterm', WIN_FILES, 'GX Zsh' },
      { 'standalone', 'C:\\Program Files\\WezTerm', WIN_FILES, 'PowerShell 7' },
      { 'standalone_no_pwsh', 'C:\\Program Files\\WezTerm', no_pwsh, 'PowerShell 5.1' },
   }
   for _, scenario in ipairs(scenarios) do
      local name, files, want = scenario[1], scenario[3], scenario[4]
      settings_text = nil
      local options = load_launch(make_wezterm('x86_64-pc-windows-msvc', scenario[2], {}), WIN_ENV, files)
      local new_tab = require('events.new-tab-button')
      local choices, choices_data = new_tab.build_choices({}, {})
      local marked = {}
      for idx, choice in ipairs(choices) do
         if choice.label:find('（默认）', 1, true) then
            table.insert(marked, idx)
         end
      end
      check('new_tab.' .. name .. '.marked_once', #marked, 1)
      check('new_tab.' .. name .. '.first_is_default', choices[1].label:find(want .. '（默认）', 1, true) ~= nil, true)
      check('new_tab.' .. name .. '.settings_last', choices[#choices].id, 'open-settings')
      check('new_tab.' .. name .. '.rows', #choices_data, #options.launch_menu)
      for idx, data in ipairs(choices_data) do
         check('new_tab.' .. name .. '.domain.' .. idx, data.domain.DomainName, options.launch_menu[idx].domain.DomainName)
      end
   end
end

-- Clicking `+`: left keeps the default action; right lists launch_menu rows plus the SSH/Unix
-- domains of the window's effective config, and the last entry opens Settings on the Shell section.
do
   settings_text = nil
   local wezterm = make_wezterm('x86_64-pc-windows-msvc', 'C:\\GX\\wezterm', {})
   local options = load_launch(wezterm, WIN_ENV, WIN_FILES)
   require('events.new-tab-button').setup()
   local click = wezterm.handlers['new-tab-button-click']
   local window = { performed = {} }
   function window:perform_action(action, _pane)
      table.insert(self.performed, action)
   end
   function window:effective_config()
      return { ssh_domains = { { name = 'prod' } }, unix_domains = { { name = 'unix' } } }
   end

   check('click.left.returns_false', click(window, 'pane', 'Left', 'default-spawn'), false)
   check('click.left.default_action', window.performed[1], 'default-spawn')

   click(window, 'pane', 'Right', 'launcher')
   local selector = window.performed[2].InputSelector
   local rows = #options.launch_menu
   check('click.right.rows', #selector.choices, rows + 3)
   check('click.right.ssh', selector.choices[rows + 1].label:find('prod', 1, true) ~= nil, true)
   check('click.right.unix', selector.choices[rows + 2].label:find('unix', 1, true) ~= nil, true)
   selector.action(window, 'pane', 'open-settings', 'x')
   check('click.settings_action', window.performed[3], 'ShowDefaultShellSettings')
   selector.action(window, 'pane', tostring(rows + 1), 'prod')
   check('click.ssh_spawn', window.performed[4].SpawnCommandInNewTab.domain.DomainName, 'prod')
   selector.action(window, 'pane', '1', 'GX Zsh')
   local spawn = window.performed[5].SpawnCommandInNewTab
   check('click.spawn.args', spawn.args[1], GX.gx_zsh)
   check('click.spawn.domain', spawn.domain.DomainName, 'local')
   check('click.spawn.tag', spawn.set_environment_variables.GX_SHELL_ID, 'gx-zsh')
   selector.action(window, 'pane', nil, nil)
   check('click.cancel', #window.performed, 5)
end

io.open, os.getenv = real_open, real_getenv
print(string.format('PASS: %d shell detection, default shell and herdr sync cases', cases))
