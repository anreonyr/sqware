//! 锁序自检：它真的在跑，而不是每一次 check 都空转（`NoHeld`）。
//!
//! 检测启用点在内核公共 `init`（分配器就绪之后、被验证的调用之前）。
//! 这里既核对已启用，又核对持有集合真的在数、并在放锁后归零；
//! 两条判据（普遍获取与有序同级对）另喂合成持有集逐条对，不靠“qtest 没报”当证据。

use riscv::register::sstatus;

use crate::machine::sync::verdict::{peer, strict};
use crate::machine::sync::{Level, SpinLock, held_len, with_pair};

/// 锁序检测已启用，且持有集合随取放锁增减。
pub fn accept() {
    armed_and_counted();
    pair_keeps_argument_order_and_restores_the_set();
    strict_rules();
    pair_rules();
}

fn armed_and_counted() {
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

/// 一对同级锁：闭包按**调用者给的顺序**拿到两把；两种调用序各自独立核，
/// 每次之后持有集回到基线、陷阱态回到原值。
fn pair_keeps_argument_order_and_restores_the_set() {
    let a = SpinLock::new_level(Level::Flat, 0u32);
    let b = SpinLock::new_level(Level::Flat, 0u32);
    let sie = sstatus::read().sie();

    with_pair(&a, &b, |x, y| {
        *x = 33;
        *y = 44;
    });
    // 分两个作用域读，不把两把守卫叠在同一个表达式里。
    let a_value = *a.lock();
    let b_value = *b.lock();
    assert_eq!(a_value, 33, "[health] pair hands over a first");
    assert_eq!(b_value, 44, "[health] pair hands over b second");
    assert_eq!(
        held_len(),
        0,
        "[health] pair returns the held set to baseline"
    );
    assert_eq!(
        sstatus::read().sie(),
        sie,
        "[health] pair restores the trap state"
    );

    // 反序调用：闭包仍按 (a, b) 交出来。
    with_pair(&b, &a, |x, y| {
        *x = 55;
        *y = 66;
    });
    let a_value = *a.lock();
    let b_value = *b.lock();
    assert_eq!(
        a_value, 66,
        "[health] pair hands over a in the caller's order"
    );
    assert_eq!(
        b_value, 55,
        "[health] pair hands over b in the caller's order"
    );
    assert_eq!(
        held_len(),
        0,
        "[health] pair returns the held set to baseline"
    );
    assert_eq!(
        sstatus::read().sie(),
        sie,
        "[health] pair restores the trap state"
    );
}

/// 普遍获取：拒递归，锁级必须严格递增。
fn strict_rules() {
    let flat = Some(Level::Flat);
    let space = Some(Level::Space);
    assert!(
        strict([(0x1000, space)], 0x2000, flat).is_ok(),
        "[health] Flat may follow Space"
    );
    assert_eq!(
        strict([(0x1000, space)], 0x2000, space),
        Err("lock-order level violation"),
        "[health] the same level may not nest"
    );
    assert!(
        strict([(0x1000, space)], 0x2000, None).is_ok(),
        "[health] an exempt lock is always allowed"
    );
    assert_eq!(
        strict([(0x1000, space)], 0x1000, flat),
        Err("recursive acquisition"),
        "[health] the same lock may not be taken twice"
    );
    assert!(
        strict(core::iter::empty(), 0x2000, Some(Level::Unit)).is_ok(),
        "[health] an empty held set allows anything"
    );
    let frame = Some(Level::Frame);
    assert_eq!(
        strict([(0x3000, frame)], 0x2000, flat),
        Err("lock-order level violation"),
        "[health] a lower level may not follow Frame"
    );
    assert!(
        strict([(0x3000, frame)], 0x2000, Some(Level::Block)).is_ok(),
        "[health] Block may follow Frame"
    );
}

/// 有序同级对：只给 Flat、先取的必须是已持的那一把且地址更小、同级只此一对。
fn pair_rules() {
    let flat = Some(Level::Flat);
    let space = Some(Level::Space);
    const LOWER: usize = 0x1000;
    const UPPER: usize = 0x2000;
    assert!(
        peer([(LOWER, flat)], UPPER, LOWER, flat).is_ok(),
        "[health] the declared predecessor in address order is allowed"
    );
    assert_eq!(
        peer([(LOWER, space)], UPPER, LOWER, space),
        Err("ordered pair is only declared for Level::Flat"),
        "[health] the pair exception is only declared for Flat"
    );
    assert_eq!(
        peer([(LOWER, space)], UPPER, LOWER, flat),
        Err("the held same-level lock is not the declared predecessor"),
        "[health] no same-level predecessor is held"
    );
    assert_eq!(
        peer([(0x3000, flat)], UPPER, LOWER, flat),
        Err("the held same-level lock is not the declared predecessor"),
        "[health] the held same-level lock is not the declared one"
    );
    assert_eq!(
        peer([(UPPER, flat)], LOWER, UPPER, flat),
        Err("the held same-level lock is not the declared predecessor"),
        "[health] the pair may not be taken in reverse address order"
    );
    assert_eq!(
        peer([(LOWER, flat), (0x3000, flat)], UPPER, LOWER, flat),
        Err("the held same-level lock is not the declared predecessor"),
        "[health] two same-level locks are already held"
    );
    assert_eq!(
        peer([(UPPER, flat)], UPPER, UPPER, flat),
        Err("recursive acquisition"),
        "[health] a pair may not re-take a held lock"
    );
    assert_eq!(
        peer(
            [(LOWER, flat), (0x4000, Some(Level::Frame))],
            UPPER,
            LOWER,
            flat
        ),
        Err("ordered pair under a higher level"),
        "[health] a pair may not sit under a higher level"
    );
    assert!(
        peer([(LOWER, flat), (0x4000, space)], UPPER, LOWER, flat).is_ok(),
        "[health] a lower level under the pair is fine"
    );
}
