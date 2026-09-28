//! driver::fail — **一台驱动怎么死**：三台共用一枚扁平的死法（号取自装配表）。
//!
//! ```text
//!   Fail   { code, text }：一个号 + 那一句话 —— `main` 的返回类型
//!   号     各域在装配表上那一号，**名字就是那个名字**（`programs::program::uart::E_UART`）；
//!          "配给那一趟没成"带 `assemble` 那一族的号（原样带）
//! ```
//!
//! # 照实记（`Step` / `Who` 两个 trait 与泛型 `Fail<W>` 已退场）
//!
//! 从前这里是一套三件的机制：各域一枚 `Step` 枚举（"只列自己走得到的步"）、两个 trait
//! （`Step` 折号与取句、`Who` 给号与"配给那趟"的格子）、外加泛型 `Fail<W>` 落地 [`Exit`]。
//! 三份 `adapt/fail.rs` 各写一遍，**共 277 行**，办的是同一件事：把"死在第几步"折成一个号
//! ＋一句话。
//!
//! 折叠的判据：那套机制换来一条**类型级**性质——"返回类型只说得出本域走得到的步子集"。
//! 而变体全是 `pub`、可达性本来就靠人看 ⇒ 那条性质是文档性的；行数却是实打实的。故按
//! "一个动作不许有两套类型"折平：**一枚结构 ＋ 各域那一号**，读数写在死处。
//!
//! **代价照实记**：那一步的读数不再集中在一张 `text()` 表里，而是写在**死处**
//! （`Fail::at(E_UART, "tree")`）——判据是"读数离它描述的那件事越近越好"，与
//! "一格判据只问一件事"同一条口径。
//!
//! # 裁（用户）：读数说**步名**，域名由号带
//!
//! 起手那一趟收进 [`Context`](crate::driver::context::Context) 之后，有一格必须裁：
//! 那句"死在哪一步"的话由**谁**说。共用的那一层不认识域名，而 `Report` 的 note 是一句
//! `&str`（要拼 `"uart: tree"` 就得给 `Fail` 带 alloc，或给出口类型加第二格）。裁定：
//!
//! ```text
//!   Fail::at(E_UART, "tree")     ← 话是**步名**，域名由号带（E_UART 就是"uart 死了"）
//! ```
//!
//! 号与域名在装配表上是**同一格的两半**（`program` 的 `died`），内核出口那一行把号一并印出来
//! ⇒ 信息没丢，只是挪了地方。**代价照实记**：那句话从此不自我说明，读的人要把号对回表；
//! 换来的是全族（三台）一套词，以及起手那一段不必为此造一个"词表"参数。
//!
//! **成功那几行 `debug!` 读数仍带域名**（`debug!("{me}: line occupied")`）：那是运行期拼的，
//! 不受这一格约束，且调试面本来就以域名分段。
//!
//! # 照实记（三份 `adapt/fail.rs` 薄壳也退场了）
//!
//! 上一刀折平之后，三台还各留一份 `adapt/fail.rs`：正文三行——一个 `DIED`（转发
//! [`programs::program`] 那一号）、一句 `ASSEMBLE`、一个 `type Fail` 别名。那是**第二个名字**
//! 加**一个转发文件**，且 `Fail::assemble` 与 [`Fail::at`] 的函数体逐字相同。故这一刀：
//!
//! - 号回**它自己的名字**（各域 `main` / `boot` / `resident` 直接写 `E_UART` / `E_RTC` /
//!   `E_ROUTER`——与装配表同一处）；
//! - 那句话回**死处**（`Hub::claim` 那一趟自己说 `"bond"` / `"list"` / `"claim"` 那种步名）；
//! - 一枚结构只留 [`Fail::at`] 一手。
//!
//! 一族因此只剩本文件。**留档**：`uart/adapt/` 随这一刀整个消失（它此后只剩那一份 fail）。
//!
//! # 照实记（号从哪来）
//!
//! [E_ROUTER](programs::program::router::E_ROUTER) /
//! [E_UART](programs::program::uart::E_UART) /
//! [E_RTC](programs::program::rtc::E_RTC) 取自 [programs::program] 那张装配表
//! （"**这一台**死了"，见那份 `program` 的 `died`）。唯一自己带号的是"配给那一趟没成"
//! ——**照实记（那一族随"收配给"那条路一起退了）**：设备那一轴改由本域自己走一趟设备账之后，
//! "配给那一趟没成"不再存在（本域的死法只剩它自己那一号 ＋ 步名）。
//!
//! # 两枚 `fail` 是两件事
//!
//! 本表是**下线**那一格——"这一域死在起手/常驻的哪一步"，读的人是内核出口与板那条死亡道；
//! `rtc::core::Fail` 是**上线**那一格——"客人那一问怎么了"，折成答码过线（`Taken` / `Past` /
//! `Denied`）。故本表住适配侧（[`Exit`] 是程序侧那一手），而它**不是**服务面的失败域。

use crate::program::Died;
use crate::{Exit, Report};

/// 一台驱动的死法：**一个号 ＋ 那一句话**（`main` 的返回类型），`?` 一路把它带出来。
pub struct Fail {
    code: Died,
    text: &'static str,
}

impl Fail {
    /// 死在**某一步**：号是装配表里那一号（[E_UART](programs::program::uart::E_UART) 那种，
    /// 或本域自己那几条步名），那句话是**步名**（`"tree"` / `"desk"` 那种，
    /// 见上面那一格裁）。
    pub const fn at(code: Died, text: &'static str) -> Self {
        Self { code, text }
    }
}

impl Exit for Fail {
    fn report(&self) -> Report<'_> {
        Report::note(self.code, self.text)
    }
}
