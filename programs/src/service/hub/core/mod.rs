//! 册上一条一条 ＋ 每类那枚盟。纯核心：不碰内核、不碰树、
//! 不碰会话——喂两个假闭包（"主人还答得出吗"）就能把规矩推理干净。
//! # 三条不变量（做进类型，不写成注释故事）
//! 1. **一台一主，且只认活着的主人**——`Cell` 两形说的是"有没有主"；"有主但主人没了"当场

mod binding;
mod bindings;
mod cell;
mod claim_request;
mod enrollment;
mod enrollment_effects;
mod enrollment_step;
mod enrollments;
mod entry;
mod execution;
mod execution_effects;
mod execution_failure;
mod execution_step;
mod executions;
mod league;
mod league_step;
mod leagues;
mod ledger;
mod owner;

pub use binding::{Binding, Step};
pub use claim_request::ClaimRequest;
pub use cell::Cell;
pub use enrollment::Enrollment;
pub use enrollment_effects::EnrollmentEffects;
pub use enrollment_step::EnrollmentStep;
pub use entry::Entry;
pub use execution::Execution;
pub use execution_effects::ExecutionEffects;
pub use execution_failure::ExecutionFailure;
pub use execution_step::ExecutionStep;
pub use ledger::Ledger;
pub use owner::Owner;
