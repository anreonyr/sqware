//! render —— stanza 表格渲染适配（替代自建表格 crates/table 的 Table/Para）。
//!
//! 形态（D4/s2 已批 + 示例实证，见 examples/stanza-demo）：
//!   · 无边框纯文本：Console Decor 全 suppress——第 0 列顶格、列间 1 空格；
//!   · 定宽列（MinWidth==MaxWidth 锁死）+ **建格时按列宽截断**（复刻"单行截断"；
//!     stanza 的 wrap 是换行坍缩——裸 &str 进 with_row 会被拆成多行，示例已实证）；
//!   · 收集 → 打印：每块（标题 + 表）在 push 时把表渲成 String 收进全局收集器
//!     （OnceLock<RefCell<Vec<(String, String)>>>，首次 push 在 panic → 分配落
//!     spare；**控制台保持静默**），最后 render_all 统一出图（标题顶格 + 空行 +
//!     缩进 2 + 空行，旧 Para 语义）。
//!
//! 为何收集器只存 String：stanza `Table`/`Styles` 含 `Box<dyn Style>` 与
//! `Content::Computed(Box<dyn Fn …>)` 等 !Send trait object——T 非 Send 则
//! `RefCell<Vec<T>>` 非 Sync、进不了 static；渲成 String（纯数据）即安全。
//!
//! 崩溃现场：portal 已无锁切到 Backend::Spare（halt 报警核），本模块的隐式分配
//! （建表 + 渲染 String + 收集器 Vec）全部落 spare 仓——预算即契约。

use core::fmt::Write;

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use stanza::renderer::console::{Console, Decor};
use stanza::renderer::Renderer;
use stanza::style::{MaxWidth, MinWidth, Styles};
use stanza::table::{Cell, Col, Content, Row, Table};

use crate::lock::SpinLock;

/// 诊断表总宽预算（旧 set_total_width(64) 的复刻基准；行末 pad 到总宽）。
pub const TOTAL_WIDTH: usize = 64;

/// 崩溃转储收集器：先收块（表已渲成文本）、后统一打印（控制台保持静默直到
/// render_all）。SpinLock 无条件 Sync（T: Vec<(String,String)> Send）；渲染文本
/// 分配发生在 spare 仓。boot 一律不触碰本静态。dump 单核、锁恒单独持有
/// （exempt，无层级——depend 不校验）。
static DUMPS: SpinLock<Vec<(String, String)>> = SpinLock::new(Vec::new());

/// 定宽列样式：MinWidth == MaxWidth → 列宽锁死，行呈固定栅格。
pub fn fixed(w: usize) -> Styles {
    Styles::default().with(MinWidth(w)).with(MaxWidth(w))
}

/// 建定宽列表（列样式锁死，供 with_row 填行）。
pub fn fixed_table(widths: &[usize]) -> Table {
    Table::default().with_cols(widths.iter().map(|&w| Col::new(fixed(w))).collect())
}

/// 一行的 N 格：每格按列宽截断建格（char 安全）——复刻单行截断；全文本仍在
/// export JSON 侧双写（见调用方 scene_row）。裸 &str 进 with_row 的换行坍缩陷阱
/// 由此杜绝（stanza wrap 只在截断后仍超宽的极端单字上发生，指标不含）。
pub fn row<const N: usize>(widths: &[usize; N], cols: [&str; N]) -> Row {
    let cells = cols
        .iter()
        .zip(widths)
        .map(|(s, &w)| cell(s, w))
        .collect();
    Row::new(Styles::default(), cells)
}

/// 建格：源文本按列宽截断（char 安全）。
pub fn cell(s: &str, w: usize) -> Cell {
    Cell::new(Styles::default(), Content::Label(trunc(s, w)))
}

/// 建格（不截断）：boot 横幅等自然布局列用。
pub fn plain(s: &str) -> Cell {
    Cell::new(Styles::default(), Content::Label(trunc(s, usize::MAX)))
}

/// 按字符数截断（至多 w 字符；多字节安全）。
fn trunc(s: &str, w: usize) -> String {
    if s.chars().count() <= w {
        s.to_string()
    } else {
        s.chars().take(w).collect()
    }
}

/// 表 → 无边框纯文本（Decor 全 suppress 的一次性渲染）。
pub fn render(t: &Table) -> String {
    let decor = Decor::default()
        .suppress_escape_codes()
        .suppress_outer_border()
        .suppress_inner_horizontal_border()
        .suppress_all_lines();
    Console(decor).render(t)
}

/// 收块入收集器（表此刻渲成文本；控制台不动）。
pub fn push(title: String, table: &Table) {
    let text = render(table);
    DUMPS.lock().push((title, text));
}

/// 收一行独立文本入收集器（如 depend 的 rule 行；空标题；打印时缩进 2）。
pub fn push_line(text: &str) {
    DUMPS.lock().push((String::new(), text.to_string()));
}

/// 统一打印：遍历收集的块，每块「标题顶格（空标题跳过）→ 空行 → 缩进 2 的
/// 表格文本 → 空行」。崩溃单核打印（报警核，其余核已 hunker），一次收敛输出。
/// 空标题 = 同段落续表（[scene] 标题只挂首表，csr/gpr/kbt/ubt 共享一个段落）。
pub fn render_all<W: Write>(out: &mut W) {
    let blocks = DUMPS.lock();
    for (title, text) in blocks.iter() {
        if !title.is_empty() {
            let _ = writeln!(out, "{title}");
            let _ = writeln!(out);
        }
        let mut ind = Indented::new(out, 2);
        let _ = ind.write_str(text);
        let _ = writeln!(out);
    }
}

/// 段落渲染：表中渲染出 String 后缩进 `indent` 空格直写 sink（boot 横幅等
/// 即时输出路径用；崩溃路径走 push → render_all）。
pub fn render_to<W: Write>(out: &mut W, t: &Table, indent: usize) {
    let s = render(t);
    let mut ind = Indented::new(out, indent);
    let _ = ind.write_str(&s);
}

/// 行首缩进包装：把多行输出整体右移 `indent` 空格（每个非空行行首补缩进；
/// 空行不补）。段落私有工具（旧 crates/table 的 Indented 随迁）。
struct Indented<'a, W: Write> {
    out: &'a mut W,
    at_bol: bool,
    indent: usize,
}

impl<'a, W: Write> Indented<'a, W> {
    fn new(out: &'a mut W, indent: usize) -> Self {
        Self {
            out,
            at_bol: true,
            indent,
        }
    }
}

impl<W: Write> Write for Indented<'_, W> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let mut rest = s;
        while let Some(i) = rest.find('\n') {
            if self.at_bol {
                for _ in 0..self.indent {
                    self.out.write_char(' ')?;
                }
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
                for _ in 0..self.indent {
                    self.out.write_char(' ')?;
                }
                self.at_bol = false;
            }
            self.out.write_str(rest)?;
        }
        Ok(())
    }
}