// 健康检查面（health）— 内核自检用例的**身体**：跑在启动后，断言走 `expect!`（失败即 panic）。
//
// **照实记（用户裁定"去掉所有的测试 / 迁移到 embedded-test"）**：这里原先还有 **8 个 `test!`
// 登记块**——展开成 `link.ld` 的 `.tests` 段里的一行，由自研的 `framework::discover()` 取到。
// 那一套连 `kernel/src/framework/`（4 文件 274 行）一起删了；登记改住
// `kernel/tests/embedded.rs` 的 `#[embedded_test::tests] mod`。**本文件只剩用例体**，
// 它们从 `pub(super)` 变成 `pub`，因为那个测试目标是**另一个 crate**。
//
// 八个用例 = 四块子系统各一 + `permit` 四例，都只经公开接口验收，与生产断言分离：
//   · `spare`  —— 后备仓预算（ring 常驻 + 溢出演练闭环）
//   · `pagetable` —— PT 回收（map/unmap 32 轮，无孤儿表、无 double-free）
//   · `stress` —— 分配器压测（block/frame 两档：混合闭环 + 持有-全释放 + 耗尽-反还）
//   · `shell` —— 内核原语外壳（任务/团队/空间）的造-收闭环（逐类净额）
//   · `permit` —— 权柄代数四例（形态位 / 成员投影 / 转发容量 / 取用顺序）
//
// 这一档（`debug`）另有一条**静默**入口（[`run`]）：同样八例、同样顺序，**失败才 panic**。

#[cfg(debug_assertions)]
use core::fmt;

/// 健康检查断言：条件不成立 → 统一报告 + fail-fast（panic）。
///
/// 与 `core::assert!` 的分工：`assert!` 只说"不成立"，本宏说**"哪个量、期望什么、实际
/// 多少"** —— 用例失败时宿主只拿到一行，量值必须自带。
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

/// 输出健康检查结果行。
///
/// 照实记：本函数在**全部配置**下都没有调用者——用例的通过汇报原先走 `cases` 那套
/// `[case] ok` 打点，而那一套已随自研框架删掉（用户裁定"迁移到 embedded-test"），
/// 用例只剩"失败才 panic"。保留是给健康 API 留一个汇报口。
#[cfg(debug_assertions)]
#[allow(dead_code)]
pub(crate) fn report_ok(item: &str, detail: fmt::Arguments) {
    crate::putln!("[health] {item}: ok ({detail})");
}

pub mod pagetable;
pub mod permit;
pub mod shell;
pub mod spare;
pub mod stress;

// ── 用例**登记**不在这里（用户裁定"迁移到 embedded-test"）─────────────────────
//
// 原先这里有 8 个 `crate::test! { "名字" { … } }` 块，靠 `.tests` 段 + `framework::discover()`
// 发现。那一套已删；登记住 `kernel/tests/embedded.rs` 的 `#[embedded_test::tests] mod`——
// **名字与次序跟着搬过去了**，用例体仍是上面那五个模块。

// ── 静默入口（`debug` 档）───────────────────────────────────────────────────
//
// 与 `kernel/tests/embedded.rs` 那一份**同例同序**：这一条不起 QEMU、不用 runner，
// 只要 `debug_assertions` 就在启动期跑一遍（失败才 panic）。故它**不是**测试框架，
// 是内核自己的启动自检。

/// 逐项验收各用例；任一失败 = fail-fast panic → crash scene。
pub fn run() {
    #[cfg(debug_assertions)]
    {
        spare::accept();
        pagetable::pagetable();
        stress::accept();
        shell::accept();
        permit::form();
        permit::members();
        permit::fanout();
        permit::order();
    }
}
