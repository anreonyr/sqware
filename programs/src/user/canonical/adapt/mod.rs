//! canonical::adapt — **适配（壳）**：碰内核、碰树、碰孔的那一半。
//! ```text
//!   console.rs   找控制台 `/svc/drv/uart/{rx,tx}`（"再问一次"那一圈在 `Face::entry_of`）
//!   terminal.rs  那一圈：轮转（读口收干净 → 写口就绪才推）＋ 把行规程的回显推给写口
//! ```
//! **这一半由 bin 自己 `mod`**（不编进 lib）：本域没有客人、也不被谁 `use`。**本域不立 `Fail`
//! 类型**：它只有一种失败（一次往返都做不成 ⇒ 找不到控制台），`main` 的返回类型直接用 `Reason`
//! ——一格不值得一个类型（与三台驱动那种"死法要分五步"的情形不同）。

pub mod console;
pub mod terminal;

/// 本域挂在板上的名字（板按它分人；编排域表里那一条也叫这个）。
pub const ME: &str = "canonical";

/// 等板 / 等树 / 找一趟控制台的总上限（毫秒）。**必须有界**：对面死在头几步时本域不能陪着挂死。
pub const MS: usize = 1000;

pub const E_NO_CONSOLE: env::Reason = 1;
