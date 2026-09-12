//! 最小演示：**孤儿缓冲**（分配了、指针丢了）—— 而且走的是**全局分配器**。
//!
//! ```sh
//! ./run.sh orphan      # 等价于 cargo run --target x86_64-unknown-linux-gnu --example orphan
//! ```
//!
//! debug 档的宿主堆由 `smartalloc` 接管（见 `src/lib.rs` 的接管段），故这里不需要任何
//! 显式 API：普通的一次 `Box::new` 丢掉指针，收尾转储就会点名它。
//!
//! **已知边界（crate 自己的）**：接管模式下报告的 `FILE:LINE` 恒是**全局分配器声明处**
//! （Rust 的 `__rust_alloc` shim 不透传 caller），不是泄漏点 —— 上游 README 也这么写
//! （"refers to the `#[global_allocator]` itself and can be ignored"）。要真实分配点，
//! 得像 crate 原本的用法那样显式调 `SmartAlloc`（那种用法在 `--no-default-features` 档
//! 已经不在本 crate 里了）。

fn main() {
    // 故意漏一块：`Box` 的指针随 `forget` 丢掉 ⇒ smartalloc 账上留着它。
    let leaked = Box::new([0u8; 8]);
    core::mem::forget(leaked);

    println!("== 收尾转储（应当列出上面那块 8 字节）：");
    alloc_probe::dump_orphans();
}
