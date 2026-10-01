use alloc::string::String;
use env::PieToken;

use super::vocab::BOND;

/// 报名：**只有类**（驱动不需要知道盟号）
#[derive(env::Frame, Clone, PartialEq, Eq, Debug)]
#[frame(len = 41)]
pub struct Bond {
    pub op: u8,
    pub class: String,
    pub back: PieToken,
}

impl Bond {
    /// 编一问（动作码固定 BOND）
    pub fn of(class: String, back: PieToken) -> Bond {
        Bond {
            op: BOND,
            class,
            back,
        }
    }
}
