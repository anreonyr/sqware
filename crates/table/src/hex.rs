//! 地址显示 —— 把地址渲染成 符号 / 分组 hex，并与宽度计算同源。
//!
//! 渲染与 addr_width 共用同一套判定：有符号 → name+0xoff，否则分组 hex
//! 0x8089_2208（四位一组、最高非零组起）。两者同源是表格列对齐不失真的前提。

use core::fmt::{self, Write};

use crate::sym;

/// 地址的显示宽度（符号串或分组 hex 的字符数），供列宽计算。
pub fn addr_width(a: usize) -> usize {
    match sym::resolve(a) {
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
    match sym::resolve(a) {
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
