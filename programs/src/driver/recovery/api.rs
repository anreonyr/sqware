//! 设备恢复那一台的协议：在发布前取得精确 BAR 恢复映射（`attach`）、
//! 业务停执行后执行本设备 reset/drain 并回读（`stop`）、停链完成后放下映射（`detach`）。
//!
//! 它只做**本设备**的恢复操作：不解释 IOMMU、不碰 ECAM、不构造任务、也不保存总设备账。
//! 报文里不接收可信 caller：发送者由内核给出；BAR 窗按 `vestor` 与本次绑定的记号核。

use env::wire::Span as _;
use env::{PieToken, TaskId};
use wire::{Message, OK};

/// 本台那一扇门在树上的坐标。
pub const ENTRY: env::Mark = env::Mark::of("recovery/entry");
pub const ENTRY_NAME: &str = "entry";
pub const ENTRY_PATH: &str = "recovery/entry";

pub const ATTACH: u8 = 1;
pub const STOP: u8 = 2;
pub const DETACH: u8 = 3;

/// 取得恢复映射：绑定号 ＋ 执行方 ＋ **本表里那一枚精确 BAR token** ＋ 页数 ＋ 能力表四格。
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
#[frame(len = 64)]
pub struct Attach {
    pub op: u8,
    pub serial: u64,
    pub execution: TaskId,
    pub bar: PieToken,
    pub pages: u32,
    pub common: u32,
    pub notify: u32,
    pub multiplier: u32,
    pub device: u32,
    pub back: PieToken,
}

/// 停／放：绑定号。
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
#[frame(len = 24)]
pub struct Step {
    pub op: u8,
    pub serial: u64,
    pub back: PieToken,
}

/// 收进来的一问。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Wire {
    Attach(Attach),
    Stop(Step),
    Detach(Step),
}

impl Wire {
    pub const LEN: usize = Attach::LEN;

    pub fn take(bytes: &[u8]) -> Option<Self> {
        match *bytes.first()? {
            ATTACH => {
                let (value, end) = <Attach as env::wire::Span>::fetch_at(bytes, 0)?;
                (end == bytes.len()).then_some(Self::Attach(value))
            }
            STOP => step::<{ STOP }>(bytes).map(Self::Stop),
            DETACH => step::<{ DETACH }>(bytes).map(Self::Detach),
            _ => None,
        }
    }

    pub fn store(&self, out: &mut [u8]) -> Option<usize> {
        match self {
            Self::Attach(value) if value.op == ATTACH => value.store_at(out, 0),
            Self::Stop(value) if value.op == STOP => value.store_at(out, 0),
            Self::Detach(value) if value.op == DETACH => value.store_at(out, 0),
            _ => None,
        }
    }
}

fn step<const OP: u8>(bytes: &[u8]) -> Option<Step> {
    let (value, end) = <Step as env::wire::Span>::fetch_at(bytes, 0)?;
    (value.op == OP && end == bytes.len()).then_some(value)
}

impl Message for Wire {
    type In = Self;
    type Buf = [u8; Self::LEN];
    const EMPTY: Self::Buf = [0; Self::LEN];

    fn store(&self, out: &mut [u8]) -> Option<usize> {
        Wire::store(self, out)
    }

    fn fetch(bytes: &[u8]) -> Option<Self> {
        Wire::take(bytes)
    }
}

/// 一答：状态 ＋ 本设备此刻的状态字节（`stop` 那一手回读的现场）。
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
#[frame(len = 8)]
pub struct Said {
    pub status: u8,
    pub device: u8,
}

impl Said {
    pub const NONE: Self = Self {
        status: 0,
        device: 0,
    };

    pub const fn of(status: u8) -> Self {
        Self {
            status,
            ..Self::NONE
        }
    }
}

impl Message for Said {
    type In = Self;
    type Buf = [u8; Self::LEN];
    const EMPTY: Self::Buf = [0; Self::LEN];

    fn store(&self, out: &mut [u8]) -> Option<usize> {
        self.store_at(out, 0)
    }

    fn fetch(bytes: &[u8]) -> Option<Self> {
        let (value, end) = <Self as env::wire::Span>::fetch_at(bytes, 0)?;
        (end == bytes.len()).then_some(value)
    }
}

/// 本手自己的失败词汇：每一格对应一个不同的下一步。
#[derive(Clone, Copy, PartialEq, Eq, Debug, env::WireCodes)]
#[wire(fallback = Bad)]
pub enum Fail {
    /// 这一号绑定不在本域记的账上。
    #[code(1)]
    Unknown,
    /// 同一号绑定的参数与已登记那一笔不一致。
    #[code(2)]
    Conflict,
    /// 本域拒了（窗不在、不是精确 BAR、能力表不成、执行方不对）。
    #[code(3)]
    Denied,
    /// 本设备没有按协议回读（reset 后状态不为 0）。
    #[code(4)]
    Unavailable,
    /// 本端读不懂那一句。
    #[code(5)]
    Bad,
}

/// 本台自己的失败方那一格（Hub 那本账据此记原始失败）。
pub const PARTY: u8 = hub_api::PARTY_RECOVERY;

/// 回信孔那一枚上的记号：这一面自己的。
pub const BACK: env::Mark = env::Mark::of("recovery-back");

fn reply_to(wire: &Wire) -> PieToken {
    match wire {
        Wire::Attach(frame) => frame.back,
        Wire::Stop(frame) => frame.back,
        Wire::Detach(frame) => frame.back,
    }
}

#[mold::contract(request = Wire, response = Said, mark = BACK, back = reply_to)]
pub struct Call;
