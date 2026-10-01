//! system::assemble — **这一景起哪些台**：过滤 ＋ 按 `deps` 算次序。
//!
//! **没有投影**：权威是那唯一一张装配表 [`PROGRAMS`](crate::program::PROGRAMS)——每一台的
//! 声明都在它自己那份 `program.rs` 里。本文件只做那张单子自己做不了的一件事：把"这一景真有
//! 的"滤出来。
//!
//! **次序不是本文件定的**（照实记：位次那一格退了）：它由各台自己那份声明里的 `deps` 算出来
//! （[`crate::program::order_scene`]）——**同一份**同时服务宿主那一侧（`cargo image` 打包时校验
//! 并打印次序）与这里。故本手只剩"过滤"那一半。
//!
//! **它不解释任何一台的字段**：要不要存在信号 / 接不接树 / 要什么、死在装配哪一步——那些由
//! [`Assembly::assemble`](crate::system::Assembly::assemble) 按那一台自己的声明走。

use alloc::vec::Vec;

use protocol::debug;

use crate::program::Program;
use crate::system::control::{Catalog, Error};

/// 这一景要起的台：**按 `deps` 算次序**（拓扑）。先起的先就绪，后面的就能向它要东西。
///
/// 只认两件事：`deps: Some`（**由编排域起**——引导域 / 编排域自己 / 压测台那几台不是）与
/// **清单里真有它**（`catalog` 是 initrd 那本账：没装进这一景的镜像就起不出来）。四枚服务
/// （`operator` / `principal` / `coalition`）也在这张单里——与其他每一台同一条路，不是"与编排
/// 者共一份字节"的那三行。
///
/// **坏图在这里是"防御"**：打包那一趟已经校验过（宿主上还报得出名字），故本处只折成一格读数。
pub fn programs(catalog: &Catalog) -> Result<Vec<&'static Program>, Error> {
    let mut list: Vec<&'static Program> = crate::program::PROGRAMS
        .iter()
        .copied()
        .filter(|p| p.relation.deps.is_some() && catalog.find(p.name()).is_some())
        .collect();
    crate::program::order_scene(&mut list).map_err(|why| {
        debug!("system: bad deps {why:?}");
        Error::Step("bad deps")
    })?;
    Ok(list)
}
