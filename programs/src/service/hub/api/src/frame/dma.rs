//! DMA 安全绑定那一面的帧与词汇：发现结果、绑定编号、关闭理由与协调状态。
//!
//! 与普通设备账那三面（bond/list/claim）分开：普通设备的主人一没，格子当场空出来；
//! DMA 绑定在责任结清前**不**因主人死亡变为可重绑。这一面是 Hub 与
//! PCI / IOMMU / 设备恢复 / 执行实例之间的协调协议，报文里不接收可信 caller
//! （发送者由内核给出，绑定编号由 Hub 签发）。

use alloc::string::String;
use env::wire::Span as _;
use env::{PieToken, TaskId};
use wire::Message;

/// 七个动作在报文里的码（与那一族同名）。
pub const DMA_ENROLL: u8 = 1;
pub const DMA_CLAIM: u8 = 2;
pub const DMA_CLOSE: u8 = 3;
pub const DMA_INSPECT: u8 = 4;
pub const DMA_SETTLE: u8 = 5;

/// Hub 签发的绑定身份。`serial` 和 IOVA 不复用，地址范围耗尽终判 `Full`。
/// 未结清记录始终保留；最近 32 笔结清结果可查询，淘汰后查询返回 `Unknown`。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct BindingId {
    pub hub: TaskId,
    pub serial: u64,
}

/// 绑定的协调状态（只属 Hub 的账，不复制 Driver 的寄存器读数）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BindingState {
    /// 已建记录，发布前置还没备齐。
    Preparing,
    /// 前置全部满足，业务已放行。
    Active,
    /// 已拒绝新业务，停链与回收在推进。
    Closing,
    /// 本次责任全部结清，设备已可重新认领。
    Closed,
}

impl BindingState {
    pub const fn code(self) -> u8 {
        match self {
            Self::Preparing => 1,
            Self::Active => 2,
            Self::Closing => 3,
            Self::Closed => 4,
        }
    }

    pub const fn of_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::Preparing),
            2 => Some(Self::Active),
            3 => Some(Self::Closing),
            4 => Some(Self::Closed),
            _ => None,
        }
    }
}

/// 关闭理由：Fault 可覆盖 Normal，不反向恢复。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Close {
    Normal,
    Fault,
}

impl Close {
    pub const fn code(self) -> u8 {
        match self {
            Self::Normal => 1,
            Self::Fault => 2,
        }
    }

    pub const fn of_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::Normal),
            2 => Some(Self::Fault),
            _ => None,
        }
    }
}

/// 失败记录：**实际失败方、操作与原失败码**。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Failure {
    pub party: u8,
    pub op: u8,
    pub code: u8,
}

/// 失败方（谁那一手没成）。
pub const PARTY_HUB: u8 = 1;
pub const PARTY_PCI: u8 = 2;
pub const PARTY_IOMMU: u8 = 3;
pub const PARTY_RECOVERY: u8 = 4;
pub const PARTY_CONTROL: u8 = 5;

/// Device registration metadata. Its source is the authenticated kernel sender.
#[derive(env::Frame, Clone, PartialEq, Eq, Debug)]
#[frame(len = 83)]
pub struct Enroll {
    pub op: u8,
    pub key: u64,
    pub line: u32,
    pub name: String,
    pub class: String,
    pub back: PieToken,
}

/// 列已登记设备：**不提供**。设备由「哪一枚门牌」认（见 `DmaClaim`），
/// 不向普通任务开放设备清单。

/// 关闭本次绑定：重复请求继续同一责任，Fault 可覆盖 Normal。
/// 认领一台：调用方本次 claim 的重试编号 ＋ 主人那一枚报活孔。
///
/// **不装设备号**：“哪一台”由“哪一枚设备门牌响了”回答（与普通设备那一面同一条口径）——
/// 门牌上挂着类联盟的 permit，准入就是既有 Operator/Identity 那一套。
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
#[frame(len = 32)]
pub struct DmaClaim {
    pub op: u8,
    pub id: u64,
    pub sensor: PieToken,
    pub back: PieToken,
}

/// 关闭本次绑定：重复请求继续同一责任，Fault 可覆盖 Normal。
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
#[frame(len = 32)]
pub struct CloseBinding {
    pub op: u8,
    pub serial: u64,
    pub reason: u8,
    pub back: PieToken,
}

/// 查看同一绑定的协调进度或原始失败。
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
#[frame(len = 32)]
pub struct Inspect {
    pub op: u8,
    pub serial: u64,
    pub back: PieToken,
}

/// 整机退场门：关闭执行实例与开放绑定，报告未结清责任数。
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
#[frame(len = 16)]
pub struct Settle {
    pub op: u8,
    pub back: PieToken,
}

