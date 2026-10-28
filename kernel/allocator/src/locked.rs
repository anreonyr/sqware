/// 一份状态只存一次；访问必须穿过原生 unit guard。辅助器不对外。
pub(crate) struct Locked<L: crate::Lock, T> {
    lock: L,
    state: core::cell::UnsafeCell<T>,
}

impl<L: crate::Lock, T> Locked<L, T> {
    pub(crate) const fn new(lock: L, state: T) -> Self {
        Self {
            lock,
            state: core::cell::UnsafeCell::new(state),
        }
    }

    /// 闭包是访问边界：原生 guard 覆盖整个访问，状态借用不越过 guard。
    pub(crate) fn with<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        let _guard = self.lock.lock();
        f(unsafe { &mut *self.state.get() })
    }
}

// SAFETY: 所有状态读写都发生在 `L::lock` 的原生 guard 之内；跨线程只要求状态可移动。
unsafe impl<L: crate::Lock, T: Send> Sync for Locked<L, T> {}
