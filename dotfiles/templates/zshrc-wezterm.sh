# >>> wezterm-gx >>>
# WezTerm 在普通/应用光标模式下会发送两组不同序列，两组都显式覆盖。
autoload -Uz up-line-or-beginning-search down-line-or-beginning-search
zle -N up-line-or-beginning-search
zle -N down-line-or-beginning-search
zstyle ':zle:up-line-or-beginning-search' leave-cursor yes
zstyle ':zle:down-line-or-beginning-search' leave-cursor yes
for keymap in emacs viins; do
  bindkey -M "$keymap" '^[[A' up-line-or-beginning-search
  bindkey -M "$keymap" '^[OA' up-line-or-beginning-search
  bindkey -M "$keymap" '^[[B' down-line-or-beginning-search
  bindkey -M "$keymap" '^[OB' down-line-or-beginning-search
done
# <<< wezterm-gx <<<
