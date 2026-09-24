//! 用例与运行器（**程序侧**那一台）—— **运行时登记**，不碰链接脚本、不用 `cargo test`。
//!
//! # 为什么程序侧不用内核那台"链接期段收集"
//!
//! 内核那台（`kernel/src/framework/`）把用例散进各模块，靠 `.tests` 段 + `KEEP` 收集——因为
//! 那些用例**不住在同一个流程里**，登记样板没人愿意写。程序侧不一样：一台探针的读数本来就在
//! **同一个 `main` 的同一段**里（它刚算完 `is` / `under` / …），就地登记一例只多一行。
//!
//! 于是这一台**两条路都省了**（照实记：这是 pilot 量出来的结论）：
//!
//!   * **官方那台**（`#![feature(custom_test_frameworks)]` + `#[test_case]` + 自定运行器）的
//!     接线**只挂 `--test` 那一档**——实测：同一个 `no_std` 文件编两遍，普通构建里
//!     `warning: function 'run' is never used`，`--test` 那一遍没有这条警告（即运行器接上了）。
//!     而镜像程序是**嵌套 `cargo build` 出来的 bin**，永远不在那一档；要用它得改整条构建管线
//!     （`cargo test --no-run` 的产物落在 `deps/<名字>-<hash>`，再打进 initrd）。
//!   * **段收集**要改 `programs/link.ld`（加 `.cases` + 两个边界符号），而读数还在局部变量里
//!     ——`fn()` 捕不了环境，就得再把读数搬进全局量。
//!
//! 运行时登记把这两件都绕开：`Box<dyn Fn()>` 捕得住环境，链接脚本一个字不动，
//! 构建管线也一个字不动（还是那个嵌套 `cargo build`）。
//!
//! # 用的样子
//!
//! ```ignore
//! let mut suite = cases::Suite::new("probe-rule");
//! suite.case("three_rules_landed", || assert!(made == 3, "made={made}"));
//! suite.case("is_binds_that_identity", || assert_eq!(is, ocall::OK));
//! …
//! suite.run();           // 全过才返回；失败走 panic 通道，域当场死
//! ```
//!
//! # 汇报格式（门只认这几行）
//!
//! ```text
//! [case] <台名>: N cases
//! [case] <台名>: run <名>      ← **开跑前**打（照实记：内核那头"先打 ok 再跑"撒过谎，
//!                                故意注入的失败照样报 ok）
//! [case] <台名>: ok <名>       ← 只有跑完才打
//! [case] <台名>: cases N ok M fail K   ← 全过才有这一行
//! ```
//!
//! **照实记（`<台名>` 那一格是补出来的）**：第一版不带台名，于是两台探针的汇总行**逐字相同**
//! （都恰好 3 例）——门的基线断言因此分不出是哪一台（一台没登记，另一台顶上，断言照绿）。
//!
//! 失败的用例**留不下 `ok`**、也留不下末行：`assert!` 走域内 panic 通道（`programs::entry`
//! 把"那句话 + `file:line:col`"交给内核，内核在自己的出口上打出来）⇒ 门看到的是
//! "`[case] run X` 之后没有 `ok X`、也没有末行"，据此点名**哪一例**。
//!
//! **一次只报一个失败**（`panic = abort`，没有 unwind，域当场死）——这是把判据搬进 SUT 的
//! 代价（今天 soak 是"一轮把所有缺的读数一次报全"），写在 `docs/harness-gate.md` 的取舍那一节。

use alloc::boxed::Box;
use alloc::format;
use alloc::vec::Vec;

use runtime::env::debug;

/// 一沓用例：运行时登记，跑完打印那份协议。
pub struct Suite {
    /// 哪一台的用例（进协议行的头一格——门按它钉**逐台**的基线）。
    who: &'static str,
    cases: Vec<(&'static str, Box<dyn Fn()>)>,
}

impl Suite {
    /// 空的一沓。`who` = 清单名（`probe-rule` / `probe-owner` / …）。
    pub fn new(who: &'static str) -> Suite {
        Suite {
            who,
            cases: Vec::new(),
        }
    }

    /// 登记一例。`name` 是给**人和门**看的名字（它出现在 `[case] run` / `[case] ok` 两行里）。
    ///
    /// 名字用**陈述句**（"这一格该是这样"），不用编号——红的时候那一行就是结论。
    pub fn case(&mut self, name: &'static str, body: impl Fn() + 'static) {
        self.cases.push((name, Box::new(body)));
    }

    /// 跑全部用例：逐例打点，末尾给汇总。**全过才返回。**
    pub fn run(&self) {
        let who = self.who;
        say(&format!("[case] {who}: {} cases", self.cases.len()));
        let mut ok = 0usize;
        for (name, body) in &self.cases {
            say(&format!("[case] {who}: run {name}"));
            body();
            ok += 1;
            say(&format!("[case] {who}: ok {name}"));
        }
        say(&format!(
            "[case] {who}: cases {} ok {ok} fail 0",
            self.cases.len()
        ));
    }
}

/// 打一行（调试面是本域唯一的嘴，与 `echo` / `guest` 用的是同一格）。
fn say(msg: &str) {
    let _ = debug::put(msg);
}
