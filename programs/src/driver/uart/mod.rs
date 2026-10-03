//! :实现侧 — 串口驱动域：serial@10000000 的持有者，兼控制台服务。
//!
//! # 本目录的分层（判据是**角色**，与 `driver/mod.rs` 那条同一句）
//! ```text
//! core/   纯：形、界、名（不碰内核、不碰设备；不许出现 runtime::）—— **lib 面**，客人也念它
//! client.rs  客人那一面：两端（`Rack` 的两个号）怎么取回、变成一对句柄 —— **lib 面**
//! adapt/  住持面：开会话、认领设备、开两具架、落两枚砖（碰内核）—— 由 bin 自己 mod
//! dev/    设备面：唯一碰 MMIO 的地方 —— 由 bin 自己 mod
//! main.rs 只剩流程：常驻那一圈三条（收写口 / 等组 / 排空设备）
//! ```
//! **`core` 与 `client` 在 lib 里**（与 `driver/rtc/{core,client}` 同款）：这一面两头都要念
//! ——驱动落砖那几枚名字与客人找砖那几枚名字必须是同一处；各写一份就编不过。

pub mod client;
pub mod core;

pub(crate) fn publication(
    _program: &crate::unit::UnitFile, _from: env::TaskId,
    target: &protocol::system::control::publication::Target,
    mark: env::Mark, requested: protocol::system::operator::Permit,
    _machine: &crate::system::common::machine::Machine,
    _roster: &crate::system::identity::bridge::Roster,
) -> Result<protocol::common::path::PathBuf, protocol::system::operator::Fail> {
    use protocol::system::control::publication::{Target, Scope};
    use protocol::system::operator::{Permit, Fail};
    let Target::Service { scope: Scope::Driver, group, name } = target else { return Err(Fail::Denied); };
    if group != "uart" || !["rx", "tx"].contains(&name.as_str())
        || mark != env::Mark::NONE || requested != Permit::Public { return Err(Fail::Denied); }
    protocol::driver::ROAD.try_join(group).and_then(|p| p.try_join(name)).ok_or(Fail::Denied)
}
