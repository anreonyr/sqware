//! pie_kind — 门闩那一类的**判别号**（[`PieKind`]）。
//!
//! # 为什么它住 `env`
//!
//! 装配那一侧要**两侧读**——内核的 `build.rs`（宿主）与编排域（riscv）——而 `programs` /
//! `protocol` 都拖着 `runtime`（riscv 内联汇编，宿主上编不过）。故凡是"装配那一侧要摆出来的
//! 东西"，定义都得住 `env`。

const KIND_POLE: u8 = PieKind::Pole as u8;
const KIND_NOLE: u8 = PieKind::Nole as u8;

/// **要的是哪一种门闩**（**判别号即线格式**：`repr(u8)`）。
///
/// 两格对应内核那两种门闩句柄（`PolePie` / `NolePie`）——发货那一侧据此挑对那一层。
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PieKind {
    /// 一段内存（设备寄存器页 / 自描述区 / 载荷区）。
    Pole,
    /// 空载荷的信号（中断门铃）。
    Nole,
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
            _ => None,
        }
    }
}
