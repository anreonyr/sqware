//! echo::adapt — **适配（壳）**：碰内核、碰树、碰孔的那一半。
//!
//! ```text
//!   console.rs  2–3：找控制台 `/device/uart`（"再问一次"那一圈在 `operator::entry_of`）
//!   tree.rs     4–5：上树两趟——落自己那块牌子（五步）＋ 一串（列号 / 翻名 / 列目录 / 问空号）
//!   echo.rs     6  ：回显那一圈——`pull` 一批 → 交给 `core::line` → 逐行写回（**壳**）
//! ```
//!
//! **照实记（第 1 步与开局那两手已不住本目录）**：上板报到只剩 `main` 里那三行
//! （`Session::open(sire, board::BERTH, …)` ＋ `board::enroll`），原 `board.rs` 随之退场；
//! 开会话那一手住 [`protocol::communication::session`]（**地板**：只认孔与路），"名字 → 号 →
//! 入口"住 [`operator::entry_of`]（树那一族的客手）——本域只声明"往哪找"（目录
//! [`protocol::driver::DIR`]、名字 [`WANT`]）。
//!
//! **这一半由 bin 自己 `mod`**（不编进 lib）：本域没有客人、也不被谁 `use`（见 `user/echo/mod.rs`）。
//! 回显那一圈的**壳**在这里、**规则**在 `core/`：壳里因此没有一条语义判定，只有次序与转发。
//!
//! **本域不立 `Fail` 类型**：它只有一种失败（一次往返都做不成 ⇒ 找不到控制台），`main` 的返回
//! 类型直接用 `Reason`——一格不值得一个类型（与三台驱动那种"死法要分五步"的情形不同）。

pub mod console;
pub mod echo;
pub mod tree;

/// 本域挂在板上的名字（板按它分人；编排域表里那一条也叫这个）。
pub const ME: &str = "echo";

/// 要找的那位服务在树上的名字：**控制台**（`/device/uart`——名字用服务名）。
pub const WANT: &str = "uart";

/// 等板 / 等树 / 找一趟控制台的总上限（毫秒）。**必须有界**：对面死在头几步时本域不能陪着挂死。
pub const MS: usize = 1000;

/// 没搭上（找不到控制台）：一次往返都做不成 ⇒ 报这一格退场。
pub const E_NO_CONSOLE: env::Reason = 1;
