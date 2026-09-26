//! eyes — **持树者的哪一双眼睛**：协调那一帧（`protocol::system::operator::frame` 的
//! `CoordFrame`）后 8 字节用的判别号。
//!
//! **它为什么住 `env`**（照实记）：这一格的读法两侧都要——装配侧按 [`Eyes::of_wire`] 现算，
//! 收的那一侧（`protocol::system::operator`）按 `from_le_bytes` 现翻；而**装配声明**
//! （`programs` 的 `Program.eyes`）是**宿主安全**模块的一格，`crates/image` 也要编得到它。
//! `protocol` 拖着 `runtime`（那两处 riscv 内联汇编在宿主上编不过）⇒ 宿主够得着的最下层
//! 只有本 crate。故它从原先那张程序声明表里落在这里（与 [`Key`](crate::Key) 同一条理由）。
//!
//! **它是装配声明上的一格，不是靠名字认的**：旧法写 `p.name == "principal"`——把那一行改个
//! 名，认它的那一侧就**静默失灵**（门禁从此判不了身份）。故它是 `Program` 上明写的一格。
//!
//! `of_wire` 的 `None`（表外）正对着 `fetch` 的 `None`（这一帧读不懂）——收的那一侧据此报
//! 一句、不静默（见 `programs/src/system/operator/server.rs`）。

use crate::wire::Field;

/// **这一台是持树者的哪一双眼睛**。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Eyes {
    /// 名册（`/sys/principal`）：答"这一位此刻代表谁"与"在不在他那一支里"。
    Roster = 0,
    /// 盟册（`/sys/coalition`）：答"这一位在那枚盟里吗"。
    League = 1,
}

impl Eyes {
    /// 线上那一格 → 这一枚（`None` = 表外，读不懂）。两侧共用同一份定义，故不必各写一遍常量。
    pub const fn of_wire(raw: u64) -> Option<Eyes> {
        match raw {
            x if x == Eyes::Roster as u64 => Some(Eyes::Roster),
            x if x == Eyes::League as u64 => Some(Eyes::League),
            _ => None,
        }
    }
}

/// **这一枚在帧里占的那 8 字节**：判别值小端、高位留零。
///
/// **impl 跟着类型走**（本仓口径）：`of_wire` 的校验就是这一格的校验，故 `fetch` 直取它；
/// 类型住本文件，impl 也住本文件。
impl Field for Eyes {
    const WIDTH: usize = 8;

    fn store(&self, out: &mut [u8]) {
        out.copy_from_slice(&(*self as u64).to_le_bytes());
    }

    fn fetch(bytes: &[u8]) -> Option<Self> {
        let raw: [u8; 8] = bytes.get(..8)?.try_into().ok()?;
        Eyes::of_wire(u64::from_le_bytes(raw))
    }
}
