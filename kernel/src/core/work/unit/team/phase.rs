/// 团队的相位：活着、被收下、注定要死，或已被撤离。
pub(crate) enum TeamPhase {
    Live,
    Paused,
    Doomed,
    Ousted,
}

impl TeamPhase {
    /// 注定要死或已被撤离：这两种相位都不再接新的事。
    pub(crate) fn terminal(&self) -> bool {
        matches!(self, TeamPhase::Doomed | TeamPhase::Ousted)
    }

    /// 已被撤离：名字与家长位都不再指向它。
    pub(crate) fn ousted(&self) -> bool {
        matches!(self, TeamPhase::Ousted)
    }
}
