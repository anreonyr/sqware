//! :实现侧 — 实时钟驱动域：rtc@101000 的持有者，兼报时服务（门牌 /svc/drv/rtc）。
//! 的那一半（`adapt/` ＋ `dev/rtc.rs`）由 bin 自己 `mod`——`PieToken` 只在铸它的那张表里念得出来，
//! 而客人不在这张表里。
//! # 纯与适配的分界
//! **"等事件"移不进纯核**：组（`Pile`）是内核的，而纯核的纪律是不出现 runtime:: ⇒ 常驻
//! 那一圈的**壳**留在 `adapt/resident.rs`；但它的**每一格判定**都在 core::Host——壳里因此
//! 没有语义 `if`（核吐什么，它就执行什么）。
//! **两份失败域**也由这条分界分开：core::Fail 是**上线**那一格（讲客人那一问，折成答码），
//! :Fail 是**下线**那一格（讲这一域死在
//! 起手/常驻的哪一步，报给内核出口）。
//! # 服务面：两个方向放进同一面

pub mod client;
pub mod core;

pub(crate) fn publication(
    program: &crate::unit::UnitFile, _from: env::TaskId,
    target: &protocol::system::control::publication::Target,
    mark: env::Mark, requested: protocol::system::operator::Permit,
    _machine: &crate::system::common::machine::Machine,
    _roster: &crate::system::identity::bridge::Roster,
) -> Result<protocol::common::path::PathBuf, protocol::system::operator::Fail> {
    use protocol::system::control::publication::{Target, Scope};
    use protocol::system::operator::{Permit, Fail};
    let Target::Service { scope: Scope::Driver, group, name } = target else { return Err(Fail::Denied); };
    if !group.is_empty() || name != program.name() || mark != protocol::driver::ENTRY_MARK
        || requested != Permit::Public { return Err(Fail::Denied); }
    protocol::driver::ROAD.try_join(name).ok_or(Fail::Denied)
}
