// 健康检查面（health）— out-of-path 系统自检验收：独立验收用例，任一检查
// 失败 = fail-fast panic → crash scene，与生产断言分离，只经公开接口验收。

use core::fmt;

/// health 断言：条件不成立 → 统一报告并 fail-fast（panic → crash scene）。
///
/// 与生产断言分离：本宏只用于验收用例，报告带 `[health]` 前缀与调用点。
/// 失败即 fail-fast——任一检查失败，后续结果已无意义。
#[macro_export]
macro_rules! expect {
    ($cond:expr, $($arg:tt)*) => {
        if !$cond {
            panic!("[health] {}", format_args!($($arg)*));
        }
    };
    ($cond:expr) => {
        $crate::expect!($cond, "expectation failed")
    };
}

/// 输出健康检查结果行（验收通过汇报；putln! 直写，不分配）。
#[allow(unused)]
pub(crate) fn report_ok(item: &str, detail: fmt::Arguments) {
    crate::putln!("[health] {item}: ok ({detail})");
}

pub mod pagetable;
pub mod spare;
pub mod stress;

/// 健康检查总入口，逐项验收：spare / pagetable / stress 均为 debug-only（任一失败
/// = fail-fast panic → crash scene；与生产断言分离）。
pub fn run() {
    #[cfg(debug_assertions)]
    spare::accept();
    #[cfg(debug_assertions)]
    pagetable::pagetable();
    #[cfg(debug_assertions)]
    stress::accept();
}
