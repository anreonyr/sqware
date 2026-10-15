//! 按地址序共持两把同级锁。
//!
//! 锁序检查器只允许严格递增，所以两把同级锁不能各自走普通 `lock()` 共持。
//! 这里给出唯一一条例外：**显式**声明一对同级锁，按**地址序**取得，
//! 由检查器核对“先取的正是这一把、且同级只此一对”。普通 `lock()` 的判据不变。
//!
//! 临界区由闭包包住：两个守卫都在闭包内取得、同作用域内按逆序析构，
//! 既不会外泄，也不会乱序。

use super::SpinLock;

/// 按地址序共持 `a` 与 `b`，闭包按调用者给的顺序拿到两把锁的 `&mut`。
///
/// 同一把锁不能与自己成对：所有构建都当场断言。
pub(crate) fn with_pair<A, B, R>(
    a: &SpinLock<A>,
    b: &SpinLock<B>,
    body: impl FnOnce(&mut A, &mut B) -> R,
) -> R {
    let a_at = a as *const SpinLock<A> as usize;
    let b_at = b as *const SpinLock<B> as usize;
    assert!(a_at != b_at, "ordered pair needs two distinct locks");
    if a_at < b_at {
        let mut first = a.lock();
        let mut second = b.lock_ordered(a);
        let out = body(&mut first, &mut second);
        drop(second);
        drop(first);
        out
    } else {
        let mut first = b.lock();
        let mut second = a.lock_ordered(b);
        let out = body(&mut second, &mut first);
        drop(second);
        drop(first);
        out
    }
}
