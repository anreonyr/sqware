//! work 全域的提交锁：任务与团队的状态迁移、pend 的登记、mail 的交接都在它下面
//! 串行。

static COMMIT: crate::machine::sync::RelLock<()> =
    crate::machine::sync::RelLock::new_level(crate::machine::sync::Level::Unit, ());

#[cfg(debug_assertions)]
pub(crate) fn assert_unlocked() {
    assert!(
        !COMMIT.held_by_current(),
        "idle wait while holding Unit commit lock"
    );
}

pub(crate) fn commit() -> crate::machine::sync::reentrant::RelLockGuard<'static, ()> {
    COMMIT.lock()
}
