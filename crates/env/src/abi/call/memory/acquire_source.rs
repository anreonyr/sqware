//! `Acquire` 的取得来源：一处来源数据，不按新旧或共享方式分操作。
//!
//! 独立的保留授权**不在本枚举里**：它是同一次 `Acquire` 的另一个可选入参
//! （那一格的载荷自己说）。

use crate::wire::{FlatReference, PieToken, TeamId, ViewId};
use crate::{Decode, Wire};

use super::allocation::Allocation;
use super::pages;
use super::reference_range::ReferenceRange;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AcquireSource {
    /// 新 RAM：取得完整来源持有，并建立一次性创建资格。
    Allocation(Allocation),
    /// 自己已有引用上的子范围：同来源、同授权上限、同撤销依赖。
    Reference(ReferenceRange),
    /// Pole 的 Pie 与授权范围内的页窗口。
    Source {
        token: PieToken,
        offset: usize,
        count: usize,
    },
    /// 目标域里已授权 View 的来源范围；`team` 是定位它的域约束。
    /// ViewId 或可读 VA 本身不是凭证，还要核所属 Nest 与来源授权。
    View {
        team: TeamId,
        view: ViewId,
        offset: usize,
        count: usize,
    },
}

impl AcquireSource {
    /// 正文里认得的标签数：越界标签即非法。
    const TAGS: usize = 4;

    /// 载荷宽度：最多的一支是 `View`（标签 + 域 + 句柄 + 偏移 + 页数）。
    const WORDS: usize = 5;
}

impl Wire for AcquireSource {
    fn pack(&self, s: &mut [usize; 6], i: &mut usize) {
        let words = match *self {
            Self::Allocation(allocation) => [0, allocation.size, allocation.align, 0, 0],
            Self::Reference(range) => [1, range.reference.get(), range.offset, range.count, 0],
            Self::Source {
                token,
                offset,
                count,
            } => [2, token.get(), offset, count, 0],
            Self::View {
                team,
                view,
                offset,
                count,
            } => [3, team.get(), view.get(), offset, count],
        };
        for word in words {
            word.pack(s, i);
        }
        debug_assert!(*i <= Self::WORDS);
    }
    fn unpack(s: &[usize; 6], i: &mut usize) -> Result<Self, Decode> {
        let mut w = [0usize; Self::WORDS];
        for word in &mut w {
            *word = usize::unpack(s, i)?;
        }
        let [tag, a, b, c, d] = w;
        if tag >= Self::TAGS {
            return Err(Decode::Invalid);
        }
        let window = |offset: usize, count: usize| pages(offset, count).map(|()| (offset, count));
        match tag {
            0 if c == 0 && d == 0 => Ok(Self::Allocation(
                Allocation::new(a, b).ok_or(Decode::Invalid)?,
            )),
            1 if d == 0 => Ok(Self::Reference(
                ReferenceRange::new(FlatReference::new(a), b, c).ok_or(Decode::Invalid)?,
            )),
            2 if d == 0 => {
                let (offset, count) = window(b, c)?;
                Ok(Self::Source {
                    token: PieToken::new(a),
                    offset,
                    count,
                })
            }
            3 => {
                let (offset, count) = window(c, d)?;
                Ok(Self::View {
                    team: TeamId::new(a),
                    view: ViewId::new(b),
                    offset,
                    count,
                })
            }
            _ => Err(Decode::Invalid),
        }
    }
}
