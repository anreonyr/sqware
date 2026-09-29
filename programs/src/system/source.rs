//! system::source — **程序来源那一格**：这一台身子的那一段字节**从哪本账里取**。
//!
//! ```text
//!   Origin::Initrd   按名字在那块账里取那一段（零拷贝：它就是那块字节的一个切片）
//!   Origin::Storage  Error::NoSource（照实记：那一台 —— 盘 / 文件系统 —— 不存在）
//! ```
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

use env::Tag;

use crate::program::Origin;
use crate::system::control::Catalog;

/// **取字节那一格的失败域**。
///
/// 它**一句都不上线**（只有装配那一枚线程自己听得见），故不必进任何码表。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Error {
    /// **这一格来源今天不存在**：`Storage` 档（盘 / 文件系统那一台）**没有造出来**。
    ///
    /// **照实记**：这不是"暂时失败"，也不是"待实现的功能"——那一台不存在，故这一格是它的
    /// **全部**答案（`Source::storage()` ＋ 这一格，就是存储档今天的实现）。
    NoSource,
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
            Error::NoSource => "no source",
            Error::Missing => "not in catalog",
        }
    }
}

/// **来源那一格的载体**：哪一档 ＋ 那一档的账。
///
/// **照实记（档与账为什么是两个格）**：档（[`Origin`]）住声明那一侧，是**宿主安全**的
/// （`crates/image` 也要编它）；账（[`Catalog`]）拖着清单格式与运行时那一层，进不了声明。
/// 两者在这里合拢——`origin` 说"该问哪一本账"，`catalog` 是 initrd 那一本（今天**唯一存在**
/// 的一本）。
pub struct Source {
    /// 这一台声明上的来源档。
    origin: Origin,
    /// initrd 那一本的读面；`Storage` 档**没有它**（那一台不存在）。
    catalog: Option<Catalog<'static>>,
}

impl Source {
    /// **initrd 档**：boot / 固件给的那块账就是来源。
    pub const fn initrd(catalog: Catalog<'static>) -> Source {
        Source {
            origin: Origin::Initrd,
            catalog: Some(catalog),
        }
    }

    /// **存储档**：那一台（盘 / 文件系统）**今天不存在**——见 [`Error::NoSource`]。
    pub const fn storage() -> Source {
        Source {
            origin: Origin::Storage,
            catalog: None,
        }
    }

    /// **取字节那一面**：这一台身子的那一段字节在哪儿。
    ///
    /// 唯一消费者是装配时那一行 `service::mint`——它要的正是"一段 `&[u8]`"，别的（名字怎么解析、
    /// 装成哪种空间、谁来放行）一概不从这里走。
    pub fn image(&self, name: Tag) -> Result<&'static [u8], Error> {
        match self.origin {
            // initrd 档：按名字在那块账里取那一段（零拷贝：它就是那块字节的一个切片）。
            Origin::Initrd => match self.catalog {
                Some(catalog) => catalog
                    .find(name.as_str())
                    .map(|entry| entry.elf)
                    .ok_or(Error::Missing),
                // 构造上到不了（[`Source::initrd`] 一定带账）；显式落地一个到不了的点，不吞错。
                None => Err(Error::NoSource),
            },
            // 存储档：那一台不存在（见 [`Error::NoSource`]）。
            Origin::Storage => Err(Error::NoSource),
        }
    }
}
