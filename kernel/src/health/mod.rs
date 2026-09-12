// 健康检查面（health）— 内核自检用例：**登记在 `--features framework` 档，跑在启动后**。
//
// 这里是**唯一**会产出 `.tests` 段登记行的模块（框架整块门控的理由见 `framework/mod.rs`
// 头注）：`test!` 块展开成段里的一行，`framework::discover()` 在启动后取到它们。
//
// 三个用例对应三块子系统，都只经公开接口验收，与生产断言分离：
//   · `spare`  —— 后备仓预算（ring 常驻 + 溢出演练闭环）
//   · `pagetable` —— PT 回收（map/unmap 32 轮，无孤儿表、无 double-free）
//   · `stress` —— 分配器压测（block/frame 两档：混合闭环 + 持有-全释放 + 耗尽-反还）
//
// 非框架档（默认 / audit / harden）保留一条 `debug_assertions` 的旧入口：三个用例
// 仍会在 debug 构建里跑一次，行为与框架落地前逐字相同。

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

/// 输出健康检查结果行（旧档的通过汇报；框架档改由 `[case] ok <name>` 打点）。
#[cfg_attr(feature = "framework", allow(dead_code))]
pub(crate) fn report_ok(item: &str, detail: fmt::Arguments) {
    crate::putln!("[health] {item}: ok ({detail})");
}

pub mod pagetable;
pub mod spare;
pub mod stress;

// ── 用例登记（`--features framework`）────────────────────────────────────────
//
// 名字是**给失败的人看的**：带子系统与"测什么"，因为首个失败即停机，宿主看到的只是
// `[case] ok <name>` 序列的截断处加一条 `[case] FAIL <name>`。
#[cfg(feature = "framework")]
crate::test! {
    "spare: 后备仓预算（ring 常驻 + 溢出演练闭环）" {
        spare::accept();
    }
}

#[cfg(all(feature = "framework", any(debug_assertions, feature = "framework")))]
crate::test! {
    "pagetable: PT 回收（32 轮 map/unmap 无孤儿表）" {
        pagetable::pagetable();
    }
}

#[cfg(feature = "framework")]
crate::test! {
    "stress: 分配器压测（block/frame 混合 + 持有 + 耗尽反还）" {
        stress::accept();
    }
}

#[cfg(feature = "framework")]
crate::test! {
    "chain: 自由链表↔pagemeta 不背离（表说空闲 ⇔ 真在链上）" {
        stress::chain();
    }
}

// ── 旧档入口（非 framework）─────────────────────────────────────────────────
//
// 与框架档互斥：`boot.rs` 按 feature 二选一（同一位置、同一时点）。
// `debug_assertions` 档才有实体，release 下是空体（与框架落地前一致）。

/// 逐项验收三个用例；任一失败 = fail-fast panic → crash scene。
pub fn run() {
    #[cfg(all(not(feature = "framework"), debug_assertions))]
    {
        spare::accept();
        pagetable::pagetable();
        stress::accept();
    }
}
