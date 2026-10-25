//! 从已选部署取启动单元，并按依赖关系排序。

use alloc::vec::Vec;

use programs::debug;

use crate::boot::Catalog;
use crate::system::app::Declarations;
use crate::system::control::unit::start::Error;
use unit::{Deployment, UnitFile};

/// 启动项必须在部署和镜像清单中同时存在。
pub fn programs(
    catalog: &Catalog,
    declarations: Declarations,
    deployment: &'static Deployment,
) -> Result<Vec<&'static UnitFile>, Error> {
    let mut list: Vec<&'static UnitFile> = deployment
        .start
        .iter()
        .map(|name| {
            let program = declarations.find(name).ok_or_else(|| {
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
