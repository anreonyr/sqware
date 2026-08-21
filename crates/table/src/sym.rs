//! 地址符号化 —— 全局注入一次的符号器。
//!
//! 调用方（boot/适配层）经 set_symbolizer 注入「addr → (名,偏移)」回调，
//! hex 显示层据此把地址渲染成符号；未注入则回退裸 hex。set-once、幂等。

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, Ordering};

/// 地址符号化回调：addr → (函数名, 偏移)。由 boot/适配层注入。
pub type SymFn = dyn Fn(usize) -> Option<(&'static str, usize)> + Sync + 'static;

/// set-once 全局符号器（AtomicBool 门 + UnsafeCell 承载）。
struct OnceSym {
    set: AtomicBool,
    val: UnsafeCell<Option<&'static SymFn>>,
}
// SAFETY: 写入恰好一次；写者先置 set(true) 再写 val；读者在 set=true 后读，
// 不会与写者并发（装配发生在 boot 单核、早于一切符号化调用）。
unsafe impl Sync for OnceSym {}
static SYM: OnceSym = OnceSym {
    set: AtomicBool::new(false),
    val: UnsafeCell::new(None),
};

/// 注入地址符号化回调（boot 装配后调用一次；重复注入忽略=幂等）。
pub fn set_symbolizer(f: &'static SymFn) {
    if SYM.set.swap(true, Ordering::AcqRel) {
        return;
    }
    // SAFETY: 首次注入，写者唯一；读者在 set 后访问。
    unsafe { *SYM.val.get() = Some(f) };
}

/// 取已注入的符号化回调（未注入 → None）。
pub(crate) fn resolve(a: usize) -> Option<(&'static str, usize)> {
    if SYM.set.load(Ordering::Acquire) {
        // SAFETY: set 后 val 已写入且此后只读。
        unsafe { *SYM.val.get() }.and_then(|f| f(a))
    } else {
        None
    }
}
