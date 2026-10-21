//! 本地精确派生请求：目标就是调用者，范围为继承或按来源起点收窄。

use env::{GrantRange, Mark, Permission, PieToken};

pub struct Derivation {
    pub source: PieToken,
    pub permission: Permission,
    pub mark: Mark,
    pub range: GrantRange,
}
