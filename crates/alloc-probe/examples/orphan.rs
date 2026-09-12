//! 最小演示：**孤儿缓冲**（分配了、指针丢了）——smartalloc 的看家判据。
//!
//! ```sh
//! cargo run --target x86_64-unknown-linux-gnu --features smartalloc --example orphan
//! ```
//! 期望输出（末尾）：`Orphaned buffer: 8 bytes allocated at line …`。
//!
//! 它与"对象没析构"是两回事：这里没有对象，只有一段丢了指针的内存 —— 正是本 crate
//! 要跟"分配器自身的账"分开看的那一类。

fn main() {
    #[cfg(feature = "smartalloc")]
    unsafe {
        use core::alloc::{GlobalAlloc, Layout};

        let a = smartalloc::SmartAlloc;
        let layout = Layout::from_size_align(8, 8).unwrap();
        let p = a.alloc(layout); // 指针就地丢掉 ⇒ 孤儿缓冲
        assert!(!p.is_null());
        smartalloc::sm_dump(true);
    }
    #[cfg(not(feature = "smartalloc"))]
    println!("请加 --features smartalloc（那条 feature 才是「架空内核分配器」的路）");
}
