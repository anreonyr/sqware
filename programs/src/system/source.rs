//! system::source — **程序来源那一格**：这一台身子的那一段字节**从哪本账里取**。
//!
//! 它只有一条路：**按名字在那块账里取那一段**（零拷贝：它就是那块字节的一个切片）。
//!
//! **照实记（'哪一档'那一格退场了）**：从前这里还有一档 `Storage`（盘 / 文件系统），而声明那一侧
//! 35 份里**没有一处选过它**，这一侧的"实现"也只有一个出口（`Error::NoSource`——那一台不存在）。
//! 那一档随 `program::Origin` 整格退场，本模块只剩**一本账**：谁给字节（boot 的 initrd / 固件那块
//! 只读视图）由 [`Source::initrd`] 的调用方定，本模块不问。
//!
//! # 为什么是一条窄面，而不是一条路
//!
//! **一段字节就是全部交接面**：`UnitCall::Build` 的正文写着"**镜像字节不被拷走**，内核按 ELF
//! 段现读 `elf` 那几页"（`crates/env/src/fid.rs:389`）⇒ 任何来源都只是"给内核一段 `&[u8]`"，
//! **无一字节需要跨域搬运**：引导域手里那份 initrd 清单、编排域领到的那段只读视图，是**同一批
//! 物理页的各自 VA**，帧里也从不带镜像。故来源这一格只落一句问话：
//!
//! ```text
//!   fn image(&self, name) -> Result<&[u8], Error>
//! ```
//!
//! 装配者不问"它是 initrd 还是盘"，只问"那一段字节在哪"；**唯一消费者是
//! [`Control::spawn`](crate::system::control::Control::spawn) 里 `service::mint` 那一行**。
//! 特权级不从这里走（它在清单那一条里，声明处是装配表的 `kind`），"怎么起"更不从这里走。

use alloc::string::String;

use crate::system::control::Catalog;

/// **取字节那一格的失败域**。
///
/// 它**一句都不上线**（只有装配那一枚线程自己听得见），故不必进任何码表。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Error {
    /// 这块账里没有这一台。
    ///
    /// **照实记（"那一条读不懂"为什么不另立一格）**：清单里某一条非法（越界 / 名字非法 /
    /// 镜像为空）与"没有这一台"**在 [`Catalog::find`] 上是同一个落点**——它读不懂那一条就答
    /// "没有"。窄面照它答，不另造一格去猜它没说的话。
    Missing,
}

impl Error {
    /// 一行读数的说法（诊断那一行印的就是它）。
    pub fn said(self) -> &'static str {
        match self {
            Error::Missing => "not in catalog",
        }
    }
}

/// **来源那一格的载体**：**那一本账**（今天只有 initrd 那一本）。
///
/// **它为什么只有一格**（照实记）：从前这里还站着一格"哪一档"（声明那一侧的 `Origin`），
/// 而那一维只有一个值 ⇒ 那一格退场（见本文件头注），剩下的就是"从哪本账取"这件事本身。
pub struct Source {
    /// 这一本账的读面。
    catalog: Catalog<'static>,
}

impl Source {
    /// **那一本账**：boot / 固件给的那块（清单与全部镜像都在里面，零拷贝借映）。
    pub const fn initrd(catalog: Catalog<'static>) -> Source {
        Source { catalog }
    }

    /// **取字节那一面**：这一台身子的那一段字节在哪儿。
    ///
    /// 唯一消费者是装配时那一行 `service::mint`——它要的正是"一段 `&[u8]`"，别的（名字怎么解析、
    /// 装成哪种空间、谁来放行）一概不从这里走。
    pub fn image(&self, name: String) -> Result<&'static [u8], Error> {
        self.catalog
            .find(name.as_str())
            .map(|entry| entry.elf)
            .ok_or(Error::Missing)
    }
}
