//! 控制台那一面的**形、界、名** —— 纯约定，一处定义。
//!
//! 三件东西：
//! - **形**：[`Bytes`] —— 这一路上过边界的就是"一段**非空**的字节"（长度即内容）。
//! - **界**：[`DRAIN_MAX`]（一次排空最多搬走多少）与 [`MAX`]（单条传输的上界）。
//! - **名**：树上的三个（[`ME`] ＋ [`RX`]/[`TX`]）——驱动落砖与客人找砖念的是同一批，
//!   故只能有一处；两处写就会在两处错开。**只有两枚门牌**：每个方向一枚，而**那一枚后面
//!   就是一具完整的架**（页上那一位即它的铃，见 `ipc::rack`）——
//!   页与铃不各占一格。
//!
//! **为什么住 `core/`**：这一层不碰内核、不碰设备（`driver/uart/mod.rs` 那条分界），
//! 只有"这一批能不能交"这一条纪律——与 rtc 的 `core/frame.rs` 同位（那里也是形与记号住 core）。
//! **两端念的是同一枚类型**：驱动与 terminal 是**两个 bin**，而本模块由 lib 出 ⇒ `Rack<Bytes>`
//! 在两边说的是同一个 `M`。"各写一份"编不过，那正是要的。
//!
//! **非空**这一条从 `Batch` 搬到这里（`Batch` 那一格随共享内存那一刀一起撤）：它本来就是
//! 类型的纪律，不是某一处的检查。

use env::wire::{fetch_bytes, store_bytes};
use wire::Message;

/// 一次排空最多搬走多少字节。设备 FIFO 只有 16 字节，取四倍宽；满了剩下的还在设备里。
pub const DRAIN_MAX: usize = 64;

/// 单条传输的字节上限。终端的回显按此分块，与输入模式的行长无关。
pub const MAX: usize = 129;

/// 编译期：一次排空那一批也塞得进一条字（改 `DRAIN_MAX` 时不必去别处对账）。
const _: () = assert!(DRAIN_MAX <= MAX);

/// 本域挂在板上的名字（板按它分人；编排域表里那一条也叫这个）。
///
/// **它住这里**（原来住 `main.rs`）：树上的路（`/svc/drv/<这个名字>`）由驱动的住持面与
/// 客人面**两头**念，只能有一处。
pub const ME: &str = "uart";

/// **读口**那一枚门牌：对面（驱动）排空出来的一批，本域取。
///
/// **它后面是一具完整的架**（那一枚页上带着"有事"那一位）——页与铃不各占一格。
pub const RX: &str = "rx";

/// **写口**那一枚门牌：本域推"一条完整的字"，对面写进设备。同上：后面是一具架。
pub const TX: &str = "tx";

/// 一段**非空**的字节 —— 这一路上过边界的那一条。
///
/// **一条的形是"长度即内容"**（[`store_bytes`] / [`fetch_bytes`] 那一形）：没有条数、没有终止符，
/// 故 `store` 返的就是它自己那么长；`fetch` 拿到"这一格剩下的全部字节"。
///
/// **空不是这一族的值**：`of(&[])` 与 `fetch` 到空都答 `None`——"交 0 字节"因此写不出来
/// （与 `Batch::of` 那条纪律同一件事，只是改由这一枚承担：驱动那一圈不必再先判一次）。
pub struct Bytes {
    bytes: [u8; MAX],
    len: usize,
}

impl Bytes {
    /// 收下一段非空字节：空或超过 [`MAX`] ⇒ `None`（不截断、不猜）。
    pub fn of(src: &[u8]) -> Option<Bytes> {
        if src.is_empty() || src.len() > MAX {
            return None;
        }
        let mut bytes = [0u8; MAX];
        bytes[..src.len()].copy_from_slice(src);
        Some(Bytes {
            bytes,
            len: src.len(),
        })
    }

    /// 这一条的内容（非空由构造保证）。
    pub fn bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

impl Message for Bytes {
    /// **发与收是同一形**：这一段字节就是过边界的全部（没有别格要拆要装）。
    type In = Bytes;
    type Buf = [u8; MAX];
    const EMPTY: Self::Buf = [0u8; MAX];

    fn store(&self, out: &mut [u8]) -> Option<usize> {
        store_bytes(out, 0, self.bytes())
    }

    /// **空 ⇒ `None`**（与 [`Bytes::of`] 同一条纪律：这一族的元素非空）。
    fn fetch(bytes: &[u8]) -> Option<Bytes> {
        Bytes::of(fetch_bytes(bytes, 0)?)
    }
}
