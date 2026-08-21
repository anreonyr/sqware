//! 表格 —— 行导向有界格子，自研渲染（无 papergrid）。
//!
//! 行导向构建：open_row 推一行（游标自增，不数行号）、blank_row 空行分隔；
//! render 逐行直写 sink。锁死无分配出口：只走 &mut fmt::Write，绝不 to_string()（std 分配出口）。
//! 不变量：同列所有格等宽（列宽 = 该列 max cell 宽或显式 set_col_width）；结构即保证。

use core::fmt::{self, Write};

use arrayvec::ArrayString;

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

/// 每格左右 padding 各 PAD 空格。
const PAD: usize = 1;

/// 表格：行导向有界格子，render 逐行直写。
///
/// 约束：row/col 数、cell 容量都是编译期常量；对齐 per-column（默认 Left）。
pub struct Table<const ROWS: usize, const COLS: usize, const CAP: usize> {
    grid: [[Cell<CAP>; COLS]; ROWS],
    nrows: usize, // 行游标：已填行数（open_row/blank_row 自增，其余行忽略）
    align: [Align; COLS],
    width: [Option<usize>; COLS],
}

impl<const ROWS: usize, const COLS: usize, const CAP: usize> Table<ROWS, COLS, CAP> {
    /// 建空表。全部容量编译期定死。
    pub fn new() -> Self {
        Self {
            grid: [[ArrayString::new(); COLS]; ROWS],
            nrows: 0,
            align: [Align::Left; COLS],
            width: [None; COLS],
        }
    }

    /// 已填行数。
    pub fn num_rows(&self) -> usize {
        self.nrows
    }

    /// 列数（编译期常量）。
    pub const fn num_cols(&self) -> usize {
        COLS
    }

    /// 空表判定。
    pub fn is_empty(&self) -> bool {
        self.nrows == 0
    }

    /// 推一行：返回整行格子的可变引用（游标自增）。动态值（地址/格式化）
    /// 直写；常量下标越界=编译错。满 ROWS 后继续推 panic。
    pub fn open_row(&mut self) -> &mut [Cell<CAP>; COLS] {
        assert!(self.nrows < ROWS, "table: rows exhausted");
        let row = self.nrows;
        self.nrows += 1;
        &mut self.grid[row]
    }

    /// 空行分隔（游标自增，格留空——渲染为该行两列 padding 空格）。
    pub fn blank_row(&mut self) {
        assert!(self.nrows < ROWS, "table: rows exhausted");
        self.nrows += 1;
    }

    /// 设某列对齐（默认 Left；Right 用于地址列整列右对齐）。
    pub fn set_col_align(&mut self, col: usize, align: Align) {
        assert!(col < COLS, "table: column out of bounds");
        self.align[col] = align;
    }

    /// 设某列显式宽（None=render 时取该列最大 cell 宽）。
    pub fn set_col_width(&mut self, col: usize, w: usize) {
        assert!(col < COLS, "table: column out of bounds");
        self.width[col] = Some(w);
    }

    /// 渲染整表到 sink（无堆：逐行直写）。
    ///
    /// 每格 = 左 PAD 空格 + 按列对齐 pad 到列宽 + 右 PAD 空格；行间换行。
    /// 末行不补尾换行（调用方定）；空表 no-op。
    pub fn render<W: Write>(&self, out: &mut W) -> fmt::Result {
        if self.nrows == 0 {
            return Ok(());
        }
        let mut width = [0usize; COLS];
        for c in 0..COLS {
            width[c] = match self.width[c] {
                Some(w) => w,
                None => {
                    let mut m = 0usize;
                    for r in 0..self.nrows {
                        m = m.max(self.grid[r][c].chars().count());
                    }
                    m
                }
            };
        }
        for r in 0..self.nrows {
            if r > 0 {
                out.write_char('\n')?;
            }
            for c in 0..COLS {
                let cell = &self.grid[r][c];
                // 左 padding：第 0 列顶格（行首无空格），其后列各 PAD 空格
                if c > 0 {
                    for _ in 0..PAD {
                        out.write_char(' ')?;
                    }
                }
                // 按列对齐 pad 到列宽
                let fill = width[c].saturating_sub(cell.chars().count());
                if self.align[c] == Align::Right {
                    for _ in 0..fill {
                        out.write_char(' ')?;
                    }
                    out.write_str(cell.as_str())?;
                } else {
                    out.write_str(cell.as_str())?;
                    for _ in 0..fill {
                        out.write_char(' ')?;
                    }
                }
                // 右 padding PAD 空格
                for _ in 0..PAD {
                    out.write_char(' ')?;
                }
            }
        }
        Ok(())
    }
}

impl<const ROWS: usize, const COLS: usize, const CAP: usize> Default for Table<ROWS, COLS, CAP> {
    fn default() -> Self {
        Self::new()
    }
}
