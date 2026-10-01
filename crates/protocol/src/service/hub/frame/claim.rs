use env::PieToken;

use super::vocab::CLAIM;

/// 认领：**要什么权**（种 / 取用 / 形态）＋ **主人那一枚**。**"哪一台"不在这帧里**——你 `find`
/// 的是哪一格，那一格上挂的就是哪一台那一份孔
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Claim {
    pub op: u8,
    pub kind: u8,
    pub access: u32,
    pub policy: u32,
    pub sensor: PieToken,
    pub back: PieToken,
}

impl Claim {
    /// 编一问（动作码固定 CLAIM）
    pub fn of(kind: u8, access: u32, policy: u32, sensor: PieToken, back: PieToken) -> Claim {
        Claim {
            op: CLAIM,
            kind,
            access,
            policy,
            sensor,
            back,
        }
    }
}
