//! 框架（framework）— 内核内测试框架：**登记在链接期，跑在启动后**。
//!
//! # 为什么不用 `cargo test`
//!
//! 本仓的测试对象要**启动后的状态**（帧池、pagemap、页表、`Space` 原语），而 cargo 的
//! 测试模型是"用例跑在 `main` 之前、内核还没起来"。另有一条实测：`#[reexport_test_
//! harness_main]` 在**非 `--test` 构建里什么都不生成**（`cannot find function
//! test_main`）⇒ 不跑 `cargo test`，上游那套发现层就拿不到。
//!
//! 故沿用 [os-test-framework](https://docs.rs/os-test-framework) 的**形态**
//! （`test!` 宏 + `Platform` 抽象 + 逐例打点），发现层换成链接期段收集。
//!
//! # 一次运行只抓一个失败（是算术，不是缺陷）
//!
//! `panic = "abort"`，没有 unwind：用例里一条 `assert!` 就走到 panic 通道，其后的用例
//! 不跑。所以"运行中汇总全部失败"做不到 —— 用例各自打点，首个失败即停机，宿主
//! （`scripts/examine.nu`）读的正是那一行。
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
/// 边界符号由 `link.ld` 定义（与 `_kernel_start` / `_rodata_start` 同一手法）。
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

/// 当前用例名（panic 通道报"哪一例"用）。
///
/// 存成原子指针而非 `&'static str`：静态量里没有字符串切片的字面形态，而
/// `from_utf8_unchecked(ptr, len)` 在**常量上下文**里不是 `const fn`。指针 + 长度分
/// 两处存，读点拼回。
///
/// **不还原**：首个 panic 即停机，没有"下一例"需要干净的名字。
static RUNNING: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
/// 当前用例名的长度（与 [`RUNNING`] 配对）。
static RUNNING_LEN: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

/// 置当前用例名（[`runner`] 每例前调用）。
pub(super) fn set_running(name: &'static str) {
    use core::sync::atomic::Ordering;
    RUNNING_LEN.store(name.len(), Ordering::Relaxed);
    RUNNING.store(name.as_ptr() as usize, Ordering::Relaxed);
}

/// 当前用例名；未开跑（或已跑完）时是空串。
///
/// 崩溃路径的读点（panic 通道报"哪一例失败"）。
pub(crate) fn running() -> &'static str {
    use core::sync::atomic::Ordering;
    let len = RUNNING_LEN.load(Ordering::Relaxed);
    let ptr = RUNNING.load(Ordering::Relaxed) as *const u8;
    if len == 0 || ptr.is_null() {
        return "";
    }
    // SAFETY: `RUNNING`/`RUNNING_LEN` 只由 `set_running` 成对写入，而它收到的
    // `name` 是 `&'static str`（用例名来自 `test!` 的字符串字面量）⇒ 指针与长度
    // 在整个运行期内有效且自洽。
    unsafe { core::str::from_utf8_unchecked(core::slice::from_raw_parts(ptr, len)) }
}
