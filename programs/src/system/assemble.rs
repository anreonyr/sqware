//! system::assemble — **这一景起哪些台**：过滤 + 按 `order` 排。
//!
//! **没有投影**：权威是那唯一一张装配表 [`PROGRAMS`](crate::program::PROGRAMS)——每一台的
//! 声明都在它自己那份 `program.rs` 里。本文件只做那张单子自己做不了的一件事：把"这一景真有
//! 的"滤出来、按 `order` 排（次序即装配次序）。
//!
//! **它不解释任何一台的字段**：要不要存在信号 / 接不接树 / 要什么、死在装配哪一步——那些由
//! [`Assembly::assemble`](crate::system::Assembly::assemble) 按那一台自己的声明走。这里只剩
//! order / iteration / context / error。

use alloc::vec::Vec;

use crate::program::Program;
use crate::system::control::Catalog;

/// 这一景要起的台：**按 `order` 排**（小的先起）。先起的先就绪，后面的就能向它要东西。
///
/// 只认两件事：`order: Some`（**由编排域起**——引导域 / 编排域自己 / 压测台那几台不是）与
/// **清单里真有它**（`catalog` 是 initrd 那本账：没装进这一景的镜像就起不出来）。四枚服务
/// （`operator` / `principal` / `coalition`）也在这张单里（order 0/1/2）——与其他每一台同一条
/// 路，不是"与编排者共一份字节"的那三行。
pub fn programs(catalog: &Catalog) -> Vec<&'static Program> {
    let mut list: Vec<&'static Program> = crate::program::PROGRAMS
        .iter()
        .copied()
        .filter(|p| p.relation.order.is_some() && catalog.find(p.name()).is_some())
        .collect();
    list.sort_by_key(|p| p.relation.order);
    list
}
