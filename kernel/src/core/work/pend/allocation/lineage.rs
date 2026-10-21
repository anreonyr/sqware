//! 派生来源的**出生证明**：父记录与同一份几何。
//!
//! 一个 `Option<Lineage>` 保证两者同在或同缺：独立根一条也没有，派生记录两条都有。
//! 子强持父、父只弱持子，所以这条来源依赖没有环；它是来源依赖，不是 Pie 的
//! sire/heirs 授权树。

use alloc::sync::Arc;

use super::source_authorization::SourceAuthorization;

use super::geometry::Geometry;
pub(crate) struct Lineage {
    parent: Arc<SourceAuthorization>,
    geometry: Arc<Geometry>,
}

impl Lineage {
    pub(crate) fn new(parent: Arc<SourceAuthorization>, geometry: Arc<Geometry>) -> Self {
        Self { parent, geometry }
    }

    pub(crate) fn parent(&self) -> &Arc<SourceAuthorization> {
        &self.parent
    }

    pub(crate) fn geometry(&self) -> &Arc<Geometry> {
        &self.geometry
    }
}
