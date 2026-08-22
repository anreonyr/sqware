//! para —— 段落渲染：标题顶格 + 内容缩进 + 块间统一空行（诊断输出形状）。
//!
//! 间距契约：**任何输出块（标题行 / 表格）之间恰一个空行**——title 无前导
//! （段首），table 前自动 gap（段内表间/标题后），段尾空行由调用方补（供下一段
//! 或结尾分隔）。缩进包装是段落私有工具，不对外裸漏。

use core::fmt::{self, Write};

use crate::table::Table;

/// 段落渲染器：把「标题 + 整表」按段落形状写出（标题经 title、表体经 table）。
pub struct Para<W: Write> {
    out: W,
    at_start: bool,
}

impl<W: Write> Para<W> {
    /// 包一个 sink（console / 栈缓冲）。at_start：本段落尚未输出（首块无前导间距）。
    pub fn new(out: W) -> Self {
        Self {
            out,
            at_start: true,
        }
    }

    /// 标题（段首，顶格）。标题与内容的空行由后续首个 table 的前导 gap 提供。
    pub fn title(&mut self, args: fmt::Arguments<'_>) {
        let _ = writeln!(self.out, "{args}");
        self.at_start = false;
    }

    /// 整表（缩进 2 空格）：段内表间自动补 1 空行（gap）。段尾空行由调用方补。
    pub fn table<const C: usize, const R: usize, const S: usize>(&mut self, t: &Table<C, R, S>) {
        if !self.at_start {
            let _ = writeln!(self.out);
        }
        self.at_start = false;
        let mut ind = Indented::new(&mut self.out);
        let _ = t.render(&mut ind);
        drop(ind);
    }
}

/// 行首缩进包装：把多行输出整体右移 2 空格（每个新行行首补缩进；空行不补）。
/// 段落私有工具——缩进只随段落使用，不对外裸漏。
struct Indented<W: Write> {
    out: W,
    at_bol: bool,
}

impl<W: Write> Indented<W> {
    fn new(out: W) -> Self {
        Self { out, at_bol: true }
    }
}

impl<W: Write> Write for Indented<W> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let mut rest = s;
        while let Some(i) = rest.find('\n') {
            if self.at_bol {
                self.out.write_str("  ")?;
                self.at_bol = false;
            }
            if i > 0 {
                self.out.write_str(&rest[..i])?;
            }
            self.out.write_char('\n')?;
            self.at_bol = true;
            rest = &rest[i + 1..];
        }
        if !rest.is_empty() {
            if self.at_bol {
                self.out.write_str("  ")?;
                self.at_bol = false;
            }
            self.out.write_str(rest)?;
        }
        Ok(())
    }
}
