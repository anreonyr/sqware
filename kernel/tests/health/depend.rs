//! 锁序检测自检：它真的在跑，而不是每一次 check 都空转（`NoHeld`）。
//!
//! 检测启用点在内核公共 `init`（分配器就绪之后、被验证的调用之前）。
//! 这里既核对已启用，又核对持有集合真的在数、并在放锁后归零。

/// 锁序检测已启用，且持有集合随取放锁增减。
pub fn accept() {
    assert!(
        crate::machine::sync::armed(),
        "[health] lock dependency check is not armed"
    );
    assert_eq!(
        crate::machine::sync::held_len(),
        0,
        "[health] held lock set is not empty at rest"
    );
    let lock = crate::machine::sync::SpinLock::new_level(crate::machine::sync::Level::Space, 0u8);
    {
        let guard = lock.lock();
        assert_eq!(
            crate::machine::sync::held_len(),
            1,
            "[health] held lock set does not track an acquisition"
        );
        assert_eq!(*guard, 0);
    }
    assert_eq!(
        crate::machine::sync::held_len(),
        0,
        "[health] held lock set does not clear on release"
    );
}
