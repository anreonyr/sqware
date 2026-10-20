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

/// 这一步必须在提交段里做：调用方自己取锁并把它一直持到发表完毕。
///
/// 只看“当场事实”的判据（如来源侧新使用判据）靠它把前提写死在代码里：
/// 单拿谓词去判、再在段外发表，中间会有人插进一条新占用。
#[cfg(debug_assertions)]
pub(crate) fn assert_held() {
    assert!(
        COMMIT.held_by_current(),
        "this step belongs inside the Unit commit section"
    );
}

pub(crate) fn commit() -> crate::machine::sync::reentrant::RelLockGuard<'static, ()> {
    COMMIT.lock()
}
