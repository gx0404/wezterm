-- Run with lua5.4; all filesystem/process effects are stubbed.
local source = 'dotfiles/plugins/httpssCssZssZsgithubsDscomsZsMLFlexersZsresurrectsDswezterm/plugin/resurrect/utils.lua'
local calls, exists, succeed, deferred = {}, false, true, {}
package.preload.wezterm = function()
   return {
      target_triple = 'x86_64-pc-windows-msvc',
      time = { call_after = function(_, callback) deferred[#deferred + 1] = callback end },
      read_dir = function()
         if not exists then error('missing directory') end
         return {}
      end,
      run_child_process = function(args)
         calls[#calls + 1] = args
         exists = succeed
         return succeed, '', succeed and '' or 'access denied'
      end,
   }
end
os.execute = function() error('must not open a shell window') end
local utils = dofile(source)
local path = [[C:/用户/O'Brien & 100%/session]]
utils.ensure_folder_exists(path)
assert(#calls == 1)
assert(calls[1][1] == 'powershell.exe')
assert(calls[1][6] == "[System.IO.Directory]::CreateDirectory('C:/用户/O''Brien & 100%/session') | Out-Null")
utils.ensure_folder_exists(path)
assert(#calls == 1, 'existing directory must not spawn a process')
exists, succeed = false, false
local ok, err = pcall(utils.ensure_folder_exists, path)
assert(not ok and err:find('access denied', 1, true))
utils.is_windows = false
succeed = true
utils.ensure_folder_exists('/tmp/space ; quote\'')
assert(calls[#calls][1] == 'mkdir' and calls[#calls][3] == '--')
assert(calls[#calls][4] == '/tmp/space ; quote\'')
package.loaded['resurrect.utils'] = utils
package.loaded['resurrect.file_io'] = { write_state = function() end }
package.preload['gx-require-probe'] = function()
   local state = dofile(source:gsub('utils.lua$', 'state_manager.lua'))
   state.change_state_save_dir('/tmp/state/')
   return state
end
exists = false
local before = #calls
local state = require('gx-require-probe')
assert(state.save_state_dir == '/tmp/state/')
assert(#calls == before and #deferred == 1, 'require must defer asynchronous process calls')
deferred[1]()
assert(#calls > before, 'deferred initialization must create directories')
exists = false
before = #calls
state.save_state({ workspace='saved', window_states={} })
assert(#calls > before, 'save must ensure its directory even before the timer runs')
print('PASS: resurrect directory creation, quoting, existing paths and failure reporting')
