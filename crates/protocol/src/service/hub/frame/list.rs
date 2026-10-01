use alloc::string::String;
use env::PieToken;

use super::vocab::LIST;

/// 列册：类 ＋ **游标**（从哪一条起取窗）。
#[derive(env::Frame, Clone, PartialEq, Eq, Debug)]
#[frame(len = 45)]
pub struct ListReq {
    pub op: u8,
    pub class: String,
    pub from: u32,
    pub back: PieToken,
}

impl ListReq {
    /// 编一问（动作码固定 LIST）。
    pub fn of(class: String, from: u32, back: PieToken) -> ListReq {
        ListReq {
            op: LIST,
            class,
            from,
            back,
        }
    }
}
