//! 表格 —— 有界格子表格，交给 papergrid 渲染。
//!
//! Table 全量收集有界 Cell（编译期常量 row/col/容量），render 时交给
//! papergrid 的 CompactGrid 渲染。锁死无分配出口：只走
//! build(&mut fmt::Write)（Display 路径），绝不 to_string()（std 分配出口）。

use core::fmt::{self, Write};

use arrayvec::ArrayString;
use papergrid::{
    colors::NoColors,
    config::{compact::CompactConfig, AlignmentHorizontal, Borders, Indent, Sides},
    dimension::Dimension,
    grid::compact::CompactGrid,
    records::IterRecords,
};

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
