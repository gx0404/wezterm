# config-lua：Lua 配置系统

## 范围

`config/`（含 derive/）、`luahelper/`、`lua-api-crates/`（15 个 crate）、
`wezterm-dynamic/`、`env-bootstrap/`、`sync-color-schemes/`。
从 wezterm.lua 到 `ConfigHandle` 的完整链路与 Lua API 面。

## 符号真源

- 加载链：`config/src/lib.rs::Config::load → load_with_overrides`；候选路径
  次序 = exe 同目录（Windows）→ `WEZTERM_CONFIG_FILE` → `--config-file` →
  `$HOME/.wezterm.lua` → XDG `wezterm/wezterm.lua`（`xdg_config_home()`）。
- Lua 上下文：`config/src/lua.rs::make_lua_context`——mlua 0.9；注册
  `wezterm` 模块、config_builder（严格 metatable：`__newindex` 即时
  `Config::from_dynamic` 校验并打印栈回溯）、`wezterm.on/emit`、
  `action_callback`；`package.searchers[2]` 被替换以把 require 的文件加入
  reload 监视列表。
- 注册中心：`env-bootstrap/src/lib.rs::register_lua_modules` 把 15 个
  lua-api-crates push 进 `config::lua::add_context_setup_func`；window-funcs
  例外，由 wezterm-gui 单独注册（GUI 进程才可用）。
- 动态值层：`wezterm-dynamic/src/value.rs::Value`（对齐 Lua 类型集且为
  TOML/JSON 超集）+ `FromDynamic/ToDynamic`（含 UnknownFieldAction）。
  Lua↔Rust 一律经 `luahelper::to_lua/from_lua` 的 Dynamic 往返。
- 句柄与重载：`config/src/lib.rs::ConfigHandle{config, generation}`；
  `reload()` 由 notify watcher（200ms 去抖）触发；监视集合 = 配置文件 +
  父目录（HOME 除外）+ `add_to_config_reload_watch_list` 登记。
- 配色数据：`config/src/scheme_data.rs::SCHEMES` 由 sync-color-schemes
  生成（见下方禁止项）；`build_default_schemes()` 消费。
- derive：`config/derive` 的 `ConfigMeta`（文档/字段元信息，供
  docs/config/lua 参考生成）。

## 不变量

- **Lua 线程模型**：Lua 是 Send 但 !Sync，只允许主线程引用；后台 fs-watch
  线程经 `LuaPipe`（smol channel）把新上下文运回主线程
  （`with_lua_config_on_main_thread` 在非主线程调用会 panic）。任何"在别的
  线程碰 Lua"的改动都是错的。
- **重载失败语义**：成功才替换配置并清错；失败保留旧配置、仅更新错误消息
  （`ConfigInner::reload` 注释）。不得让坏配置清空运行时行为。
- **generation 计数**：`TerminalConfiguration::generation` 每次配置变更必须
  递增，消费方（终端缓存、GUI 字体缓存）靠它刷新。
- **严格 builder**：config_builder 的 `__newindex` 即时校验新值；新增配置
  字段必须带 FromDynamic 语义（unknown/deprecated 字段行为显式声明），否则
  用户拼写错误将被静默吞掉。
- **事件返回值**：`wezterm.on` 的 handler 返回 false 可阻断默认动作；
  `EventState` 队列上限见 gui-rendering 域。

## 禁止项

- 不手改 `config/src/scheme_data.rs` 与 `docs/colorschemes/**`——只能经
  `sync-color-schemes` 重建（它抓取上游主题仓库生成）。
- 不在 lua-api-crates 之间互相 import 造成注册顺序耦合；注册顺序唯一真源
  是 env-bootstrap。
- 新 Lua API 必须同时考虑：rustdoc 注释（docs/config/lua 由其生成，见
  code-comments）+ `since` 标注 + config-builder 严格校验。

## 验证

- 定向：`cargo nextest run -p config -p wezterm-dynamic`。
- 配置解析改动：用 `--config 'key=value'` 内联覆盖冒烟（无需写文件）；
  重载行为用 `WEZTERM_CONFIG_FILE` 指向临时文件 + touch 触发 watcher 人工
  核对。
- Lua API 面变更：`ci/generate-docs.py` 相关索引重建走 `make generated-check`
  比对；新增公开函数的文档锚点要进 KB（`make kb`）。
