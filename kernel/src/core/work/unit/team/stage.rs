/// 团队的阶段：还在构造（没有默认入口）或已就绪。
pub(crate) enum TeamStage {
    Constructing,
    Ready { default_entry: usize },
}
