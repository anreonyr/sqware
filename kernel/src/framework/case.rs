//! 用例（framework/case.rs）— `.tests` 段里的一行。
//!
//! `repr(C)` 是**链接契约**：`discover()` 把段当 `[Case]` 切片读，而段的边界由
//! `link.ld` 按字节给出 —— 布局必须由语言保证，不能靠编译器心情。

/// 一个用例：叫得出名字（汇报用）、调得动（跑它）。
///
/// 只有两个字段，因为用例只需要这两样：`filter` / `tags` / `seed` 之类都没有用户，
/// 等真有了再加（无用户的字段就是飞线）。
#[repr(C)]
pub(crate) struct Case {
    /// 用例名（`test!` 的字符串字面量，`'static`）。
    pub(crate) name: &'static str,
    /// 用例体。`fn()` 而非 `Box<dyn Fn>`：无分配、可进静态量。
    pub(crate) body: fn(),
}
