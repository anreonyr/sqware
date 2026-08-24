//! addr —— 地址显示：全局符号化 + 分组 hex 渲染，渲染与宽度同源。
//!
//! 原 crates/table 的 hex + sym 两模块合并（sym 的唯一公共面就是地址显示，
//! 合一是"形态重设计"的最小当量）：set_symbolizer 全局注入一次，
//! render_addr / addr_width 共用同一判定——有符号 → name+0xoff，否则分组 hex
//! 0x8089_2208（四位一组、最高非零组起）。两者同源是表格列对齐不失真的前提。

use core::cell::UnsafeCell;
use core::fmt::{self, Write};
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

/// 地址的显示宽度（符号串或分组 hex 的字符数），供列宽计算。
pub fn addr_width(a: usize) -> usize {
    match resolve(a) {
        Some((name, off)) => name.len() + 3 + hex_digits(off), // "+0x" + 偏移
        None => grouped_hex_len(a),
    }
}

fn hex_digits(mut v: usize) -> usize {
    let mut n = 1;
    while v > 15 {
        v >>= 4;
        n += 1;
    }
    n
}

/// 分组 hex 文本长度："0x" + 4·组数 + (组数-1) 下划线，从最高非零组起。
fn grouped_hex_len(a: usize) -> usize {
    let mut groups: usize = 0;
    let mut started = false;
    for sig in (0..=3).rev() {
        let g = (a >> (sig * 16)) & 0xffff;
        if !started {
            if g == 0 && sig > 0 {
                continue;
            }
            started = true;
        }
        groups += 1;
    }
    2 + groups * 4 + groups.saturating_sub(1)
}

/// 渲染一个地址进 sink（符号化优先 / 四位分组 hex）。
pub fn render_addr<W: Write>(w: &mut W, a: usize) -> fmt::Result {
    match resolve(a) {
        Some((name, off)) => write!(w, "{name}+{off:#x}"),
        None => {
            const HX: &[u8; 16] = b"0123456789abcdef";
            w.write_str("0x")?;
            let mut started = false;
            for sig in (0..=3).rev() {
                let g = (a >> (sig * 16)) & 0xffff;
                if !started {
                    if g == 0 && sig > 0 {
                        continue;
                    }
                    started = true;
                } else {
                    w.write_char('_')?;
                }
                for c in [
                    HX[g >> 12],
                    HX[(g >> 8) & 0xf],
                    HX[(g >> 4) & 0xf],
                    HX[g & 0xf],
                ] {
                    w.write_char(c as char)?;
                }
            }
            Ok(())
        }
    }
}