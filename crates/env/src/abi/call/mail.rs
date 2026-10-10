//! call::mail — **Mail 域（class 5，数据轴：消息穿孔）**：调用表（[`MailCall`]）与失败词汇（[`MailFail`]）。

use crate::abi::wait::Wait;
use crate::wire::{PieToken, TaskId, VirtAddr};
use crate::UnitTarget;
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
    #[slot(32)]
    #[ret(())]
    Push {
        token: PieToken,
        msg: VirtAddr,
        len: usize,
    },
    /// Discard applies only to an oversized head, never to address or permission failures.
    #[slot(33)]
    #[ret3(PullOutcome)]
    Pull {
        token: PieToken,
        buf: VirtAddr,
        max: usize,
        oversize: Oversize,
    },
    /// Pull and Signal need FETCH; Push and Empty need STORE.
    #[slot(34)]
    #[ret(bool)]
    Wait {
        token: PieToken,
        condition: MailCondition,
        millis: Wait,
    },
    /// Pole clears one bit idempotently. Hole acknowledges one counted event;
    /// Hole/Nole accept FIRST only and preserve their Busy acknowledgement contract.
    #[slot(35)]
    #[ret(())]
    Hush { token: PieToken, bit: Bit },
    /// Pole sets one bit idempotently. Hole accumulates an event, up to its bound.
    /// Hole/Nole accept FIRST only and preserve their Busy contract.
    #[slot(36)]
    #[ret(())]
    Ring { token: PieToken, bit: Bit },
    #[slot(37)]
    #[ret3((usize, TaskId, usize))]
    Peek { token: PieToken },
    #[slot(38)]
    #[ret(())]
    Attach { tole: PieToken, source: Source },
    #[slot(39)]
    #[ret(())]
    Detach { tole: PieToken, source: Source },
    #[slot(40)]
    #[ret3(AwaitReply)]
    Await { tole: PieToken, millis: Wait },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Bit(u8);
impl Bit {
    pub const FIRST: Self = Self(0);
    pub const fn of(index: usize) -> Option<Self> {
        if index < usize::BITS as usize { Some(Self(index as u8)) } else { None }
    }
    pub const fn index(self) -> usize { self.0 as usize }
    pub const fn mask(self) -> usize { 1usize << self.0 }
}
impl crate::Wire for Bit {
    fn pack(&self, s: &mut [usize; 6], i: &mut usize) { self.index().pack(s, i); }
    fn unpack(s: &[usize; 6], i: &mut usize) -> Result<Self, crate::Decode> {
        Self::of(usize::unpack(s, i)?).ok_or(crate::Decode::Invalid)
    }
}

/// A precise resource condition, Unit observation, or capability reference.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Mail { pie: PieToken, condition: MailCondition },
    Join { target: UnitTarget },
    Inspect { task: TaskId, token: PieToken },
}
impl Source {
    pub fn words(self) -> [usize; 3] {
        match self {
            Self::Mail { pie, condition } => [1, pie.get(), condition.wire()],
            Self::Join { target: UnitTarget::Task(task) } => [2, task.get(), 0],
            Self::Join { target: UnitTarget::Team(team) } => [3, team.get(), 0],
            Self::Inspect { task, token } => [4, task.get(), token.get()],
        }
    }
    pub fn of(words: [usize; 3]) -> Option<Self> {
        match words {
            [1, pie, condition] if pie != 0 => Some(Self::Mail { pie: PieToken::new(pie), condition: MailCondition::of(condition)? }),
            [2, task, 0] => Some(Self::Join { target: UnitTarget::Task(TaskId::new(task)) }),
            [3, team, 0] => Some(Self::Join { target: UnitTarget::Team(crate::TeamId::new(team)) }),
            [4, task, token] if token != 0 => Some(Self::Inspect { task: TaskId::new(task), token: PieToken::new(token) }),
            _ => None,
        }
    }
}
impl crate::Wire for Source {
    fn pack(&self, s: &mut [usize; 6], i: &mut usize) { for word in self.words() { word.pack(s, i); } }
    fn unpack(s: &[usize; 6], i: &mut usize) -> Result<Self, crate::Decode> {
        let mut words = [0; 3]; for word in &mut words { *word = usize::unpack(s, i)?; }
        Self::of(words).ok_or(crate::Decode::Invalid)
    }
}

/// Pending means POLL had no source or the original deadline expired.
/// Source-level failures preserve the source; group failures are MailResult errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AwaitReply {
    Pending,
    Source { source: Source, fail: Option<MailFail> },
}
impl AwaitReply {
    pub fn is_none(self) -> bool { self == Self::Pending }
    pub fn mail(self) -> Option<(PieToken, MailCondition)> {
        match self { Self::Source { source: Source::Mail { pie, condition }, .. } => Some((pie, condition)), _ => None }
    }

    pub fn words(self) -> [usize; 3] {
        match self {
            Self::Pending => [0; 3],
            Self::Source { source, fail } => {
                let mut words = source.words();
                // Low byte is the source kind; high byte is the existing failure code magnitude.
                let code = fail.map_or(0, |e| (-crate::FailCode::code(e)) as usize);
                words[0] |= code << 8; words
            }
        }
    }
    pub fn of(mut words: [usize; 3]) -> Option<Self> {
        if words == [0; 3] { return Some(Self::Pending); }
        let code = words[0] >> 8; words[0] &= 255;
        let source = Source::of(words)?;
        let fail = match code {
            0 => None, 1 => Some(MailFail::Denied), 2 => Some(MailFail::Dead),
            3 => Some(MailFail::Busy), 4 => Some(MailFail::OoM),
            5 => Some(MailFail::HandedOver), 6 => Some(MailFail::Gone), _ => return None,
        };
        Some(Self::Source { source, fail })
    }
}
impl crate::wire::FromTriple for AwaitReply {
    fn from_triple(a: usize, b: usize, c: usize) -> Self {
        Self::of([a, b, c]).expect("invalid Await reply")
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
