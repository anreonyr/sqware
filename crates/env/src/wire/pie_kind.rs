//! pie_kind — 门闩那一类的**判别号**（[`PieKind`]）。
//!
//! # 为什么它住 `env`
//!
//! 装配那一侧要**两侧读**——内核的 `build.rs`（宿主）与编排域（riscv）——而 `programs` /
//! `protocol` 都依赖 execution（含 riscv 内联汇编）。故凡是"装配那一侧要摆出来的
//! 东西"，定义都得住 `env`。

const KIND_POLE: u8 = PieKind::Pole as u8;
const KIND_NOLE: u8 = PieKind::Nole as u8;
const KIND_HOLE: u8 = PieKind::Hole as u8;
const KIND_TOLE: u8 = PieKind::Tole as u8;

/// **要的是哪一种门闩**（**判别号即线格式**：`repr(u8)`）。
///
/// 资源目录与授出方使用同一枚种类标签。
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PieKind {
    /// 一段内存（设备寄存器页 / 自描述区 / 载荷区）。
    Pole,
    /// 空载荷的信号（中断门铃）。
    Nole,
    Hole,
    Tole,
}

impl PieKind {
    /// 线上那一格解回（**判别号不认识 ⇒ `None`**——读的人按"这一帧读不懂"处置，不猜）。
    ///
    /// **它为什么住类型自己身上**（impl 跟着类型走）：这一格的读者有两条
    /// （驱动那几枚 `Ask` 与设备账认领那一帧的解回）——两处各写一遍 `match 0/1/…` 就是
    /// 两份判别号表。
    pub const fn of(raw: u8) -> Option<PieKind> {
        match raw {
            KIND_POLE => Some(PieKind::Pole),
            KIND_NOLE => Some(PieKind::Nole),
            KIND_HOLE => Some(PieKind::Hole),
            KIND_TOLE => Some(PieKind::Tole),
            _ => None,
        }
    }
}

impl crate::wire::Field for PieKind {
    const WIDTH: usize = 1;
    fn store(&self, out: &mut [u8]) {
        out[0] = *self as u8;
    }
    fn fetch(bytes: &[u8]) -> Option<Self> {
        Self::of(*bytes.first()?)
    }
}
