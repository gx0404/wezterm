//! zh-CN 译表：按 key（英文原文）字典序排序，binary_search 查找。
//! 新增条目必须保持有序（`i18n::tests::zh_table_sorted_unique` 守门）。
//! 带参数的文案存 `{name}` 模板，取词后经 `i18n::fill` 填参。

use std::borrow::Cow;

pub(crate) static ZH_CN: &[(&str, &str)] = &[
    // 条目自 R2（GUI 文案）与 R5（CLI 帮助）轮次持续补充
];

/// Look up `key` in the zh-CN table, falling back to the key itself
pub fn translate(key: &'static str) -> Cow<'static, str> {
    ZH_CN
        .binary_search_by(|(k, _)| (*k).cmp(key))
        .ok()
        .map(|idx| Cow::Borrowed(ZH_CN[idx].1))
        .unwrap_or(Cow::Borrowed(key))
}
