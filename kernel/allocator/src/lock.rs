/// 分配器内部锁的角色。`None` 是既有 Bump/Pool/Pump 那类不参与层级图的锁。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LockRole {
    Frame,
    Tally,
    Spare,
}

/// 分配器只依赖这一条锁契约：原生 unit guard。
///
/// # Safety
///
/// `lock` 必须为每次调用提供互斥的原生 guard：取得时与释放时同操作共享
/// Acquire/Release（或等价 happens-before），保护写对所有持锁者可见；guard 覆盖调用方
/// 对状态的整个访问，drop 时解锁，且不得把守卫或状态借用带出该访问。
pub unsafe trait Lock: Sync + Sized {
    type Guard<'a>
    where
        Self: 'a;

    /// 按角色构造原生锁；`None` 不进入层级图。
    fn new(role: Option<LockRole>) -> Self;

    /// 取原生 unit guard。
    fn lock(&self) -> Self::Guard<'_>;
}
