//! :实现侧 — 线路由者（中断面域）：外部中断的收与结（U 态，一枚线程）。
//! **它为什么叫 router**：它管的是**线**（哪条线、谁领走、领完怎么结），不是某一台设备。
//! 要认的那三样在 `adapt/boot.rs` 的三条 `Ask` 里。

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
