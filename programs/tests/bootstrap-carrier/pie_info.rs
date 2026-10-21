#[derive(Clone)]
pub struct PieInfo {
    pub token: crate::PieToken,
    pub mark: crate::Mark,
    pub owner: crate::TaskId,
    pub vestor: crate::TaskId,
    pub alive: bool,
    pub kind: crate::PieKind,
    pub permission: crate::Permission,
}
