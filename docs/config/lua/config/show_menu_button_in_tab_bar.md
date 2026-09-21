# `show_menu_button_in_tab_bar`

{{since('nightly')}}

Fork addition, default `true`.

Renders a `☰` main-menu button at the right end of the tab bar (both the
fancy and the retro tab bar). Clicking it opens the main menu
([ShowMainMenu](../keyassignment/ShowMainMenu.md)). The button has its
own `MenuButton` mouse-binding `region`, so it can be bound to a
different action per
[mouse binding](../mouse.md).

```lua
config.show_menu_button_in_tab_bar = false
```
