#![no_std]
//! plan —— **装机的账**：哪几台程序、哪一枚门闩、哪一段区，以及它们过开机那一刻的字节。
//!
//! **判据（这个 crate 的存在理由）**：这里全是**两侧都要读的"账"**——打包器（**宿主**）
//! 读它决定"哪几台进哪张镜像"，内核与引导域、编排域（**riscv**）读它认设备、起程序、
//! 取两块借映块。而它**一处也不碰运行时代码**：不出现任何 `*Call` / 域词表 / `FailCode` /
//! `trap` / `runtime` / 内核类型——**它只认表与字节**。
//!
//! 由此它从 `env` 里分出来：`env` 是**过线的**（U↔S 的线格式 + 门闩权限），`plan` 是
//! **装机的**。两者都必须在宿主与 riscv 上编得过，但那是"**编译得了**"这一条共同点，
//! 不是"同一件事"——它们从前同住一个 crate，理由只是前者。方向单向：`plan → env`。
//!
//! **模块与它从前在 `env::wire` 下的叶名一一对应**（`plan::key` → `plan::key`，
//! `plan::assembly` → `plan::assembly`），故那一刀在全仓是**前缀互换**：
//!
//! ```text
//!   assembly.rs   装配单：哪几台进哪张镜像（`ALL` / `ENTRY` / `Plan` / `Row` / `Spot` / `E_*`）
//!   supply.rs     供给那一族的词汇（`Need` / `Want` / `Kind` / `At` / `class_block`）
//!   key.rs        坐标：一条供给记录里"它是哪一件"的那一格
//!   args.rs       启动参数布局（boot → root 的**入口账**）
//!   pair.rs       配对块（boot → root 的**门闩账**）
//!   manifest.rs   initrd 清单（boot → root 的**程序账**；写侧是打包器）
//! ```
//!
//! **照实记（`wire/` 那层壳为什么没跟过来）**：它原先的全部作用，就是把"不是 wire 的东西"
//! 塞进一个 `wire` 名字底下——`pair.rs` 自己的头注就写着"**这不是 envcall 载荷**，而是
//! 启动期借映块的线格式"，`args.rs`/`manifest.rs`/`supply.rs` 同理。这里按**它是什么东西**
//! 摆：三笔开机账（`args` / `pair` / `manifest`）、一份坐标（`key`）、一张装配单（`assembly`）
//! 与它的词汇（`supply`）。
//!
//! **留在 `env` 的那几件**（它们看着像"装机的"，其实是过线的）：`ProgramKind`（`Build`
//! 的载荷字段，有 `Wire` impl）、`Name` / `NAME_LEN`（内核要按 `Name` 的口径校验孔记号，
//! 而内核不依赖 `protocol`）、`Permission` / `Access` / `Policy`。故本 crate 反过来引它们。

extern crate alloc;

pub mod args;
pub mod assembly;
pub mod key;
pub mod manifest;
pub mod pair;
pub mod supply;

// **面**：与从前 `env::wire` 那边同一口径——可命名的类型一律转出，故 `plan::Key` 与
// `plan::key::Key` 两条路都在；`args` 那五个裸常量仍留自己的模块（名字太通用，见 `env::wire`
// 那条同源的照实记），`assembly` 与 `manifest` 亦然（`plan::assembly::ALL` 这一形照旧）。
pub use key::{KEY_LEN, Key};
pub use pair::{PAIR_LEN, Pair};
pub use supply::{At, Kind, Need, Want, class_block};
