//! IOMMU 驱动那一扇门的协议：取得帧引用、发布翻译、撤销、关队列与解除引用。
//!
//! 五手各自一个独立效果，不合并（撤翻译不等于排空、停止成功不等于资源关闭成功）。
//! 报文里不接收可信 caller：发送者由内核给出；交付进来的来源窗按 `vestor` 与本次绑定
//! 的记号核（不经报文自报）。

use env::wire::Span as _;
use env::{PieToken, TaskId};
use wire::{Message, OK};

/// 本台那一扇门在树上的坐标。
pub const ENTRY: env::Mark = env::Mark::of("iommu/entry");
pub const ENTRY_NAME: &str = "entry";
pub const ENTRY_PATH: &str = "iommu/entry";

/// 五个动作在报文里的码。
pub const ACQUIRE: u8 = 1;
pub const PUBLISH: u8 = 2;
pub const INVALIDATE: u8 = 3;
pub const QUIESCE: u8 = 4;
pub const RELEASE: u8 = 5;

/// 取得帧引用：绑定号 ＋ 设备号 ＋ 执行方 ＋ 本绑定的 IOVA 起址 ＋ 访问位。
///
/// 窗的页数与长度**由执行方交进来的那一枚窗自己说**（打开它量一下），不由报文自报；
/// `iova` 与绑定号推出的起址当场对，对不上就拒。
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
#[frame(len = 48)]
pub struct Acquire {
    pub op: u8,
    pub serial: u64,
    pub requester: u32,
    pub execution: TaskId,
    pub iova: u64,
    pub access: u32,
    pub back: PieToken,
}

/// 一步（发布／撤销／关队列／解除）：绑定号 ＋ 本次责任的理由（只有 `release` 用它）。
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
#[frame(len = 24)]
pub struct Step {
    pub op: u8,
    pub serial: u64,
    pub reason: u8,
    pub back: PieToken,
}

/// 收进来的一问。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Wire {
    Acquire(Acquire),
    Publish(Step),
    Invalidate(Step),
    Quiesce(Step),
    Release(Step),
}

impl Wire {
    pub const LEN: usize = Acquire::LEN;

    pub fn take(bytes: &[u8]) -> Option<Self> {
        match *bytes.first()? {
            ACQUIRE => {
                let (value, end) = <Acquire as env::wire::Span>::fetch_at(bytes, 0)?;
                (end == bytes.len()).then_some(Self::Acquire(value))
            }
            PUBLISH => step::<{ PUBLISH }>(bytes).map(Self::Publish),
            INVALIDATE => step::<{ INVALIDATE }>(bytes).map(Self::Invalidate),
            QUIESCE => step::<{ QUIESCE }>(bytes).map(Self::Quiesce),
            RELEASE => step::<{ RELEASE }>(bytes).map(Self::Release),
            _ => None,
        }
    }

    pub fn store(&self, out: &mut [u8]) -> Option<usize> {
        match self {
            Self::Acquire(value) if value.op == ACQUIRE => value.store_at(out, 0),
            Self::Publish(value) if value.op == PUBLISH => value.store_at(out, 0),
            Self::Invalidate(value) if value.op == INVALIDATE => value.store_at(out, 0),
            Self::Quiesce(value) if value.op == QUIESCE => value.store_at(out, 0),
            Self::Release(value) if value.op == RELEASE => value.store_at(out, 0),
            _ => None,
        }
    }
}

/// 那一形四步共用：动作码对上了才收。
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

/// 一答：状态 ＋ 还要不要再推一格 ＋ 此刻还持有几页帧。
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
#[frame(len = 16)]
pub struct Said {
    pub status: u8,
    pub pending: u8,
    pub held: u32,
}

impl Said {
    pub const NONE: Self = Self {
        status: 0,
        pending: 0,
        held: 0,
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
    /// 同一号绑定的参数与已登记那一笔不一致（或同时只有一笔的范围内已有别的绑定）。
    #[code(2)]
    Conflict,
    /// 本域拒了（窗不在、权限不够、区间非法、访问位越界）。
    #[code(3)]
    Denied,
    /// 硬件没按协议回答（回读不对、fence 超期、队列不停）。
    #[code(4)]
    Unavailable,
    /// 本端读不懂那一句。
    #[code(5)]
    Bad,
    /// **执行方还没把窗交到本域表里**：这一步还没就绪，不是失败（协调入口原样重试）。
    #[code(6)]
    NotReady,
}

/// 本台自己的失败方那一格（Hub 那本账据此记原始失败）。
pub const PARTY: u8 = hub_api::PARTY_IOMMU;

/// 回信孔那一枚上的记号：这一面自己的。
pub const BACK: env::Mark = env::Mark::of("iommu-back");

fn reply_to(wire: &Wire) -> PieToken {
    match wire {
        Wire::Acquire(frame) => frame.back,
        Wire::Publish(frame) => frame.back,
        Wire::Invalidate(frame) => frame.back,
        Wire::Quiesce(frame) => frame.back,
        Wire::Release(frame) => frame.back,
    }
}

#[mold::contract(request = Wire, response = Said, mark = BACK, back = reply_to)]
pub struct Call;
