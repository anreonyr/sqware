// 健康检查面（health）— 系统自检（out-of-path 验收），与生产路径分离
//
// 与「护栏层（fence，in-path 运行时不变量检查）」相对：health 是**独立验收用例**，
// boot 时一次性调用，验证功能行为（而非功能路径上的自我证明）。任一检查失败 =
// fail-fast panic → crash scene（halt 转储），与生产断言分离——本面不混入
// 任何实现/护栏代码，只经公开接口验收。
//
// 分层（in-path / out-of-path）：
//   功能   memory/allocator/{block,frame,hybrid,...} — 实现本身
//   护栏   memory/allocator/fence/*                    — in-path 检查
//   健康   health/*                                    — out-of-path 验收（本面）
//
// 检查项逐一注册为模块函数，`boot::init` 在 spawn 用户任务前调用。

use core::fmt;

/// health 断言：条件不成立 → 统一报告并 fail-fast（panic → crash scene）。
///
/// 与生产断言（debug_assert/checker/fence）分离：本宏只用于验收用例，报告带
/// `[health]` 前缀与调用点。失败即 halt——boot 验收任一失败，后续结果已无意义。
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

