//! line::frame — **形与码**：两句话、两份形状，加一张失败域与状态码的双射表。
//!
//! ```text
//!   登记（门牌那条路上一问一答）  [OCCUPY][坐标 16B]   →  [状态码 1B]
//!   线泊位（路由者 ↔ 客户）       [记号 1B]           两个方向同一份形状
//! ```
//!
//! **照实记（`Fail` 并进来了）**：失败域原先单独住 `line::core`，而那一份与本文件**只差一张
//! 双射表**（[`fail_codes!`](crate::fail_codes) 就在下面，两侧看同一张）。`line::core` 的另
//! 一半（按线号索引的账 `Lines`）已搬回它的持有者那里（`programs/src/driver/router/core/lines.rs`）
//! ⇒ 那一份文件没有了，失败域随之并到这里：**形与码一处**。
//!
//! 投递与排空是**同一条路的两个方向**，故不带动作码、也不带线号——方向就是那一格，而**泊位
//! 就是坐标**（客户手里没有"线"，见 `super::mod`）。那 1 字节不是信息，是**形状的下限**：
//! 孔不收 0 字节的报文。登记那一句带一个动作码（`OCCUPY`）——**照实记**：它原来是为了与"招呼"
//! 那一形状分开（同一扇门、靠帧长分），那条形状已经退休。**留着**（用户裁定）：这一格不是死
//! 字段——[`Message::fetch`] 真的读它（形状不对就答 `None`，路由器不动账），而且本仓**三扇门
//! 都带动作码**（板那一扇的 `REGISTER` 一族、树那一扇的 `land`/`find` 一族）——去掉只省
//! 1 字节，换来"任何恰好 17 字节推上来的东西都算一次登记"。

use env::Mark;
use env::Key;

use crate::message::Message;

/// 四个原语会失败在哪一格。**一格对应一个不同的下一步**。
///
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

// ── 面不相撞（**编译期**钉住——用户裁定"常量交给编译器"）────────────────────
//
// 原先这是宿主台的一条运行时用例（`the_two_marks_of_this_road_do_not_collide`）。
const _: () = assert!(BACK_MARK.get() != Mark::of(LANE).get());
const _: () = assert!(BACK_MARK.get() != Mark::of("line-tip").get());
const _: () = assert!(BACK_MARK.get() != Mark::NONE.get());

/// 线泊位两个方向那一个记号：**帧不报内容，只报"有事"**（形状的下限，见文件头）。
pub const NOTE: u8 = 1;

/// 成功那一格：**全协议同一个号**——定义在 `protocol/src/fail_codes.rs`（`fail_codes!` 的第二个参数就是它），
/// 本族只把它转出来。
pub use crate::fail_codes::OK;

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

/// 登记那一帧：动作码 ＋ 坐标。
///
/// **照实记（这一份从前是什么样）**：它从前是一对**自由函数**（`pack_occupy` /
/// `unpack_occupy`）＋ 一处手算的长度（`OCCUPY_LEN = 1 + KEY_LEN`）。今天收成报那一层那两样：
/// **一张表**（`#[derive(env::Frame)]` 求长）＋ **一个 `impl Message`**（编解一处）。
///
/// **`op` 那一格留着**（用户裁定，见文件头）：[`Message::fetch`] 真的读它——形状不对就答
/// `None`，路由器不动账。
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Occupy {
    pub op: u8,
    pub key: Key,
}

impl Occupy {
    /// 编一句登记：动作码固定 [`OCCUPY`]，荷载是那一段区。
    pub fn of(key: Key) -> Occupy {
        Occupy { op: OCCUPY, key }
    }
}

impl Message for Occupy {
    /// **读出来就是那个坐标**：动作码是形状的一部分（`fetch` 里认），读的人要的就是它
    /// （从前 `unpack_occupy` 答的也是它）。
    type In = Key;
    type Buf = [u8; Occupy::LEN];
    const EMPTY: Self::Buf = [0u8; Occupy::LEN];

    fn store(&self, out: &mut [u8]) -> Option<usize> {
        self.store_in(out)
    }

    /// 拆一帧登记：**不是那个形状就答 `None`**（别人往这扇门推别的东西时，不猜）。
    ///
    /// **照实记（"恰好 17"那一格）**：表那一手只要求"够长"，而这一形的判据是**恰好**
    /// `Occupy::LEN`——长短都不认，故这里补回这一格（同从前 `unpack_occupy` 的第一句）。
    ///
    /// 坐标那一格的判别号不认识也答 `None`（[`Key`] 那一手判废）；**认识、但不是"区"的那两形
    /// 照收**——路由者按坐标查表查不到，自然答 `UNKNOWN`（线挂在设备上，那是它的账）。
    fn fetch(bytes: &[u8]) -> Option<Key> {
        if bytes.len() != Occupy::LEN {
            return None;
        }
        let occupy = Occupy::fetch(bytes)?;
        (occupy.op == OCCUPY).then_some(occupy.key)
    }
}
