use alloc::sync::Arc;

use crate::core::memory::nest::NestError;
use crate::core::work::unit::space::Space;
use crate::machine::sync::OnceLock;

use super::Team;
use super::construct::make_team;

pub(crate) static KERNEL_TEAM: OnceLock<Arc<Team>> = OnceLock::new();

/// 落座内核团队（只一次）：它没有家长、也没有 sire。
pub(crate) fn init_kernel(space: Arc<Space>) -> Result<&'static Arc<Team>, NestError> {
    if KERNEL_TEAM.get().is_none() {
        let team = make_team(space, None, env::TaskId::new(0), false)?;
        assert!(KERNEL_TEAM.set(team).is_ok());
    }
    Ok(KERNEL_TEAM.get().expect("kernel team initialized"))
}

pub fn kernel() -> Option<&'static Arc<Team>> {
    KERNEL_TEAM.get()
}
