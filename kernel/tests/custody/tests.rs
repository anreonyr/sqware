// 托管状态机的判据：单取独占、批量登记的前缀语义、放回与幂等。

use super::custody::{Custody, register};
use super::error::FlatError;

/// 批量登记依次进行：前缀已登记，碰到在途的那一份整批失败，**前缀不回退**。
#[test]
fn a_batch_registers_its_prefix_then_stops_at_an_in_transit_carrier() {
    let first = Custody::new();
    let second = Custody::new();
    assert!(!first.is_aliased(), "新造的那一份还没被登记");
    assert!(second.enter(), "空载的那一份可以取独占");
    assert_eq!(
        register([&first, &second]),
        Err(FlatError::Busy),
        "碰在途的那一份整批报 Busy"
    );
    assert!(first.is_aliased(), "失败之后已经登记的前缀仍是已登记");
    // 单登记那一份是幂等的。
    assert_eq!(register([&first]), Ok(()));
}

/// 在途的那一份既不能再取独占，也不能被登记。
#[test]
fn an_in_transit_carrier_is_neither_claimed_nor_registered() {
    let custody = Custody::new();
    assert!(custody.enter());
    assert!(!custody.enter(), "在途的承载不能再取独占");
    assert_eq!(custody.register(), Err(FlatError::Busy));
}

/// 放回之后能再取独占；登记一次之后重复登记是幂等的。
#[test]
fn a_released_carrier_is_re_claimable_and_registration_is_idempotent() {
    let custody = Custody::new();
    assert!(custody.enter());
    custody.leave();
    assert!(custody.enter(), "放回之后还能再取独占");
    custody.leave();
    assert_eq!(register([&custody]), Ok(()));
    assert!(custody.is_aliased());
    assert_eq!(register([&custody]), Ok(()), "重复登记是幂等的");
}
