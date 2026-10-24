//! 一答：状态、是否还要再推一格、还持有几页帧，以及本 Domain 的私有分配。

use env::TaskId;
use env::wire::Span as _;
use wire::Message;

/// 38 字节；接收缓冲按 48 备。
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
#[frame(len = 38)]
pub struct Said {
    pub status: u8,
    pub pending: u8,
    pub held: u32,
    pub engine: TaskId,
    pub key: u64,
    pub iova: u64,
    pub bytes: u64,
}

impl Said {
    pub const NONE: Self = Self {
        status: 0,
        pending: 0,
        held: 0,
        engine: TaskId::new(0),
        key: 0,
        iova: 0,
        bytes: 0,
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
    type Buf = [u8; 48];
    const EMPTY: Self::Buf = [0; 48];

    fn store(&self, out: &mut [u8]) -> Option<usize> {
        self.store_at(out, 0)
    }

    fn fetch(bytes: &[u8]) -> Option<Self> {
        let (value, end) = <Self as env::wire::Span>::fetch_at(bytes, 0)?;
        (end == bytes.len()).then_some(value)
    }
}
