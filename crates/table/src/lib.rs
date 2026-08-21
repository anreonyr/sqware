//! table — 无堆表格渲染：地址显示 + 有界格子表格，交给 papergrid 渲染。
//!
//! 本 crate 提供两件事，均锁死无分配（崩溃现场无堆无锁）：
//!   1. 地址显示 —— render_addr/addr_width：地址 → 符号 / 分组 hex，
//!      符号器经 set_symbolizer 全局注入（未注入回退 hex）。
//!   2. 表格 —— Table 收集有界 Cell，render 走 papergrid 的
//!      CompactGrid::build(&mut fmt::Write)（Display 路径）。
//!
//! 不变量：
//!   · 渲染绝不调用 to_string()（那是 std 分配出口）——只有
//!     render(&mut impl fmt::Write) 一条出口。
//!   · papergrid 的 Dimension::get_width 语义 = 整列总宽（内容 + padding），
//!     内部再减回 padding 得内容摆放区；故 render 按内容宽 + CELL_PAD 喂入。
//!   · 对齐是 papergrid 的单一全局水平对齐（无逐列）——depend 原 emit_row
//!     语义即各列左对齐、地址格按列宽补格，恰好契合。

#![no_std]

use core::cell::UnsafeCell;
use core::fmt::{self, Write};
use core::sync::atomic::{AtomicBool, Ordering};

use arrayvec::ArrayString;
use papergrid::{
    colors::NoColors,
    config::{compact::CompactConfig, AlignmentHorizontal, Borders, Indent, Sides},
    dimension::Dimension,
    grid::compact::CompactGrid,
    records::IterRecords,
};

// ── 地址显示 ─────────────────────────────────────────────────

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
fn sym_fn() -> Option<&'static SymFn> {
    if SYM.set.load(Ordering::Acquire) {
        // SAFETY: set 后 val 已写入且此后只读。
        unsafe { *SYM.val.get() }
    } else {
        None
    }
}

/// 地址的显示宽度（符号串或分组 hex 的字符数），供列宽计算。
pub fn addr_width(a: usize) -> usize {
    match sym_fn().and_then(|f| f(a)) {
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
    match sym_fn().and_then(|f| f(a)) {
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

// ── 表格 ────────────────────────────────────────────────────

/// 行内对齐。
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Right,
}

/// 一个格子：有界栈字符串。写满自动截断（不 panic）——崩溃现场语义。
pub type Cell<const CAP: usize> = ArrayString<CAP>;

/// 渲染输出缓冲：一段有界栈字符串，调用方整块直写自己的 sink。
pub type Line<const CAP: usize> = ArrayString<CAP>;

/// 每格左右 padding 各 PAD 空格 → 每列需预留的 padding 总和。
/// base_config 与之同源，改 padding 只动这一处。
const PAD: usize = 1;
const CELL_PAD: usize = PAD * 2;

/// 表格：全量收集有界格子，render 时交给 papergrid 渲染。
///
/// 约束：row/col 数、cell 容量都是编译期常量；渲染统一全局对齐
/// （默认 Left，可 set_align）；列宽=显式值或该列最大 cell 宽。
pub struct Table<const ROWS: usize, const COLS: usize, const CAP: usize> {
    grid: [[Cell<CAP>; COLS]; ROWS],
    nrows: usize, // 已用行数（其余行忽略）
    width: [Option<usize>; COLS],
    align: Align,
}

impl<const ROWS: usize, const COLS: usize, const CAP: usize> Table<ROWS, COLS, CAP> {
    pub fn new() -> Self {
        Self {
            grid: [[ArrayString::new(); COLS]; ROWS],
            nrows: 0,
            width: [None; COLS],
            align: Align::Left,
        }
    }

    /// 当前已填行数。
    pub fn rows(&self) -> usize {
        self.nrows
    }

    /// 在 (row, col) 建一格，返回格的可写引用（write! 或 render_addr 直进）。
    pub fn cell(&mut self, row: usize, col: usize) -> &mut Cell<CAP> {
        assert!(row < ROWS && col < COLS, "table: cell out of bounds");
        self.nrows = self.nrows.max(row + 1);
        &mut self.grid[row][col]
    }

    /// 设全局对齐（默认 Left；Right 用于地址列整列右对齐）。
    pub fn set_align(&mut self, align: Align) {
        self.align = align;
    }

    /// 显式设某列宽（None=render 时取该列最大 cell 宽）。
    pub fn col_width(&mut self, col: usize, w: usize) {
        self.width[col] = Some(w);
    }

    /// 渲染整表到 sink（无堆：走 papergrid build 的 Display 路径）。
    ///
    /// papergrid 的 Dimension.get_width 语义 = 整列总宽（内容 + 左右 padding），
    /// 故这里按内容宽 + CELL_PAD 喂给它。
    pub fn render<W: Write>(&self, out: &mut W) -> fmt::Result {
        if self.nrows == 0 {
            return Ok(());
        }
        let mut content = [0usize; COLS];
        (0..COLS).for_each(|c| {
            content[c] = match self.width[c] {
                Some(w) => w,
                None => {
                    let mut m = 0usize;
                    for r in 0..self.nrows {
                        m = m.max(self.grid[r][c].chars().count());
                    }
                    m
                }
            };
        });
        let dim = ConstDims { content };
        let cfg = base_config(self.align);
        // 只取前 nrows 行：&self.grid[..nrows] 借 self，build 即时消费，无 fake 'static。
        let records = IterRecords::new(&self.grid[..self.nrows], COLS, Some(self.nrows));
        CompactGrid::new(records, cfg, dim, NoColors).build(out)
    }
}

impl<const ROWS: usize, const COLS: usize, const CAP: usize> Default for Table<ROWS, COLS, CAP> {
    fn default() -> Self {
        Self::new()
    }
}

/// papergrid 的列宽/行高。
struct ConstDims<const COLS: usize> {
    content: [usize; COLS],
}

impl<const COLS: usize> Dimension for ConstDims<COLS> {
    /// 返回整列总宽（内容 + padding）——papergrid 会计回 available = 总宽 - padding。
    fn get_width(&self, column: usize) -> usize {
        self.content[column] + CELL_PAD
    }
    fn get_height(&self, _row: usize) -> usize {
        1
    }
}

/// 默认配置：无边框、每格左右各 PAD 空格 padding，全局对齐。
fn base_config(align: Align) -> CompactConfig {
    let halign = match align {
        Align::Left => AlignmentHorizontal::Left,
        Align::Right => AlignmentHorizontal::Right,
    };
    CompactConfig::new()
        .set_borders(Borders::empty())
        .set_padding(Sides::new(
            Indent::spaced(PAD),
            Indent::spaced(PAD),
            Indent::zero(),
            Indent::zero(),
        ))
        .set_alignment_horizontal(halign)
}
