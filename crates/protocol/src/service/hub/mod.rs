//! 这一台机器上有哪些设备、谁在驱它们，以及"我要驱这一类"这一问。
//! # 它办哪三件事
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

/// 的门牌（那是 hub 落的账）
pub const NAME: &str = "hub";
