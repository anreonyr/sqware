//! PCI 驱动那一扇门的协议：落位、开启与清除 MASTER。
//!
//! 落位与 MASTER 是两手：落位只开内存空间（配置不授权发起），开启只在协调入口
//! （设备 Driver）核过全部前置之后发生。可信调用者由内核发送者身份确定。

use env::wire::Span as _;
use env::PieToken;
use wire::{Message, OK};

/// 本台那一扇门在树上与服务名下的坐标。
pub use pci_declaration::marks::ENTRY;
pub(crate) use pci_declaration::marks::CONFIG;
pub const ENTRY_NAME: &str = "entry";
pub const ENTRY_PATH: &str = "pci/entry";

/// 三个动作在报文里的码。
pub const PLACE: u8 = 1;
pub const ENABLE: u8 = 2;
pub const DISABLE: u8 = 3;
pub const RELEASE: u8 = 4;
pub const BAR_MARK: env::Mark = env::Mark::of("pci/recovery-bar");

/// 落位：绑定号、总线 requester 与 BAR。接收方是实际调用者。
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
#[frame(len = 24)]
pub struct Place {
    pub op: u8,
    pub serial: u64,
    pub requester: u32,
    pub bar: u8,
    pub back: PieToken,
}

/// 开启／清除 MASTER：绑定号与总线 requester。
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
#[frame(len = 24)]
pub struct Toggle {
    pub op: u8,
    pub serial: u64,
    pub requester: u32,
    pub back: PieToken,
}

/// 收进来的一问。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Wire {
    Place(Place),
    Enable(Toggle),
    Disable(Toggle),
    Release(Toggle),
}

impl Wire {
    pub const LEN: usize = Place::LEN;

    pub fn take(bytes: &[u8]) -> Option<Self> {
        match *bytes.first()? {
            PLACE => {
                let (value, end) = <Place as env::wire::Span>::fetch_at(bytes, 0)?;
                (end == bytes.len()).then_some(Self::Place(value))
            }
            ENABLE => {
                let (value, end) = <Toggle as env::wire::Span>::fetch_at(bytes, 0)?;
                (value.op == ENABLE && end == bytes.len()).then_some(Self::Enable(value))
            }
            DISABLE => {
                let (value, end) = <Toggle as env::wire::Span>::fetch_at(bytes, 0)?;
                (value.op == DISABLE && end == bytes.len()).then_some(Self::Disable(value))
            }
            RELEASE => {
                let (value, end) = <Toggle as env::wire::Span>::fetch_at(bytes, 0)?;
                (end == bytes.len()).then_some(Self::Release(value))
            }
            _ => None,
        }
    }

    pub fn store(&self, out: &mut [u8]) -> Option<usize> {
        match self {
            Self::Place(value) if value.op == PLACE => value.store_at(out, 0),
            Self::Enable(value) if value.op == ENABLE => value.store_at(out, 0),
            Self::Disable(value) if value.op == DISABLE => value.store_at(out, 0),
            Self::Release(value) if value.op == RELEASE => value.store_at(out, 0),
            _ => None,
        }
    }
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

/// 一答：状态 ＋ 落位那一枚的坐标。
///
/// `bar` 是实际调用者表里的精确 BAR token。
/// 不解释它、也不把它当自己表里的号。
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
#[frame(len = 16)]
pub struct Said {
    pub status: u8,
    pub pages: u32,
    pub bar: PieToken,
}

impl Said {
    pub const NONE: Self = Self {
        status: 0,
        pages: 0,
        bar: PieToken::NONE,
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
    /// 这一号绑定不在本域记的落位里。
    #[code(1)]
    Unknown,
    /// 同一号绑定的参数与已落位那一笔不一致。
    #[code(2)]
    Conflict,
    /// 本域拒了（设备不在、窗口放不下、状态不允许）。
    #[code(3)]
    Denied,
    /// 这一手不成：回读与写入对不上。
    #[code(4)]
    Unavailable,
    /// 本端读不懂那一句。
    #[code(5)]
    Bad,
}

/// 回信孔那一枚上的记号：这一面自己的。
pub const BACK: env::Mark = env::Mark::of("pci-back");

fn reply_to(wire: &Wire) -> PieToken {
    match wire {
        Wire::Place(frame) => frame.back,
        Wire::Enable(frame) => frame.back,
        Wire::Disable(frame) => frame.back,
        Wire::Release(frame) => frame.back,
    }
}

#[mold::contract(request = Wire, response = Said, mark = BACK, back = reply_to)]
pub struct Call;
