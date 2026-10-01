//! 过滤 ＋ 按 after 算次序。
//! **没有投影**：权威是那唯一一张装配表 PROGRAMS——每一台的
//! 的"滤出来。

use alloc::vec::Vec;

use protocol::debug;

use crate::boot::Catalog;
use crate::system::control::Error;
use crate::unit::UnitFile;

/// 这一景要起的台：**按 `after` 算次序**（拓扑）。先起的先就绪，后面的就能向它要东西。
/// 只认两件事：`deps: Some`（**由编排域起**——引导镜像自己 / 压测台那几台不是）与
/// **清单里真有它**（`catalog` 是 initrd 那本账：没装进这一景的镜像就起不出来）。四枚服务
/// （`operator` / `principal` / `coalition`）也在这张单里——与其他每一台同一条路，不是"与编排
/// 者共一份字节"的那三行。
pub fn programs(catalog: &Catalog) -> Result<Vec<&'static UnitFile>, Error> {
    let mut list: Vec<&'static UnitFile> = crate::unit::PROGRAMS
        .iter()
        .copied()
        .filter(|p| p.relation.after.is_some() && catalog.find(p.name()).is_some())
        .collect();
    crate::unit::order_scene(&mut list).map_err(|why| {
        debug!("system: bad deps {why:?}");
        Error::Step("bad deps")
    })?;
    Ok(list)
}
