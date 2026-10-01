//! hub — **设备账那一族**：这一台机器上有哪些设备、谁在驱它们，以及"我要驱这一类"这一问。
//! ```text
//!   hub
//!   ├── NAME   hub —— 这一族在树上的那一段（/svc/hub）
//!   ├── frame  三条原语的形与码（bond / list / claim）＋ 一张四格失败表
//!   ├── grant  三枚面（一原语一面）＋ 它们的记号
//!   └── client 客侧三手（报名 / 列册 / 认领）
//! ```
//! # 它办哪三件事
//! ```text
//!   bond   报名   驱动 → hub：「许我驱这一类」         ⇒ hub 把这位放进那一类的盟（盟册 admit）
//!   list   列册   谁 → hub：这一类里有什么             ⇒ 名字 ＋ 有主那一位掩码
//!   claim  认领   **在那一台自己那一份孔上**：「这台归我」 ⇒ 一张契（Deed）
//! ```
//! **`bond` 那一格为什么存在**：准用那一轴写在**树**上（`/dev/<类>/<名>` 的 `permit = Among(c_类)`），
//! 而"许给这一类的盟"要先有人在那枚盟里。盟册的 `enter` 钥匙是**发送者那一格**——驱动自己入不了
//! 别人的名，故由 hub 代报名。
//! **驱动不需要知道盟号**：它只说"我要驱这一类"。
//! # `claim` 那一格为什么在设备那一格上

pub mod client;
pub mod frame;
pub mod grant;

pub use client::Face;
pub use frame::{
    ALIVE_MARK, BACK_MARK, BAD, BOND, BOOT, CLAIM, DEAD, DENIED, DEV_ROAD, DTB, Deed, ENROLL_CAP,
    ENROLL_MAX, Enroll, Fail, IRQ, LIST, LIST_MAX, OK, TAKEN, UNKNOWN, Window,
};
pub use grant::{Grant, grant_of};

/// 这一族在树上的那一段名字：**`hub`**（`/svc/hub`）。
/// 三枚面挂在它下面（`/svc/hub/{bond,list,claim}`）；设备那一轴是 `/dev/<类>/<名>`，不是这一族
/// 的门牌（那是 hub 落的账）。
pub const NAME: &str = "hub";
