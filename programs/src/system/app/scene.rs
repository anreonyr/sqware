//! 从已选部署取启动单元，并按依赖关系排序。

use alloc::vec::Vec;

use programs::debug;

use crate::boot::Catalog;
use crate::system::control::unit::start::Error;
use crate::unit::UnitFile;

/// 启动项必须在部署和镜像清单中同时存在。
pub fn programs(catalog: &Catalog) -> Result<Vec<&'static UnitFile>, Error> {
    let deployment = crate::unit::select_deployment(catalog.deployment(), catalog.entry())
        .map_err(|why| {
            debug!("system: invalid deployment {}: {why:?}", catalog.deployment());
            Error::Step("invalid deployment selection")
        })?;
    let mut list: Vec<&'static UnitFile> = deployment
        .start
        .iter()
        .map(|name| {
            let program = crate::unit::PROGRAMS
                .iter()
                .copied()
                .find(|program| program.name() == *name)
                .ok_or_else(|| {
                    programs::debug!(
                        "system: deployment {} references missing UnitFile {}",
                        deployment.name,
                        name
                    );
                    Error::Step("deployment start UnitFile missing")
                })?;
            if catalog.find(program.loader.image).is_none() {
                programs::debug!(
                    "system: deployment {} start image missing {}",
                    deployment.name,
                    name
                );
                return Err(Error::Step("deployment start image missing"));
            }
            Ok(program)
        })
        .collect::<Result<_, _>>()?;
    crate::unit::order_scene(&mut list, deployment.targets, deployment.bootstrap).map_err(
        |why| {
            debug!("system: bad deps {why:?}");
            Error::Step("bad deps")
        },
    )?;
    Ok(list)
}
