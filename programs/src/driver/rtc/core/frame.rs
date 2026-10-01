//! 两句话、两种答形（一张字段表就是一处定义）。
//! **四张表、两个方向**：Now / Arm 是问的两形，Time / Status 是答的两形——偏移与
//! 长度全部由字段宽度求和得出（`#[derive(env::Frame)]` 那一处定义），手写的那五枚自由函数
//! （`pack_ask` / `pack_arm` / `unpack_ask` / `pack_time` / `unpack_time`）与那四个长度常量

use env::{Mark, PieToken};
use protocol::wire::message::Message;

/// 问那一句的动作码：「现在几点」
pub const ASK: u8 = 1;

/// 问那一句的动作码：「在 `at` 叫我」
pub const ARM: u8 = 2;

/// 回信孔的记号：客人每趟铸一枚、借给驱动（**收方按它验那一格**）
pub const BACK: Mark = Mark::of("rtc-back");

/// 失败域与答话那一格**一处编**：三个码与两向读法由 protocol::WireCodes 从
pub use super::fail::{BAD, PAST, TAKEN, code_to_fail, fail_to_code};
/// 答话那一格：收下了——**全协议那一个"没失败"**（protocol::OK），本族不再写第二遍
pub use protocol::OK;

/// **问那一形 · 「现在几点」**：动作码 ＋ 那一格
/// 动作码由 Now::of 钉进来（表那一格是裸字节，是构造那一手保证的）
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Now {
    pub op: u8,
    pub back: PieToken,
}

/// **问那一形 · 「再过多 long 叫我」**：动作码 ＋ 那一格 ＋ **一个相对量**
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Arm {
    pub op: u8,
    pub back: PieToken,
    pub after_ns: u64,
}

impl Now {
    /// 编一问：`back` = "我借给你的那枚回信孔**在你表里**是几号"
    pub fn of(back: PieToken) -> Now {
        Now { op: ASK, back }
    }
}

impl Arm {
    /// 编一问：`after_ns` 是**相对量**（纳秒）——绝对时刻由收帧的人算（见 Wire::Arm）
    pub fn of(back: PieToken, after_ns: u64) -> Arm {
        Arm {
            op: ARM,
            back,
            after_ns,
        }
    }
}

/// **解出来的一问**——与板 / 树 / 名册 / 盟籍四族同一个名字同一个位置（"解出来的一问"叫 `Wire`）
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Wire {
    /// 「现在几点」
    Now,
    /// 「**再过多 long** 叫我」（**相对量**，纳秒）
    Arm { after_ns: u64 },
}

impl Wire {
    /// 解一问：返 `(那一格, 那一问)`。**不是那个形状就答 `None`**（别人往这扇门推别的东西时
    /// 不猜、不动账、也不回话——那一格读得出来也不答，因为没有可信的"往哪回"可言：动作码不认
    /// 就问不出这一帧该有多长）
    pub fn take(bytes: &[u8]) -> Option<(PieToken, Wire)> {
        match *bytes.first()? {
            ASK if bytes.len() == Now::LEN => {
                let ask = Now::fetch(bytes)?;
                Some((ask.back, Wire::Now))
            }
            ARM if bytes.len() == Arm::LEN => {
                let ask = Arm::fetch(bytes)?;
                Some((
                    ask.back,
                    Wire::Arm {
                        after_ns: ask.after_ns,
                    },
                ))
            }
            _ => None,
        }
    }
}

/// **答那一形 · 一个时刻**：驱动读设备那一刻的纳秒计数（u64 LE）
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Time {
    pub ns: u64,
}

/// **答那一形 · 一个答码**：收下了没有（OK / TAKEN / PAST / BAD）
/// 与板 / 树那两族的 1 字节答**同名同位**（`Status`）：一格状态、没有荷载
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Status {
    pub status: u8,
}

impl Time {
    /// 编一答：一个时刻
    pub const fn of(ns: u64) -> Time {
        Time { ns }
    }
}

impl Status {
    /// 编一答：一个答码
    pub const fn of(code: u8) -> Status {
        Status { status: code }
    }
}

impl Message for Time {
    /// 解开之后就是**那个时刻**——读的人不必再念一遍"它叫 `ns`"
    type In = u64;
    /// 定长一答（Time::LEN）
    type Buf = [u8; Time::LEN];
    const EMPTY: Self::Buf = [0u8; Time::LEN];

    /// 要的是定长数组、返 `()`，两回事
    fn store(&self, out: &mut [u8]) -> Option<usize> {
        Time::store_at(self, out, 0)
    }

    fn fetch(bytes: &[u8]) -> Option<u64> {
        if bytes.len() != Time::LEN {
            return None;
        }
        Some(Time::fetch(bytes)?.ns)
    }
}

impl Message for Status {
    /// 解开之后就是**那一格答码**
    type In = u8;
    type Buf = [u8; Status::LEN];
    const EMPTY: Self::Buf = [0u8; Status::LEN];

    fn store(&self, out: &mut [u8]) -> Option<usize> {
        Status::store_at(self, out, 0)
    }

    /// **恰好 1 字节**
    fn fetch(bytes: &[u8]) -> Option<u8> {
        if bytes.len() != Status::LEN {
            return None;
        }
        Some(Status::fetch(bytes)?.status)
    }
}
