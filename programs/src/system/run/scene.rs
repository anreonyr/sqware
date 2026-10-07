//! 从镜像清单选择单元，并按依赖关系排序。

use alloc::vec::Vec;

use programs::debug;

use crate::boot::Catalog;
use crate::system::control::serve::start::Error;
use crate::unit::UnitFile;

/// 只选择声明依赖且包含在镜像中的单元。
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
