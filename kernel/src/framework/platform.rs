//! 适配（framework/platform.rs）— 汇报的**策略出口**。
//!
//! 框架核心（[`crate::framework::runner`]）不认识控制台：它只调 [`Platform::print`]。
//! 内核侧的实现 [`Kernel`] 把它接到 `putln!` —— 于是"用例怎么跑"与"结果怎么出去"分开。

use core::fmt;

/// 跑完之后的结局。
///
/// **只有"通过"一个取值**：失败走不到这里 —— 用例里的 `assert!` 直接进 panic 通道
/// （[`super::case_failed`]），那一刻就报完并停机了。本枚举存在的理由是让 `run` 的
/// 返回值读起来是一句话，也是将来万一要"跑完再判"时的落点。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Status {
    /// 全部用例通过。
    Pass,
}

/// 汇报的策略出口。
///
/// `print` 必须**无锁无堆**：用例跑在启动早期，也在崩溃路径上会被调。
pub(crate) trait Platform: Sync {
    /// 输出一行（内核侧：`putln!` 直写控制台，**自带换行**）。
    fn print(&self, args: fmt::Arguments);
}

/// 内核侧实现：控制台直写。
pub(crate) struct Kernel;

impl Platform for Kernel {
    fn print(&self, args: fmt::Arguments) {
        // 必须走 `putln!` 而不是 `console::_write`：后者不补换行，逐例打点会并成一行
        // ——而这一行正是门要断言的东西（`examine.nu` 按行 expect）。第一版写错过，
        // 症状是三例与汇总挤在同一行里。
        crate::putln!("{args}");
    }
}
