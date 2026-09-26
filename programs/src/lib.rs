#![no_std]
//! programs — 镜像里装载的程序集合（**每个程序一份 `main.rs`**，就住在它那一片模块的目录里）。
//!
//! **分档按特权级**（唯一声明处：`plan::assembly::ALL` 里这一行的 `kind`）：[`root`] 与 [`system`]
//! 是 S 态那一档（引导域 / 编排域），[`user`] 是 U 态那一档（今天**只剩 `echo`**：调试回显；
//! 六位试客搬去了 `harness`，见下面那条照实记）。
//!
//! **照实记（iii：三个服务不再是程序）**：持树者 / 身份 / 结盟原先各是一个 bin、一个域（身份与
//! 结盟还是 **U 态**）。现在它们与编排者**共用一份字节**（`prog-system`），在**编排域自己的域**
//! 里各占一枚线程 ⇒ 本 crate 的产品档从 9 个 bin 降到 **6** 个；而"特权级唯一声明处"这条不变
//! ——那一份字节在装配单上是 `Supervisor`，三枚内件随域（同域四枚线程共享一张页表，这是代价）。
//! **按角色分的那一族不分档**：**驱动整块留在 [`driver`]**——成员的特权级仍各自在
//! 装配单 声明（今天三台驱动都是 **U 态**；见 [`driver`] 的头注）。
//!
//! **测具不在这里**（照实记：用户裁定"测试和程序分开"）：探针（`probe-*`）与压测台
//! （`rig` / `load` / `beat` / `again` / `group` 与它们的受害者）整体搬去了隔壁那个 crate
//! **`harness`**——它们只借这里的一件共享入口（`extern crate programs;` ⇒ [`entry`] 的
//! `_start`）。哪几台进哪张镜像，仍只在 `plan::assembly::ALL` 每行的 `scenes` 里声明。
//!
//! **表归主人**：四张硬件需求单在**收方**（今天住 `plan::assembly`——装配单本就要把那一格
//! 摆出来，故与它同层：本域要哪几枚、落到它自己那张表的第几格）；boot 的两块账在
//! **引导域**（`root/boot.rs`：只有它读得到）——装配者只是 `use` 它们，不另抄一份。
//!
//! 内含之后**共用件只剩三枚**：`entry`（`_start` + panic 处理，每个程序共用）、
//! [`service`]（那台装配机器，两个装配者 `root` / `system` 共用）与
//! [`driver::assemble`]（**客侧**那台机器，三台驱动与房客共用）。
//!
//! **判据是「谁在说话」**：从外面找上某份协议的人用的一切（正文、判定、帧、**客侧那几手**）
//! 住 `crates/protocol`；那位协议的**实现方**（谁循环、谁记账、谁起线程、谁调内核）跟着
//! **用它那个程序所在的档**走——于是本 crate 顶层就是**那几个程序族**：[`root`]（引导域：
//! 入口 ＋ [`root::boot`] ＋ [`root::supply`]）、[`system`]（编排域：入口 ＋ 装配单 ＋ 机器 ＋
//! 住本域的那几枚线程——**板**、**持树者**、**名册**、**盟册**）、[`driver`]（三台驱动 ＋
//! 客侧那台机器）、[`user`]（回显）。
//!
//! **照实记（这一层壳撤了两次）**：`principal` / `coalition` 两处是先补进"实现方"那一行的
//! （它们的实现方一直住在同一处，那一行从前漏了它们）；随后用户裁定「搬」，三枚内件连同板
//! 一起收进 `system/`；**再一刀**用户裁定"**把 `root` 移出 `supervisor`**"——于是
//! `supervisor/` 这层壳整个撤掉：它原来的判据是"**S 态那一档**"（唯一声明处是装配单的 `kind`），
//! 而 `root` 与 `system` **都是 S 态** ⇒ 撤了 `root` 之后它只剩 `system` 一个孩子，没有可包的
//! 东西。`root/`、`system/` 与共用的 `service.rs` 从此都摆在本文件这一层。
//! **一句话**：目录与那条判据是同一句话的两种写法，写完就没什么可包的了。
//!
//! 共享的"干活"住 `src/` 顶层与 `driver/` 本级：一份源码编一次，各程序只 `use`，
//! 不再有 `#[path]` 复制与"另一半是死码"的 `#[allow(dead_code)]`。
//!
//! **目录即程序**：每个程序的入口（`main.rs`）与它那一片模块同住一个目录——`system/`
//! 里既有实现也有 `main.rs`，`root/`、`driver/router/`、`driver/uart/`、
//! `driver/rtc/` 同理；`bin/` 那一层撤了。
//!
//! **照实记（iii 之后那三枚不再有 `main.rs`；现在它们住 `system/` 之下）**：持树者 / 名册 /
//! 盟册的入口退成**角色体**——它们的 `server::serve()` 由 `system/main.rs` 的 `main` 按
//! [`Role`](service::Role) 分派（一枚 ELF 只能有一处 `#[entry]` ⇒ 四枚线程共用同一
//! 个入口，靠 `Spawn` 那一格 `args` 分开）；目录随之收进 `system/{operator,principal,
//! coalition}/`——与 `board/` 同一条判据（**谁住编排域，谁住 `system/` 之下**）。
//!
//! **设备侧同理**：谁要读设备，谁的目录里放自己的设备模块（`driver/router/` 下的 `plic.rs`、
//! `driver/uart/` 下的 `uart.rs`、`driver/rtc/` 下的 `rtc.rs`）——**设备语义各带各的，装配契约才
//! 共享**。
//!
//! 其余驱动侧（名字→线号 / 终端渲染）随旧树一起清了（tag `proto-v1-baseline`），
//! 需要时按新形状写——**不从那一套搬**。
//!
//! 今天产品这一档有**六个**程序——**恰好是产品镜像那 6 条**：
//!
//! ```text
//!   U 态  prog-echo    调试回显（产品镜像里排最后一条，编排域等它退场才收场）
//!         prog-router / prog-uart / prog-rtc      三台驱动
//!   S 态  prog-root / prog-system                  引导域 / 编排域
//! ```
//!
//! **另一档不占条数**：`prog-system` 那**一份字节**里住着四个角色（编排者 + 持树者 + 身份 +
//! 结盟），后三个是**编排域里的线程**（`scenario.rs` 的 `INNER`）——它们没有 bin、没有自己的域。
//!
//! **另 22 台测具**住 `harness`（**不进产品镜像的一切**：探针 6 + 试客 6 + 压测台 10）。
//!
//! **照实记（六位试客是第二刀搬走的）**：`guest` / `passer` / `lodger` / `sleeper` / `subject` /
//! `member` 原先住这里（`user/` 那一间）——它们量的是**服务**，去掉机器照转（产品镜像实测过），
//! 于是按用户裁定 **甲** 搬去 `harness`。这条边界因此成了一句可查的话：**本 crate 里的 6 个 bin
//! 就是产品镜像那 6 条**。
//! **特权级不在这里声明**——那一格在 `plan::assembly::ALL` 里这一行的 `kind`。

extern crate alloc;

pub mod driver;
pub mod entry;
pub mod root;
pub mod service;
pub mod system;
pub mod user;

// 出口那一套的转发：生成物（`entry_<路径>.rs`）里写的是 `programs::…`，各 bin 的
// `main` 返回类型也写 `programs::Report` / `programs::Exit`，故这几个名字得在 crate 根上
// 够得着。**本体在 `runtime::core::exit`**（判据是"内核读不读它"，见那边的照实记）——
// 这里只是转发，故 `use programs::{Exit, Report}` 这一形照旧。
pub use runtime::core::exit::{Exit, Report};

pub use env::Reason;
/// 入口那一手（过程宏）：bin 里写 `#[entry] fn main() …`，展开与符号名见那个 crate。
pub use mold::entry;
