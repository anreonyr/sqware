//! 拆图入口上的测试暂停点：只在选景里、且夹具装上了才存在。
//!
//! `Cleanup::finish` 是在 `cull` 已经返回、所有 pend、COMMIT 与 PoleOperation 都
//! 放开之后才跑的：夹具借这一点暂停这一次拆图，去核“清理已经取得、但还没 finish”
//! 那一段真实状态，再由别处放行。
//!
//! 它不常开：整个模块只在 `cfg(debug_assertions)` ＋ 选景 feature 下编出来，
//! 夹具装的是它自己的函数，core 不认识任何场景。

use super::Pie;
use crate::machine::sync::SpinLock;

static PROBE: SpinLock<Option<fn(&[Pie])>> = SpinLock::new(None);

/// 装一个拆图暂停探针：`Cleanup::finish` 入口每次调用一次（带上这一次拆图卸下的
/// 那几枚 Pie）。置回 `None` 就拆掉。
pub(crate) fn arm(probe: Option<fn(&[Pie])>) {
    assert!(crate::testing());
    *PROBE.lock() = probe;
}

/// 调用暂停探针；先把探针取出来、放掉自己的锁，再调用——探针在锁外跑，
/// 不持任何 core 锁，也不与另一颗 hart 上的等待分享这一把锁。
pub(super) fn visit(removed: &[Pie]) {
    let probe = *PROBE.lock();
    if let Some(probe) = probe {
        probe(removed);
    }
}
