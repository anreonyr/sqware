//! call::mail — **Mail 域（class 5，数据轴：消息穿孔）**：调用表（[`MailCall`]）与失败词汇（[`MailFail`]）。

use crate::abi::wait::Wait;
use crate::wire::{PieToken, TaskId, VirtAddr};
use mold::{Envcall, Fail};

/// Mail 域（class 5：数据轴）的失败词汇。
#[derive(Fail)]
pub enum MailFail {
    /// token 不在表里 / 权不够 / 不是孔（递了铃、页、组）。
    Denied = -1,
    /// 那一枚已封印。
    Dead = -2,
    /// 条件未就绪（孔上已有手／正被取用／位已响／没有可取之事／铃已响）。
    #[busy]
    Busy = -3,
    /// 表项备不下。
    OoM = -4,
    /// 这一枚已交出去（交回即复原）。
    HandedOver = -5,
    /// 递出那只手所依的内存已经没了：发送方那段不可读，或它那个空间已回收。
    ///
    /// 与 `Denied` 分开：`Denied` 是"换够大的缓冲再来"，`Gone` 是"这条路废了"。
    Gone = -6,
}

/// `MailFail` 的结果别名。
pub type MailResult<T> = Result<T, erra::Error<MailFail>>;

/// Message transfer and resource readiness.
#[derive(Envcall)]
#[call(class = 5, fail = MailFail)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MailCall {
    #[slot(0)]
    #[ret(())]
    Push {
        token: PieToken,
        msg: VirtAddr,
        len: usize,
    },
    /// Discard applies only to an oversized head, never to address or permission failures.
    #[slot(1)]
    #[ret3(PullOutcome)]
    Pull {
        token: PieToken,
        buf: VirtAddr,
        max: usize,
        oversize: Oversize,
    },
    /// Pull needs FETCH; Push and Empty need STORE. Nole/Pole support Pull only.
    #[slot(2)]
    #[ret(bool)]
    Wait {
        token: PieToken,
        condition: MailCondition,
        millis: Wait,
    },
    #[slot(3)]
    #[ret(())]
    Hush { token: PieToken },
    #[slot(4)]
    #[ret(())]
    Ring { token: PieToken },
    #[slot(5)]
    #[ret3((usize, TaskId, usize))]
    Peek { token: PieToken },
}

/// Readiness is a hint; another caller can change the queue before the next operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MailCondition {
    Pull,
    Push,
    Empty,
}
impl MailCondition {
    pub fn wire(self) -> usize {
        self as usize
    }
    pub fn of(raw: usize) -> Option<Self> {
        match raw {
            0 => Some(Self::Pull),
            1 => Some(Self::Push),
            2 => Some(Self::Empty),
            _ => None,
        }
    }
}
impl crate::Wire for MailCondition {
    fn pack(&self, s: &mut [usize; 6], i: &mut usize) {
        self.wire().pack(s, i);
    }
    fn unpack(s: &[usize; 6], i: &mut usize) -> Result<Self, crate::Decode> {
        Self::of(usize::unpack(s, i)?).ok_or(crate::Decode::Invalid)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Oversize {
    Keep,
    Discard,
}
impl crate::Wire for Oversize {
    fn pack(&self, s: &mut [usize; 6], i: &mut usize) {
        (*self as usize).pack(s, i);
    }
    fn unpack(s: &[usize; 6], i: &mut usize) -> Result<Self, crate::Decode> {
        match usize::unpack(s, i)? {
            0 => Ok(Self::Keep),
            1 => Ok(Self::Discard),
            _ => Err(crate::Decode::Invalid),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PullOutcome {
    Received { len: usize, sender: TaskId },
    Discarded { len: usize, sender: TaskId },
}
impl crate::wire::FromTriple for PullOutcome {
    fn from_triple(len: usize, sender: usize, tag: usize) -> Self {
        let sender = TaskId::new(sender);
        match tag {
            0 => Self::Received { len, sender },
            1 => Self::Discarded { len, sender },
            _ => unreachable!("invalid Pull result"),
        }
    }
}
