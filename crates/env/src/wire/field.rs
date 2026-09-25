//! **按字节缓冲编解码**：过线那一格自己是多宽、怎么写进字节、怎么读回来。
//!
//! 与 [`Wire`](super::Wire) 的分工：`Wire` 是**六寄存器 ABI** 那一层（`[usize; 6]`），
//! 这一层是**报文帧**那一层（`&[u8]` / `&mut [u8]`）。两层都在 `env`：过线的那些东西住一处。
//!
//! **`fetch` 的 `None` 说的是"这一帧读不懂"**，不是"这个字段的值不合规矩"——值那一层各有各的
//! 失败域（如 [`NameError`](crate::wire::NameError)），故这里只答 `Option`。
//!
//! 本文件还有 `frame!` 宏（**定长帧**那一族的一处定义）：帧的偏移全部由 [`Field::WIDTH`]
//! 求和得出，从而两头不可能各写一份。

use crate::wire::{Name, PieToken, TaskId, NAME_LEN};

/// **过线的一格**：定宽 ＋ 会写会读。
pub trait Field: Sized {
    /// 线上占几字节（**定长**——帧的偏移全部由它求和得出）。
    const WIDTH: usize;
    /// 写进 `out`（长度恰是 [`Field::WIDTH`]）。
    fn store(&self, out: &mut [u8]);
    /// 从 `bytes` 读回来；**长度不足或那一格读不成** ⇒ `None`（不猜、不崩）。
    fn fetch(bytes: &[u8]) -> Option<Self>;
}

impl Field for TaskId {
    const WIDTH: usize = 8;
    fn store(&self, out: &mut [u8]) {
        out.copy_from_slice(&(self.get() as u64).to_le_bytes());
    }
    fn fetch(bytes: &[u8]) -> Option<Self> {
        let raw: [u8; 8] = bytes.get(..8)?.try_into().ok()?;
        Some(TaskId::new(u64::from_le_bytes(raw) as usize))
    }
}

impl Field for PieToken {
    const WIDTH: usize = 8;
    fn store(&self, out: &mut [u8]) {
        out.copy_from_slice(&self.to_bytes());
    }
    fn fetch(bytes: &[u8]) -> Option<Self> {
        PieToken::from_bytes(bytes.get(..8)?)
    }
}

impl Field for Name {
    /// **定长、带填充**：`Name::bytes()` 是那 32 字节的整个数组（内容之后的填充也上线）。
    /// 帧的偏移要的是"这一格占多宽"，故取 `NAME_LEN`，不是内容的长度。
    const WIDTH: usize = NAME_LEN;
    fn store(&self, out: &mut [u8]) {
        out.copy_from_slice(self.bytes());
    }
    fn fetch(bytes: &[u8]) -> Option<Self> {
        Name::from_bytes(bytes.get(..NAME_LEN)?.try_into().ok()?).ok()
    }
}

// ── `frame!`：定长帧的一处定义 ──────────────────────────────

/// **定长帧**那一族的一处定义：给一张字段表，生成结构体 ＋ 长度 ＋ 一对 `store` / `fetch`。
///
/// ```ignore
/// env::frame! {
///     /// 板那条提示帧：号 ＋ 定长名字 ＋ 答话路那一格。
///     pub struct Tip {
///         who: TaskId,
///         name: Name,
///         reply: PieToken,
///     }
/// }
/// ```
///
/// 生成的东西**一眼看得完**（没有隐藏机制）：`pub struct` ＋ 公开字段、`pub const LEN`
/// （**字段宽度之和**）、`store(&self, &mut [u8; LEN])`、`fetch(&[u8]) -> Option<Self>`。
///
/// **偏移一处都不写**——两半由**同一张字段表**生成，故"同一条长度写两处、改一处漏一处
/// **编得过**"那个病**写不出来**（`COORD_FRAME` 那一格栽的正是它：那边的照实记写着"靠注释说
/// 必须同值"）。
///
/// **它只管定长字段序列**：变长（`Ask::Road` 那样一条路几段不定）与重复（计数 ＋ 一段数组）
/// 那两类**不归它**，那几族本来就各有各的一对 `pack` / `unpack`。
///
/// **字段的字节编解码归 [`env::wire::Field`]**（`WIDTH` / `store` / `fetch`）：宏只负责
/// "顺序与偏移"，一格自己是多宽、怎么写，是那一格自己的事。
#[macro_export]
macro_rules! frame {
    (
        $(#[$meta:meta])*
        $vis:vis struct $name:ident {
            $($field:ident : $ty:ty),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        $vis struct $name {
            $(pub $field: $ty),+
        }

        impl $name {
            /// 这一帧线上占几字节：**字段宽度之和**（一处定义）。
            pub const LEN: usize = 0 $(+ <$ty as $crate::wire::Field>::WIDTH)+;

            /// 写进 `out`。
            pub fn store(&self, out: &mut [u8; Self::LEN]) {
                let mut at = 0usize;
                $(
                    <$ty as $crate::wire::Field>::store(
                        &self.$field,
                        &mut out[at..at + <$ty as $crate::wire::Field>::WIDTH],
                    );
                    at += <$ty as $crate::wire::Field>::WIDTH;
                )+
                let _ = at;
            }

            /// 从 `bytes` 读回来；**长度不足** ⇒ `None`（不猜、不崩）。
            pub fn fetch(bytes: &[u8]) -> Option<Self> {
                let mut at = 0usize;
                $(
                    let $field = <$ty as $crate::wire::Field>::fetch(
                        bytes.get(at..at + <$ty as $crate::wire::Field>::WIDTH)?,
                    )?;
                    at += <$ty as $crate::wire::Field>::WIDTH;
                )+
                let _ = at;
                Some(Self { $($field),+ })
            }
        }
    };
}