/// 收进来的一问。
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum DmaWire {
    Enroll(Enroll),
    Claim(DmaClaim),
    Close(CloseBinding),
    Inspect(Inspect),
    Settle(Settle),
}

impl DmaWire {
    pub const LEN: usize = Enroll::LEN;

    pub fn take(bytes: &[u8]) -> Option<Self> {
        match *bytes.first()? {
            DMA_ENROLL => {
                let (value, end) = <Enroll as env::wire::Span>::fetch_at(bytes, 0)?;
                (end == bytes.len()).then_some(Self::Enroll(value))
            }
            DMA_CLAIM => {
                let (value, end) = <DmaClaim as env::wire::Span>::fetch_at(bytes, 0)?;
                (end == bytes.len()).then_some(Self::Claim(value))
            }
            DMA_CLOSE => {
                let (value, end) = <CloseBinding as env::wire::Span>::fetch_at(bytes, 0)?;
                (end == bytes.len()).then_some(Self::Close(value))
            }
            DMA_INSPECT => {
                let (value, end) = <Inspect as env::wire::Span>::fetch_at(bytes, 0)?;
                (end == bytes.len()).then_some(Self::Inspect(value))
            }
            DMA_SETTLE => {
                let (value, end) = <Settle as env::wire::Span>::fetch_at(bytes, 0)?;
                (end == bytes.len()).then_some(Self::Settle(value))
            }
            _ => None,
        }
    }

    pub fn store(&self, out: &mut [u8]) -> Option<usize> {
        match self {
            Self::Enroll(value) if value.op == DMA_ENROLL => value.store_at(out, 0),
            Self::Claim(value) if value.op == DMA_CLAIM => value.store_at(out, 0),
            Self::Close(value) if value.op == DMA_CLOSE => value.store_at(out, 0),
            Self::Inspect(value) if value.op == DMA_INSPECT => value.store_at(out, 0),
            Self::Settle(value) if value.op == DMA_SETTLE => value.store_at(out, 0),
            _ => None,
        }
    }
}

impl Message for DmaWire {
    type In = Self;
    type Buf = [u8; Self::LEN];
    const EMPTY: Self::Buf = [0; Self::LEN];

    fn store(&self, out: &mut [u8]) -> Option<usize> {
        DmaWire::store(self, out)
    }

    fn fetch(bytes: &[u8]) -> Option<Self> {
        DmaWire::take(bytes)
    }
}

/// 一答：状态 ＋ 绑定进度 ＋ 失败记录 ＋ 未结清笔数。
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
#[frame(len = 16)]
pub struct DmaSaid {
    pub status: u8,
    pub state: u8,
    pub party: u8,
    pub op: u8,
    pub code: u8,
    pub count: u8,
    pub serial: u64,
}

impl DmaSaid {
    /// 空的一张（失败那一答用它）。
    pub const NONE: Self = Self {
        status: 0,
        state: 0,
        party: 0,
        op: 0,
        code: 0,
        count: 0,
        serial: 0,
    };

    pub const fn of(status: u8) -> Self {
        Self {
            status,
            ..Self::NONE
        }
    }

    /// 一张签好的绑定编号。
    pub const fn bound(serial: u64, state: BindingState) -> Self {
        Self {
            status: wire::OK,
            state: state.code(),
            serial,
            ..Self::NONE
        }
    }

    /// 一份进度：状态 ＋ 原始失败（没失败时失败方与码都是 0）。
    pub const fn progress(serial: u64, state: BindingState, failure: Option<Failure>) -> Self {
        let (party, op, code) = match failure {
            Some(failure) => (failure.party, failure.op, failure.code),
            None => (0, 0, 0),
        };
        Self {
            status: wire::OK,
            state: state.code(),
            party,
            op,
            code,
            serial,
            ..Self::NONE
        }
    }

    pub fn failure(&self) -> Option<Failure> {
        (self.party != 0).then_some(Failure {
            party: self.party,
            op: self.op,
            code: self.code,
        })
    }
}

impl Message for DmaSaid {
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

/// 回信孔那一枚上的记号：**这一面自己的**（与普通设备账那三面分得开）。
pub const DMA_BACK: env::Mark = env::Mark::of("hub-dma-back");

/// 从一问里取出回信孔那一格。
fn reply_to(wire: &DmaWire) -> PieToken {
    match wire {
        DmaWire::Enroll(frame) => frame.back,
        DmaWire::Claim(frame) => frame.back,
        DmaWire::Close(frame) => frame.back,
        DmaWire::Inspect(frame) => frame.back,
        DmaWire::Settle(frame) => frame.back,
    }
}

/// 这一面的契约：一问一答、回信孔那一格随动作码走。
#[mold::contract(request = DmaWire, response = DmaSaid, mark = DMA_BACK, back = reply_to)]
pub struct Call;
