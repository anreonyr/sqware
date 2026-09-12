//! 运行器（framework/runner.rs）— 逐例跑、逐例打点、末尾给结局。
//!
//! # 汇报格式（门要断言的唯一新 marker）
//!
//! ```text
//! [case] N cases
//! [case] ok <name>
//! [case] FAIL <name> — <原因> @ <调用点>
//! [case] cases N ok M fail K
//! ```
//!
//! 逐例一行是给**人**看的（哪一例、哪一步）；末行汇总行是给**门**看的（机器可判定）。
//!
//! # 失败即停机，故没有"继续跑下一例"
//!
//! 用例体的 `assert!` 直接走 panic 通道（`halt.rs` 的 `#[cfg(feature = "framework")]`
//! 短路分支），由它报"哪一例 + 原因"再停机。本模块因此**看不到**失败 —— 它只负责把
//! 通过的例子数清楚，并在全部通过时给 `Pass`。见模块头注的算术。

use super::{Kernel, Platform, Status, discover, set_running};

/// 跑全部用例并给出结局。
///
/// 调用点：`boot` 期（`kernel/src/boot.rs`），在调度器就绪之后、`spawn_root` 之前 ——
/// 与既往 `health::run()` 同一位置。故用例**没有 shell、没有装槽**，只有单核与早启动期
/// 设施（`putln!`、块/frame 分配器、页表树、`Space` 原语、`fence`）。
///
/// # 通过 = **放行启动**，不是停机
///
/// 全部通过就正常返回，`boot` 继续往下走（`spawn_root` → shell）。测试档要在**同一趟**
/// 里接着跑验收门那 15 步 shell 序列，停机会把它们全掐掉 —— 这一条第一版写错过
/// （`Platform::finish` 走了停机自环，症状是 shell 再也起不来）。停机只发生在失败：
/// 那是 panic 通道（[`case_failed`]）的事。
pub(crate) fn run(platform: &dyn Platform) -> Status {
    let cases = discover();
    platform.print(format_args!("[case] {} cases", cases.len()));
    // **先跑后报**，顺序不能反：`ok` 行是用例跑完之后才允许说出口的话。第一版写成
    // "先打 ok 再跑"，于是一个**故意注入的失败**照样被报成 ok（实测：断言改成
    // `occupied >= ring + 1` 后仍打 `[case] ok spare: …`）——报告撒谎比不报更坏。
    // 现在失败的用例**留不下完整的一行**，缺席即失败，汇总行也随之自洽。
    let mut done = 0usize;
    for case in cases {
        set_running(case.name);
        (case.body)();
        done += 1;
        platform.print(format_args!("[case] ok {}", case.name));
    }
    platform.print(format_args!(
        "[case] cases {} ok {done} fail 0",
        cases.len()
    ));
    Status::Pass
}

/// 崩溃路径的出口：某一例 panic 了 → 报"哪一例 + 原因"再停机。
///
/// 由 `halt.rs` 的 panic 通道在 `--features framework` 下调用（那里是唯一的
/// `#[panic_handler]`，本模块**不**再声明一个）。
///
/// 为什么在这里而不是 halt 里：`halt` 认识崩溃转储（寄存器/栈/页表），本模块认识
/// "用例"。
///
/// # 但要**保住位置**
///
/// 第一版只打 message，于是护栏在启动期抓到缺陷时，读数只有"哪一例、断言说了什么"
/// —— 而 `index 4078 / base 4076` 这类信息**指不出是哪个调用点**。`PanicInfo::location()`
/// 就是那个调用点（`check_*` 的 panic 都在调用者的行上），它比整份寄存器转储有用得多。
/// 教训：省掉转储是对的，省掉位置不是。
pub(crate) fn case_failed(info: &core::panic::PanicInfo) -> ! {
    let kernel = Kernel;
    match info.location() {
        Some(loc) => kernel.print(format_args!(
            "[case] FAIL {} — {}（{}:{}）",
            super::running(),
            info.message(),
            loc.file(),
            loc.line()
        )),
        None => kernel.print(format_args!(
            "[case] FAIL {} — {}",
            super::running(),
            info.message()
        )),
    }
    // 失败即停机：干净自退（宿主按 qemu 自退/退出码判定），不走 `abort`。
    crate::runtime::diagnose::halt::halt_loop()
}
