//! 一次 `Attach` 的控制权选择：这段映射由谁控制。
//!
//! 默认 `Caller`（发起 `Attach` 的那个 Task）；`SpaceMembers` 是目标 Space 成员，
//! 只在发起者同时通过来源授权与目标域授权时才可选，且**逐映射**生效——
//! 它把这一段登记到目标 Space 自己的控制记录上，不是“同 Space 自动全控”，
//! 也不给任何来源/其它 View 加权限。

use crate::{Decode, Wire};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewControl {
    /// 发起 `Attach` 的那个 Task（普通私有映射的默认）。
    Caller,
    /// 目标 Space 的成员（显式选择，逐映射生效）。
    SpaceMembers,
}

impl Wire for ViewControl {
    fn pack(&self, s: &mut [usize; 6], i: &mut usize) {
        let word = match self {
            Self::Caller => 0usize,
            Self::SpaceMembers => 1usize,
        };
        word.pack(s, i);
    }
    fn unpack(s: &[usize; 6], i: &mut usize) -> Result<Self, Decode> {
        match usize::unpack(s, i)? {
            0 => Ok(Self::Caller),
            1 => Ok(Self::SpaceMembers),
            _ => Err(Decode::Invalid),
        }
    }
}
