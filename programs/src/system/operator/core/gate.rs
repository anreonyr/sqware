//! 裁决 → 线上那一格：怎么问（Facts）、怎么翻（verdict）。
//! 把 judge 那三格答案翻成线上那一格码（Code）。判据要问的那几条边由树域那一侧

use env::TaskId;

use protocol::system::operator::{Permit, Ruling};

use super::judge::{Facts, judge};

pub const WIRE_OK: u8 = 0;
pub const WIRE_DENIED: u8 = 8;
pub const WIRE_UNJUDGED: u8 = 9;

/// 一颗线上码——裁决那一侧的全部出口
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Code {
    /// 放行（继续去动树）
    Ok,
    /// 这一位不许。**终态**
    Denied,
    /// 判不了：这一问要的那条事实问不到——对面不答 / 超时（**会好**），或那一号是碑 /
    /// 那一格是块窗格 / 开者那扇门封印了（**好不了**）。两类同格；**重试是客人的策略**
    Unjudged,
}

impl Code {
    /// 写给客人的那一格
    pub const fn wire(self) -> u8 {
        match self {
            Code::Ok => WIRE_OK,
            Code::Denied => WIRE_DENIED,
            Code::Unjudged => WIRE_UNJUDGED,
        }
    }

    /// 放行了吗——调用点只该问这一句
    pub const fn passed(self) -> bool {
        matches!(self, Code::Ok)
    }
}

/// 判一格：`facts` 是那几条边（Facts 的四问），`permit` 是那一格自己那一句话
pub fn verdict(facts: &impl Facts, who: TaskId, permit: Permit) -> Code {
    match judge(facts, who, permit) {
        Ruling::Allow => Code::Ok,
        Ruling::Deny => Code::Denied,
        Ruling::Unjudged => Code::Unjudged,
    }
}
