local wezterm = require('wezterm')
local platform = require('utils.platform')
local font_files = require('utils.font-files')

-- local font_family = 'Maple Mono NF'
-- Windows：GDI/DirectWrite 对 "JetBrainsMono Nerd Font"（GDI legacy 族名）只认
-- Regular 面——粗体变体解析落空后回退到微软雅黑（比例字体），表现为粗体行
-- 字母间距错乱。typographic 族名 "JetBrainsMono NF" 两路都能解析全部字重
-- （验证：wezterm ls-fonts 的 When Intensity=Bold 段应命中 NerdFont Bold 而非
-- Microsoft YaHei / 内置 JetBrains Mono）。macOS/Linux 用原名。
local font_family = platform.is_win and 'JetBrainsMono NF' or 'JetBrainsMono Nerd Font'
-- local font_family = 'CartographCF Nerd Font'

-- 本机沿用旧字号 12（源机器为 12.5）。
local font_size = platform.is_mac and 12 or 12

local fallback = {
   { family = font_family, weight = 'Regular' },
   -- 正文字重保持 Regular，选中和标题交给 TUI 自己强调；CJK 轻微校正高度。
   { family = 'Noto Sans CJK SC', weight = 'Regular', scale = 1.05 },
}

-- 末位兜底：Noto Sans CJK 2.001 不含 Unicode 14 新增的 U+9FFD–9FFF 等码位，缺字形时
-- wezterm 会弹 "No fonts contain glyphs" 告警。Unifont 覆盖整个 BMP，只在前面都
-- 没有字形时才会被选中。仅当字体文件存在才加入，避免没装的机器反而多一条
-- 「字体未找到」告警。盲文与方块字符由 wezterm 自绘（custom_block_glyphs），不走这里。
local unifont = io.open('/usr/share/fonts/truetype/unifont/unifont.ttf', 'r')
if unifont then
   unifont:close()
   table.insert(fallback, { family = 'Unifont' })
end

-- Windows 的彩色 emoji 与中文兜底，同样只在字体文件存在时加入。
-- 回退链是 JetBrainsMono NF → Noto Sans CJK SC → Segoe UI Emoji：微软雅黑是比例字体，
-- 字形风格与 Noto 不同，只在找不到 Noto Sans CJK 文件（独立安装的 WezTerm 没有随包的
-- 那份）时才作为 CJK 兜底补进来，排在 emoji 之前。Noto 文件在系统字体目录、用户字体目录
-- 或 GX Shell 安装包的 fonts 目录里任一处即可。
if platform.is_win then
   local dirs = font_files.windows_font_dirs(os.getenv, wezterm.executable_dir)
   if
      not font_files.any_exists(dirs, font_files.NOTO_CJK_FILES)
      and font_files.any_exists(dirs, { 'msyh.ttc' })
   then
      table.insert(fallback, { family = 'Microsoft YaHei' })
   end
   if font_files.any_exists(dirs, { 'seguiemj.ttf' }) then
      table.insert(fallback, { family = 'Segoe UI Emoji', assume_emoji_presentation = true })
   end
end

-- East Asian Ambiguous 宽度：herdr（src/ui/text.rs::display_width，
-- UnicodeWidthStr::width 非 _cjk 版本）与 wezterm 的
-- treat_east_asian_ambiguous_width_as_wide（未设置，走默认 false）都按窄
-- 字符处理，两侧口径已对齐，无需改动，此处只留档避免重复排查。

return {
   font = wezterm.font_with_fallback(fallback),
   font_size = font_size,
}
