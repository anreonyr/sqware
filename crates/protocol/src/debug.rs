//! debug — **一行调试面的嘴**：`debug!(...)` 就是"打一行"。
//!
//! 全树那二十几份逐字相同的 `fn say(msg: &str)` 与各处 `debug::put(&format!(…))` 收成这一支宏
//! ——与 `contract` 那支 `fail_codes!` 同一条口径：**一处定义，谁都能用**。
//!
//! **它只在 debug 构建下有效**：`cfg!(debug_assertions)` 为假时那一格不进 ⇒ release 的机器
//! **不带解读数**（要读数就跑 dev 档，或在档里显式 `debug-assertions = true`）。
//!
//! **为什么是 `if cfg!(…)` 而不是两支 `#[cfg]` 宏**：这一支要在**表达式位置**也用得（今天有
//! `_ => debug!("…"),` 那种写法），而 `#[cfg]` 挂在表达式上不是稳定语法；且这么写 release 也把
//! 这些行**编一遍**（参数照旧被类型检查，"只被读数用到"的变量也不会因整支被 cfg 掉而冒
//! `unused`），只是运行时不落一格。
//!
//! **它住 `protocol`**：那是**唯一同时被 `programs` 与 `harness` 依赖、又已经拖着 `runtime`**
//! 的一层（`contract` 的判据正是"依赖里没有 `runtime`"，碰不得）。

/// 宏的身子。不导出：调用点一律走 [`debug!`]。
#[doc(hidden)]
pub fn put(msg: &str) {
    let _ = runtime::env::debug::put(msg);
}

/// 打一行调试面。**只在 debug 构建下有效**（见本模块头注）。
///
/// ```text
///   debug!("uart: ier=rx at={base:#x}");   // 内联格式参数照旧
///   debug!("rtc: got {got}");              // 与 format! 同一套写法
/// ```
#[macro_export]
macro_rules! debug {
    ($($arg:tt)*) => {
        if cfg!(debug_assertions) {
            $crate::debug::put(&$crate::__format!($($arg)*));
        }
    };
}
