//! para —— 段落渲染：标题顶格 + 空行 + 内容缩进（诊断输出的统一段落形状）。
//!
//! 段落 = 一段独立输出的最小自含单位：标题行（顶格）→ 空行 → 内容（缩进 2）。
//! 语义约束：一标题 = 一张表——段内多表列宽各自 auto 无法对齐，合并单表才是
//! 对齐正道（boot banner / crash scene 均如此）。缩进包装是段落私有工具，
//! 不对外裸漏：缩进语义只属于段落层。

use core::fmt::{self, Write};

use crate::table::Table;

/// 段落渲染器：把「标题 + 整表」按段落形状写出（标题经 title、表体经 table）。
pub struct Para<W: Write> {
    out: W,
}

impl<W: Write> Para<W> {
    /// 包一个 sink（console / 栈缓冲）。
    pub fn new(out: W) -> Self {
        Self { out }
    }

    /// 标题段头："\n{args}\n\n"（顶格标题 + 与其后内容空一行）。
    pub fn title(&mut self, args: fmt::Arguments<'_>) {
        let _ = writeln!(self.out, "\n{args}");
        let _ = writeln!(self.out);
    }

    /// 整表按段落写出：内容缩进 2 空格 + 补尾换行。
    pub fn table<const C: usize, const R: usize, const S: usize>(&mut self, t: &Table<C, R, S>) {
        let mut ind = Indented::new(&mut self.out);
        let _ = t.render(&mut ind);
        drop(ind);
        let _ = writeln!(self.out);
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