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
    /// Pull and Signal need FETCH; Push and Empty need STORE.
    #[slot(2)]
    #[ret(bool)]
    Wait {
        token: PieToken,
        condition: MailCondition,
        millis: Wait,
    },
    #[slot(3)]
    #[ret(())]
    Hush { token: PieToken, bits: Bits },
    #[slot(4)]
    #[ret(())]
    Ring { token: PieToken, bits: Bits },
    #[slot(5)]
    #[ret3((usize, TaskId, usize))]
    Peek { token: PieToken },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Bit(u8);
impl Bit {
    pub const FIRST: Self = Self(0);
    pub const fn of(index: usize) -> Option<Self> {
        if index < usize::BITS as usize { Some(Self(index as u8)) } else { None }
    }
    pub const fn index(self) -> usize { self.0 as usize }
    pub const fn bits(self) -> Bits { Bits(1usize << self.0) }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bits(usize);
impl Bits {
    pub const FIRST: Self = Self(1);
    pub const fn of(bits: usize) -> Option<Self> { if bits == 0 { None } else { Some(Self(bits)) } }
    pub const fn get(self) -> usize { self.0 }
    pub fn iter(self) -> impl Iterator<Item = Bit> {
        (0..usize::BITS as usize).filter(move |index| self.0 & (1usize << index) != 0).map(|index| Bit(index as u8))
    }
}
impl crate::Wire for Bits {
    fn pack(&self, s: &mut [usize; 6], i: &mut usize) { self.get().pack(s, i); }
    fn unpack(s: &[usize; 6], i: &mut usize) -> Result<Self, crate::Decode> {
        Self::of(usize::unpack(s, i)?).ok_or(crate::Decode::Invalid)
    }
}

/// Readiness is a hint; another caller can change the queue before the next operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MailCondition {
    Pull,
    Push,
    Empty,
    Signal(Bit),
}
impl MailCondition {
    pub fn wire(self) -> usize {
        match self { Self::Pull => 0, Self::Push => 1, Self::Empty => 2, Self::Signal(bit) => 3 + bit.index() }
    }
    pub fn of(raw: usize) -> Option<Self> {
        match raw {
            0 => Some(Self::Pull),
            1 => Some(Self::Push),
            2 => Some(Self::Empty),
            _ => Bit::of(raw.checked_sub(3)?).map(Self::Signal),
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
