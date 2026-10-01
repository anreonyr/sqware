//! line::frame — **形与码**：两句话、两份形状，加一张失败域与状态码的双射表。
//! ```text
//!   登记（门牌那条路上一问一答）  [OCCUPY][线号 4B]    →  [状态码 1B]
//!   线泊位（路由者 ↔ 客户）       [记号 1B]           两个方向同一份形状
//! ```

use env::Mark;

use crate::wire::message::Message;

/// 四个原语会失败在哪一格。**一格对应一个不同的下一步**。
/// 它住本文件（与那四个状态码同一处）：**失败域与状态码是一张双射表**（见下面的
/// [`fail_codes!`](crate::fail_codes)）。账那一边（`Lines` 的四原语）按它折，客侧那一侧也按它认。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fail {
    /// 本控制器上没有这条线（没有主、或线号越出 `[1, device_count]`）⇒ 回头查树。
    Unknown,
    /// 这条线有人了 ⇒ 换个名字，或者等它 `vacate`。
    Taken,
    /// 那一帧推不出去（口封了 / 对端没了）⇒ 按"没投成"算，不置忙。
    Denied,
}

/// 登记那一句的动作码。
pub const OCCUPY: u8 = 1;

/// 线泊位的记号（两侧同一个）。
pub const LANE: &str = "line";

/// 回信孔的记号（登记那一答从它回来）。
pub const BACK_MARK: Mark = Mark::of("line-back");

const _: () = assert!(BACK_MARK.get() != Mark::of(LANE).get());
const _: () = assert!(BACK_MARK.get() != Mark::of("line-tip").get());
const _: () = assert!(BACK_MARK.get() != Mark::NONE.get());

/// 成功那一格：**全协议同一个号**——定义在 `protocol/src/wire/fail_codes.rs`（`fail_codes!` 的第二个参数就是它），
/// 本族只把它转出来。
pub use crate::wire::fail_codes::OK;

/// 状态码与失败域**同源**：一格对应一个不同的下一步。
pub const UNKNOWN: u8 = 1;
pub const TAKEN: u8 = 2;
pub const DENIED: u8 = 3;
pub const BAD: u8 = 4;

crate::fail_codes! {
    /// 失败域 → 状态码（**一处编**：客户与路由者看同一张表）。`None`（没失败）⇒ `OK`。
    bijective Fail; OK;
    Fail::Unknown => UNKNOWN,
    Fail::Taken => TAKEN,
    Fail::Denied => DENIED,
}

/// 登记那一帧：动作码 ＋ **线号**。
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Occupy {
    pub op: u8,
    pub line: u32,
}

impl Occupy {
    /// 编一句登记：动作码固定 [`OCCUPY`]，荷载是那条线的号。
    pub fn of(line: u32) -> Occupy {
        Occupy { op: OCCUPY, line }
    }
}

impl Message for Occupy {
    /// **读出来就是那个号**：动作码是形状的一部分（`fetch` 里认），读的人要的就是它。
    type In = u32;
    type Buf = [u8; Occupy::LEN];
    const EMPTY: Self::Buf = [0u8; Occupy::LEN];

    fn store(&self, out: &mut [u8]) -> Option<usize> {
        self.store_at(out, 0)
    }

    /// 拆一帧登记：**不是那个形状就答 `None`**（别人往这扇门推别的东西时，不猜）。
    /// **"恰好 `Occupy::LEN`"**：表那一手只要求"够长"，而这一形的判据是**恰好**——长短都不认。
    fn fetch(bytes: &[u8]) -> Option<u32> {
        if bytes.len() != Occupy::LEN {
            return None;
        }
        let occupy = Occupy::fetch(bytes)?;
        (occupy.op == OCCUPY).then_some(occupy.line)
    }
}
