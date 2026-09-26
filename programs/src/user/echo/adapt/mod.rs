//! echo::adapt — **适配（壳）**：碰内核、碰板、碰树、碰孔的那一半。
//!
//! ```text
//!   board.rs    1  ：上板报到（板据此看得见本域的死；挂不上照旧回显）
//!   console.rs  2–3：找控制台 `FIND /device/uart`（有界重试，取回那枚孔）
//!   tree.rs     4–5：上树两趟——落自己那块牌子（五步）＋ 一串（列号 / 翻名 / 列目录 / 问空号）
//!   echo.rs     6  ：回显那一圈——`pull` 一批 → 交给 `core::line` → 逐行写回（**壳**）
//! ```
//!
//! **这一半由 bin 自己 `mod`**（不编进 lib）：本域没有客人、也不被谁 `use`（见 `user/echo/mod.rs`）。
//! 回显那一圈的**壳**在这里、**规则**在 `core/`：壳里因此没有一条语义判定，只有次序与转发。
//!
//! **本域不立 `Fail` 类型**：它只有一种失败（一次往返都做不成 ⇒ 找不到控制台），`main` 的返回
//! 类型直接用 `Reason`——一格不值得一个类型（与三台驱动那种"死法要分五步"的情形不同）。

pub mod board;
pub mod console;
pub mod echo;
pub mod tree;

/// 本域挂在板上的名字（板按它分人；编排域表里那一条也叫这个）。
pub const ME: &str = "echo";

/// 要找的那位服务在树上的名字：**控制台**（`/device/uart`——名字用服务名）。
pub const WANT: &str = "uart";

/// 等板 / 等树 / 找一趟控制台的总上限（毫秒）。**必须有界**：对面死在头几步时本域不能陪着挂死。
pub const MS: usize = 1000;

/// 找不到就再问一次的间隔（毫秒）：门牌是驱动落的，本域可能比它先起。
pub const RETRY_MS: usize = 1;

/// 没搭上（找不到控制台）：一次往返都做不成 ⇒ 报这一格退场。
pub const E_NO_CONSOLE: env::Reason = 1;
