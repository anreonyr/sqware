//! Private Hub → Control device-identity installation, never an Operator tile.
use env::{PieToken, TaskId};

use env::wire::Span as _;
use system_api::identity::{CoalitionId, PageId};
use wire::message::Message;

pub use super::marks::ACTIVATE_BACK as BACK;
pub use super::marks::ACTIVATE_ENTRY as ENTRY;

/// 一趟最多几枚盟：**与入册那一段同一格**（一张装配单上"类"的条数上界）
pub const ACTIVATE_MAX: usize = super::frame::ENROLL_MAX;

/// **一趟报完这一批**：哪几枚盟归谁管。
///
/// 原来是"一枚盟一趟"——装配期 hub 起手每类一趟、十几类就是十几趟同步往返，全压在它
/// "报就绪"之前（那一族见 `kernel/src/layout.rs` 头注）。收成一趟之后，hub 起手那一段
/// 只剩"立盟"那十几趟（identity）＋**这一趟**（control）。
#[derive(Clone, Copy, env::Frame)]
pub struct Activate {
    pub task: TaskId,
    n: u8,
    #[frame(count = n, fill = CoalitionId::EMPTY)]
    coalitions: [CoalitionId; ACTIVATE_MAX],
    pub back: PieToken,
}

impl Activate {
    /// 编一段：**一枚盟号都不翻译**（条数 ＋ 那几枚号照原样过线）
    pub fn of(task: TaskId, coalitions: &[CoalitionId], back: PieToken) -> Option<Self> {
        if coalitions.is_empty() || coalitions.len() > ACTIVATE_MAX {
            return None;
        }
        let mut held = [CoalitionId::EMPTY; ACTIVATE_MAX];
        held.get_mut(..coalitions.len())?
            .copy_from_slice(coalitions);
        Some(Self {
            task,
            n: coalitions.len() as u8,
            coalitions: held,
            back,
        })
    }

    /// 几枚
    pub fn len(&self) -> usize {
        self.n as usize
    }

    /// 第 `i` 枚（`i < len()` 才有）
    pub fn coalition(&self, i: usize) -> Option<CoalitionId> {
        (i < self.len()).then(|| self.coalitions[i])
    }

    /// 合不合规矩：**这一段非空且不越界、发送者不空、每一枚的 authority 不空**
    fn valid(&self) -> bool {
        let n = self.len();
        n > 0
            && n <= ACTIVATE_MAX
            && self.task.get() != 0
            && self.coalitions[..n].iter().all(|c| c.authority.get() != 0)
    }
}

impl Message for Activate {
    type In = Self;
    type Buf = [u8; Self::LEN];
    const EMPTY: Self::Buf = [0; Self::LEN];

    fn store(&self, out: &mut [u8]) -> Option<usize> {
        self.valid().then_some(())?;
        self.store_at(out, 0)
    }

    fn fetch(bytes: &[u8]) -> Option<Self> {
        let (ask, end) = Self::fetch_at(bytes, 0)?;
        (end == bytes.len() && ask.valid()).then_some(ask)
    }
}
