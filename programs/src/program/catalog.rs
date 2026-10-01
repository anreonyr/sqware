//! catalog — **装配表那一块**：那一份份声明（`#[path]` 拉进来的）与 [`PROGRAMS`] 这张单。
//!
//! **照实记（层六·2 第一块：从 `mod.rs` 切出来）**：这一块从 `program/mod.rs` 的 588–681 行整体
//! 搬到同名目录下的 `catalog.rs`——**它自足**（除上面的类型定义外不引别的），且**这一段是给人扫的**
//! （`#[rustfmt::skip]` 那一张表）。模块名不变：`mod.rs` 里 `pub use catalog::*;` 把这一块**原样
//! 摆回 `crate::program` 那个名字空间** ⇒ 全仓引用一处都不用动。
//!
//! **它要从上面借两样**：[`Program`] 与 [`SCENE_UNIT`]（那一台目标单元）——故一行 `use super::{…}`。

use super::{Program, SCENE_UNIT};

// ── 每台自己的声明 ───────────────────────────────────────────────────
//
// **它们不在这份文件的自然模块树里**：那些目录（`driver/uart/`、`system/operator/`…）都拖着
// runtime / protocol 代码，`crates/image` 进不去。故只由 [`PROGRAMS`] 这一处按 `#[path]` 拉
// 进来一次——**唯一的声明点**。

#[path = "../user/canonical/program.rs"]
pub mod canonical;
#[path = "../service/coalition/program.rs"]
pub mod coalition;
/// harness 那 23 台（**测具**）：它们的身子住隔壁那个 crate，而其中 13 台**由编排域起**
/// ——编排域要按 `order` / 存在信号 / `bind` / `died` 起它们，故声明必须由本 crate 编译。
/// `harness` 依赖 `programs`，反向不可能。故这一族的声明住这里（一份，不拆 23 份：
/// "紧挨着身子"对身子不在本 crate 的那几台本来就不成立，不假装）。
#[path = "../decl/harness.rs"]
pub mod harness;
#[path = "../service/hub/program.rs"]
pub mod hub;
#[path = "../service/operator/program.rs"]
pub mod operator;
#[path = "../service/principal/program.rs"]
pub mod principal;
#[path = "../root/program.rs"]
pub mod root;
#[path = "../driver/router/program.rs"]
pub mod router;
#[path = "../driver/rtc/program.rs"]
pub mod rtc;
#[path = "../system/program.rs"]
pub mod system;
#[path = "../driver/uart/program.rs"]
pub mod uart;

/// **装配表**：镜像里可能有的全部程序。**次序是硬事实**——它就是装载次序（`ROOT_OFFSET`
/// 按位次算），且各景按 [`Program::wanted_by`] 过滤 ⇒ 加一台要想清楚放哪。
///
/// **本表只有引用**：每一台的声明都在它自己那份 `program.rs` 里，这里不再写第二遍。
///
/// **本表一行一台，`rustfmt` 请绕开**：默认那套会把每台摊成十几行，于是"哪几台进哪张镜像"
/// 就没法一眼扫完——而这张表**就是**给人扫的。
#[rustfmt::skip]
pub const PROGRAMS: &[&Program] = &[
    &root::PROGRAM,
    // 四枚服务（持树者 / 名册 / 盟册 / 设备账）：各自一个 bin、一个域，与其他每一台同一条 `mint` 路。
    &operator::PROGRAM,
    &principal::PROGRAM,
    &coalition::PROGRAM,
    &hub::PROGRAM,
    &canonical::PROGRAM,
    // 客人 / 过客 / 房客：量服务用的（去掉机器照转）。
    &harness::GUEST,
    &harness::PASSER,
    &harness::LODGER,
    // 三台驱动。
    &router::PROGRAM,
    &uart::PROGRAM,
    &rtc::PROGRAM,
    &harness::SLEEPER,
    &harness::SUBJECT,
    &harness::MEMBER,
    &system::PROGRAM,
    &harness::PROBE_DENIED,
    &harness::PROBE_OWNER,
    &harness::PROBE_RULE,
    &harness::PROBE_RULE_OTHER,
    &harness::PROBE_LEASE,
    &harness::PROBE_BOUND,
    // 控制面那位真客人（`/svc/sys/control/state` 那一格）：**排在 `canonical` 之前**，见它自己那份声明。
    &harness::PROBE_CONTROL,
    // 操作面那一族（`/svc/sys/operator/{part,land,…}`）：**两位一对**——`gate` 拿控制面会话把七格
    // 验一遍并取回那一枚入口、铺好试验场；`land` 只持 `land` 一位（时序见各自那份声明）。
    &harness::PROBE_OPERATOR_GATE,
    &harness::PROBE_OPERATOR_LAND,
    // 压测台与它们的受害者（整台替换引导镜像）。
    &harness::CHURN,
    &harness::RIG,
    &harness::BUSY,
    &harness::PARK,
    &harness::HANG,
    &harness::LOAD,
    &harness::BEAT,
    &harness::AGAIN,
    &harness::WAITER,
    &harness::GROUP,
    // **这一趟装配本身**（[`SCENE_UNIT`]）：一个[目标单元](Kind::Target)——没有身子、不进任何
    // 镜像（宿主那一侧按 `wanted_by` 与 `kind` 两格滤掉），它在这张表里只为"这一趟走完"给一个落点。
    &SCENE_UNIT,
];

/// 清单条数上界与注册表条数必须相容（见 [`env::manifest::MAX_PROGRAMS`] 的头注）。
///
/// **这一条就是从前那个"数出来的数"的替身**：加一台超过上界 ⇒ 当场编不过，不可能静默卡住。
const _: () = assert!(PROGRAMS.len() <= env::manifest::MAX_PROGRAMS);

