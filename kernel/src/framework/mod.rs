//! 框架（framework）— 内核内测试框架：**登记在链接期，跑在启动后**。
//!
//! # 为什么不用 `cargo test`（2026-09-24 又量了一遍，四条读数）
//!
//! 本仓的测试对象要**启动后的状态**（帧池、pagemap、页表、`Space` 原语），而 cargo 的
//! 测试模型是"用例跑在 `main` 之前、内核还没起来"。故沿用
//! [os-test-framework](https://docs.rs/os-test-framework) 的**形态**（`test!` 宏 + `Platform`
//! 抽象 + 逐例打点），发现层换成**链接期段收集**。
//!
//! "官方那条路（`custom_test_frameworks`）能不能换掉发现层"——早先只留下一句结论，那一刀把它
//! **量全了**，四条：
//!
//! ```text
//!   ① `[[bin]] harness = false`    cargo **不传 `--test`**（只给 `--cfg test`）⇒ 官方那台
//!                                  机器整个不启动：`cannot find function test_main`（E0425）
//!   ② `[[test]] path = "src/main.rs"`  同一份源码再挂一个测试靶，**照样不传** `--test`
//!   ③ `cargo rustc … -- --test`    这一条**能接上**：生成 `test_main` + 收集 `#[test_case]`，
//!                                  产物仍是 EXEC、入口仍是 `_start`（0x80200000）
//!   ④ 起机实测                      `test_main` 在真机上被调到、返回，启动继续 ✓
//!                                  ——**但只收上来 1 例**（我们那 8 条 `test!` 它看不见），
//!                                  而且**给不出用例名**（`type_name_of_val` = `dyn core::any::Any`）
//! ```
//!
//! ④ 那两句是决定性的：`[case] ok <名字>` 是本仓的协议（门的逐台基线与人读的日志都认它），而官方
//! 那条路**给不了名字** ⇒ 名字表还得自己另备一份；且 `test_main` 只负责**调我们的运行器** ⇒
//! 运行器、逐例打点、`RUNNING`（panic 通道靠它挑出口）、`Status`（通过 = 放行启动）**一样都省
//! 不掉**。真正省下来的只有 `.tests` 那一段（`link.ld` 5 行 + `test!` 13 行 + [`discover`] 14 行），
//! 代价却是：构建从 `cargo build` 改道成 `cargo rustc … -- --test`，产物从稳定的
//! `framework/sqware` 变成 `build/kernel/<hash>/out/sqware`（**跟着环境变**）。
//!
//! ⇒ **不换**。这条账记在这里，免得下次再量一遍。
//!
//! # 一次运行只抓一个失败（是算术，不是缺陷）
//!
//! `panic = "abort"`，没有 unwind：用例里一条 `assert!` 就走到 panic 通道，其后的用例
//! 不跑。所以"运行中汇总全部失败"做不到 —— 用例各自打点，首个失败即停机；`[case]`
//! 那几行是**人工读**的（`crates/gate/tests/examine.rs` 的判据里没有 `[case]`，它只管自己那五条）。
//!
//! # 为什么整块门控、而用例集中在 health/
//!
//! `.tests` 段是 `KEEP(*(.tests))` 之外的普通段：**没有引用时会被链接器丢弃**，
//! `#[used]` 只保证"进了目标文件"，不保证"活过链接"。用例一旦散进各内联模块，段被丢
//! 的症状是**静默零用例** —— 比失败更坏（绿着，什么都没测）。故框架整块由
//! `--features framework` 门控（与上游 `#[cfg(test)]` 同一个理由），用例集中在
//! [`crate::health`]，那里是唯一会产出 `.tests` 行的模块。

mod case;
mod platform;
mod runner;

pub(crate) use case::Case;
pub(crate) use platform::{Kernel, Platform, Status};

/// 跑全部用例（`boot` 期入口；见 [`runner::run`]）。
pub(crate) use runner::{case_failed, run};

/// 发现全部用例：链接脚本给出的段边界 → 切片。
///
/// 边界符号由 `link.ld` 定义（与 `_kernel_base` / `_rodata_start` 同一手法）。
/// 空段是合法结果（区间长度 0），故本函数不 panic —— 但它也**报不出**"零用例"这件事，
/// 那是宿主的事（见模块头注）。
fn discover() -> &'static [Case] {
    unsafe extern "C" {
        static __tests_start: u8;
        static __tests_end: u8;
    }
    // SAFETY: 两个符号由链接脚本定义在 `.tests` 段两侧，区间即该段；`Case` 是
    // `repr(C)` 的 POD（`&'static str` + 裸函数指针），而段在 link.ld 里按 8 对齐。
    unsafe {
        let start = core::ptr::addr_of!(__tests_start) as *const Case;
        let end = core::ptr::addr_of!(__tests_end) as *const Case;
        let n = (end as usize - start as usize) / size_of::<Case>();
        core::slice::from_raw_parts(start, n)
    }
}

/// 登记一个用例。
///
/// 展开成 `.tests` 段里的一行（`#[used]` 防优化掉，`link_section` 定位置）。静态量
/// 不可同名，故用匿名 `const` 给出唯一作用域 —— 调用方**不必**提供标识符（上游
/// `test!` 用 `fn _test()` 免掉这件事：函数可重名，静态量不行）。
#[macro_export]
macro_rules! test {
    ($name:literal $body:block) => {
        const _: () = {
            #[used]
            #[unsafe(link_section = ".tests")]
            static CASE: $crate::framework::Case = $crate::framework::Case {
                name: $name,
                body: || $body,
            };
        };
    };
}

/// 当前在跑的用例（空 = 没有用例在跑）。
///
/// **一个字的发布**：panic 通道靠它挑出口，而"哪一例"必须要么是完整的一例、要么是
/// 空 —— 长度 + 指针两个原子会给出第三种状态（新长度配旧指针）。发布 `Case` 指针
/// 让那个状态**不可表达**。
static RUNNING: core::sync::atomic::AtomicPtr<Case> =
    core::sync::atomic::AtomicPtr::new(core::ptr::null_mut());

/// 置当前用例（[`runner`] 每例**开跑前**调用）。
pub(super) fn set_running(case: &'static Case) {
    use core::sync::atomic::Ordering;
    RUNNING.store(core::ptr::from_ref(case).cast_mut(), Ordering::Relaxed);
}

/// 归零：全部用例跑完（[`runner`] 返回前**唯一**一处）。此后 panic 走崩溃转储。
pub(super) fn clear_running() {
    use core::sync::atomic::Ordering;
    RUNNING.store(core::ptr::null_mut(), Ordering::Relaxed);
}

/// 当前用例名；没有用例在跑（未开跑 / 已跑完）时是空串。
///
/// 崩溃路径的读点：panic 通道据此挑"用例失败"还是"崩溃现场"。
pub(crate) fn running() -> &'static str {
    use core::sync::atomic::Ordering;
    let ptr = RUNNING.load(Ordering::Relaxed);
    if ptr.is_null() {
        return "";
    }
    // SAFETY: `RUNNING` 只由 `set_running` / `clear_running` 写，写进去的是
    // `.tests` 段里那一行 `Case` 的 `&'static` 引用；段在链接期成形、运行期不变
    // ⇒ 指针与它指的 `name` 在整个运行期内都有效。
    unsafe { (*ptr).name }
}
