//! 表格 —— 无堆容器渲染：Table 装 Cell、Cell 装 Str、Str 由格式化器产。
//!
//! 层级（签名即文档，格式化与容纳分离是类型义务）：
//!   Fmt（格式化器，产 &str）→ Cell（容器，装 Str + 自对齐）→ Table（容器，装 Cell）
//!
//! 渲染：列宽 per-column Width 约束（内容需求 clamp 到 [min,max]，max 超宽显示
//! 截断）；第 0 列顶格、列间 1 空格；每格由 Cell::pad 按自对齐成型。行位置由
//! Table 管理（rows_mut 迭代器推进）；行耗尽返回 None，调用方静默跳过——诊断
//! 路径零 panic。段落形状（标题顶格 + 内容缩进）归 para，本模块只渲染表体。

use core::fmt::{self, Write};

use arrayvec::ArrayString;

/// 列内对齐（每格自带，默认 Left）。
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Right,
}

/// 渲染输出缓冲：一段有界栈字符串（ArrayString 别名；Fmt 行缓冲同用它）。
pub type Line<const CAP: usize> = ArrayString<CAP>;

/// 格 = Str 容器 + 自对齐。
///
/// 内容超 S 截断（不 panic）；不实现 Write——str 必由格式化器（Fmt）产生，直写
/// 格子的形态不可表达（格式化与容纳分离）。空串 = 空格子（渲染为该列宽空格）。
#[derive(Clone, Copy)]
pub struct Cell<const S: usize> {
    buf: ArrayString<S>,
    align: Align,
}

impl<const S: usize> Cell<S> {
    /// 建格即放内容（空串也合法）。
    pub fn new(s: &str) -> Self {
        let mut buf = ArrayString::new();
        let _ = buf.push_str(s);
        Self {
            buf,
            align: Align::Left,
        }
    }

    /// 设自对齐（默认 Left）。链式：`Cell::new(s).align(Align::Right)`。
    pub fn align(mut self, a: Align) -> Self {
        self.align = a;
        self
    }

    /// 内容读出。
    pub fn as_str(&self) -> &str {
        self.buf.as_str()
    }

    /// 成型：按自对齐把内容成型到 width 后直写 sink（Left 补右侧 / Right 补左侧）。
    /// 内容超 width = 显示截断（只输出前 width 字符）——列 max 硬上限；内容本体
    /// 与导出（as_str）仍全文，双写不损。
    pub fn pad<W: Write>(&self, out: &mut W, width: usize) -> fmt::Result {
        let s = self.buf.as_str();
        let len = s.chars().count();
        if len > width {
            for ch in s.chars().take(width) {
                out.write_char(ch)?;
            }
            return Ok(());
        }
        let fill = width - len;
        match self.align {
            Align::Left => {
                out.write_str(s)?;
                for _ in 0..fill {
                    out.write_char(' ')?;
                }
            }
            Align::Right => {
                for _ in 0..fill {
                    out.write_char(' ')?;
                }
                out.write_str(s)?;
            }
        }
        Ok(())
    }
}

/// 列宽约束：显示列宽 = 内容需求 clamped 到 [min, max]——min 补足（内容不足补
/// 空格）、max 硬上限（内容超宽显示截断；Cell 内容与导出仍全文，双写不损）。
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Width {
    pub min: usize,
    pub max: usize,
}

impl Width {
    /// 免约束：纯 auto（min 0、max 无界）。
    pub const AUTO: Width = Width {
        min: 0,
        max: usize::MAX,
    };

    /// 定宽（min == max）：整列恒 w。
    pub const fn fixed(w: usize) -> Width {
        Width { min: w, max: w }
    }
}

/// 表 = Cell 的容器（R 行 × C 列，格容量 S）。列宽 per-column 约束（Width），
/// 第 0 列顶格、列间 1 空格由 render 保证。
pub struct Table<const C: usize, const R: usize, const S: usize> {
    grid: [[Cell<S>; C]; R],
    nrows: usize,
    width: [Width; C],
}

impl<const C: usize, const R: usize, const S: usize> Table<C, R, S> {
    /// 建空表（格子全空串，列宽全 AUTO）。
    pub fn new() -> Self {
        Self {
            grid: [[Cell {
                buf: ArrayString::new(),
                align: Align::Left,
            }; C]; R],
            nrows: 0,
            width: [Width::AUTO; C],
        }
    }

    /// 设某列宽约束：`Width::fixed(10)`（定宽）或 `Width { min, max }`（区间）。
    pub fn set_width(&mut self, c: usize, w: Width) {
        debug_assert!(c < C, "table: column out of bounds");
        self.width[c] = w;
    }

    /// 行的可变迭代器：next 推进一行并返回该行格子（行位置由本表管理，调用方
    /// 不必数行号）。行耗尽返回 None——调用方静默跳过，诊断路径不 panic。
    pub fn rows_mut(&mut self) -> RowsMut<'_, C, S> {
        RowsMut {
            rows: self.grid.as_mut_slice().as_mut_ptr(),
            left: R,
            nrows: &mut self.nrows,
            _marker: core::marker::PhantomData,
        }
    }

    /// 已填行数（渲染范围）。
    pub fn num_rows(&self) -> usize {
        self.nrows
    }

    /// 渲染整表到 sink：列宽 auto、第 0 列顶格、列间 1 空格、行间换行，末行不补
    /// 尾换行（段落收尾归 para::Para）。空表 no-op。渲染层不截断（列宽按内容）。
    pub fn render<W: Write>(&self, out: &mut W) -> fmt::Result {
        if self.nrows == 0 {
            return Ok(());
        }
        // 列宽 = 每列内容需求 clamped 到该列 Width 约束：[min, max]。
        let mut width = [0usize; C];
        for c in 0..C {
            let mut need = 0usize;
            for r in 0..self.nrows {
                need = need.max(self.grid[r][c].buf.as_str().chars().count());
            }
            width[c] = need.clamp(self.width[c].min, self.width[c].max);
        }
        for r in 0..self.nrows {
            if r > 0 {
                out.write_char('\n')?;
            }
            for c in 0..C {
                if c > 0 {
                    out.write_char(' ')?; // 列间 1 空格；第 0 列顶格
                }
                self.grid[r][c].pad(out, width[c])?;
                out.write_char(' ')?; // 列尾间距（对齐既有 PAD 语义）
            }
        }
        Ok(())
    }
}

impl<const C: usize, const R: usize, const S: usize> Default for Table<C, R, S> {
    fn default() -> Self {
        Self::new()
    }
}

/// 行迭代器：每次 next 借出下一行（指针 + 剩余计数，std IterMut 同款形态），
/// 同步推进行计数（渲染范围）。借用期 'a 由 PhantomData 统一约束。
pub struct RowsMut<'a, const C: usize, const S: usize> {
    rows: *mut [Cell<S>; C],
    left: usize,
    nrows: &'a mut usize,
    _marker: core::marker::PhantomData<&'a mut [[Cell<S>; C]]>,
}

impl<'a, const C: usize, const S: usize> Iterator for RowsMut<'a, C, S> {
    type Item = &'a mut [Cell<S>; C];

    fn next(&mut self) -> Option<Self::Item> {
        if self.left == 0 {
            return None;
        }
        // SAFETY: rows 指向 left 个未借出的行；每次借出首行后指针前进、计数递减，
        // 各行互不重叠（迭代互斥由自身状态保证）；'a 是构造时的 Table 可变借用期。
        unsafe {
            let row = &mut *self.rows;
            self.rows = self.rows.add(1);
            self.left -= 1;
            *self.nrows += 1;
            Some(row)
        }
    }
}
