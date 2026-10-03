use alloc::{string::String, vec::Vec};
use protocol::debug;
use protocol::system::operator::client::Face;
use env::{PieToken, TaskId, Wait};
use env::wire::Span as _;
use protocol::common::path::{Path, PathBuf};
use protocol::system::operator::{EntryId, Fail, Permit};
use protocol::system::identity::Selector;
use protocol::system::control::publication::{self as pubcall, Frame, Object, Reply, Scope, Target};
use runtime::env::mail::{self, HolePie};
use crate::system::operator::bridge::Tree;
use crate::system::identity::bridge::{Roster, validate_permit, current_authority};
use crate::system::identity::names::Names;
use crate::system::common::life::table::{Slot, State, Table};
use crate::system::common::machine::Machine;
use crate::system::runtime::Runtime;
pub type PublicationRule = fn(&crate::unit::UnitFile, TaskId, &Target, env::Mark, Permit,
    &Machine, &Roster) -> Result<PathBuf, Fail>;

struct Record {
    road: PathBuf,
    target: Option<Target>,
    publisher: TaskId,
    owner: TaskId,
    authority: Option<TaskId>,
    entry: Option<PieToken>,
    borrowed: bool,
    permit: Permit,
    mount: EntryId,
    mounted: bool,
}
pub struct Publication {
    pub entry: Option<PieToken>,
    records: Vec<Record>,
}
impl Publication {
    pub fn new() -> Self {
        Self {
            entry: mail::unseal_hole(pubcall::ENTRY).ok(),
            records: Vec::new(),
        }
    }
    pub fn internal(
        &mut self,
        tree: &mut Tree,
        road: &Path,
        entry: PieToken,
        permit: Permit,
        publisher: TaskId,
        authority: Option<TaskId>,
    ) -> Result<(), &'static str> {
        let previous = self
            .records
            .iter()
            .position(|r| r.road.as_str() == road.as_str());
        if let Some(at) = previous {
            let r = &self.records[at];
            if !r.mounted {
                return Err("publication cleanup pending");
            }
            if r.publisher != publisher {
                return Err("internal publication requires retirement");
            }
            if r.entry != Some(entry) {
                return Err("internal publication conflict");
            }
            if r.permit == permit {
                return Ok(());
            }
            let mount = tree.mount(road, Some(entry), permit, Some(publisher), true)?;
            self.records[at].mount = mount;
            self.records[at].permit = permit;
            self.records[at].authority = authority;
            return Ok(());
        }
        self.records
            .try_reserve(1)
            .map_err(|_| "publication capacity")?;
        let mount = tree.mount(road, Some(entry), permit, Some(publisher), false)?;
        self.records.push(Record {
            road: road.to_path_buf(),
            target: None,
            publisher,
            owner: publisher,
            authority,
            entry: Some(entry),
            borrowed: false,
            permit,
            mount,
            mounted: true,
        });
        Ok(())
    }
    pub fn poll(
        &mut self, table: &Table, roster: &Roster, machine: &Machine,
        static_tasks: &[TaskId], runtime: &mut Runtime, names: &mut Names, tree: &mut Tree,
    ) -> Result<(), &'static str> {
        tree.connect(table.living().filter_map(|row| match row.slot {
            Slot::Live { task, .. } if matches!(row.state, State::Starting | State::Ready) => Some(task),
            _ => None,
        }))?;
        let mut living = [TaskId::new(0); Table::CAP];
        let mut count = 0;
        for row in table.living() {
            if let Slot::Live { task, .. } = row.slot
                && matches!(row.state, State::NeverStarted | State::Starting | State::Ready)
                && !runtime::env::unit::join(task, Wait::POLL).unwrap_or(true)
            { living[count] = task; count += 1; }
        }
        let me = runtime::env::unit::self_id();
        let authority = current_authority(roster);
        let host = tree.host();
        let live = |task| task == me || Some(task) == authority || Some(task) == host || living[..count].contains(&task);
        let mut at = 0;
        while at < self.records.len() {
            let record = &self.records[at];
            if !record.mounted || !live(record.publisher) || !live(record.owner) {
                self.remove(tree, at)?;
            } else { at += 1; }
        }
        names.sweep(roster, tree, live)?;
        runtime.remove(tree, live)?;
        runtime.prepare(table, roster, static_tasks, names, tree)?;
        let mut bytes = [0; Frame::LEN];
        if let Some(entry) = self.entry {
            while let Ok((n, from)) = HolePie::from_token(entry).pull(&mut bytes, Wait::POLL) {
                let Some(frame) = Frame::take(&bytes[..n]) else {
                    continue;
                };
                let source = matches!(mail::inspect(frame.entry), Ok((vestor, owner, _)) if vestor == from && owner == from);
                if !valid_back(frame.back, from) {
                    if frame.op == pubcall::PUBLISH
                        && source
                        && !self.records.iter().any(|r| r.entry == Some(frame.entry))
                    {
                        let _ = mail::forget(frame.entry);
                    }
                    continue;
                }
                let back = frame.back;
                let result = self.answer(table, roster, machine, runtime, names, tree, from, &frame);
                if frame.op == pubcall::PUBLISH
                    && source
                    && !self.records.iter().any(|r| r.entry == Some(frame.entry))
                {
                    let _ = mail::forget(frame.entry);
                }
                reply(back, result.unwrap_or_else(Reply::fail));
            }
        }
        names.poll(roster);
        Ok(())
    }
    fn answer(
        &mut self,
        table: &Table,
        roster: &Roster,
        machine: &Machine,
        runtime: &mut Runtime,
        names: &mut Names,
        tree: &mut Tree,
        from: TaskId,
        frame: &Frame,
    ) -> Result<Reply, Fail> {
        let target = frame.target().ok_or(Fail::Denied)?;
        if frame.op == pubcall::RUNTIME {
            let Target::RuntimeResource { task, .. } = target else {
                return Err(Fail::Denied);
            };
            let r = runtime
                .runs
                .iter()
                .find(|r| r.task == task)
                .ok_or(Fail::Unknown)?;
            return Ok(Reply {
                status: 0,
                kind: 0,
                task,
                number: r.team.get() as u64,
            });
        }
        if frame.op == pubcall::UNPUBLISH {
            let at = self
                .records
                .iter()
                .position(|r| r.target.as_ref() == Some(&target))
                .ok_or(Fail::Unknown)?;
            if self.records[at].publisher != from {
                return Err(Fail::Denied);
            }
            self.remove(tree, at).map_err(|_| Fail::Unknown)?;
            return Ok(Reply::mount(EntryId::new(0)));
        }
        if frame.op != pubcall::PUBLISH || !live(table, from) {
            return Err(Fail::Denied);
        }
        if !matches!(mail::inspect(frame.entry), Ok((vestor, owner, _)) if vestor == from && owner == from)
        {
            return Err(Fail::Denied);
        }
        let (road, permit, owner) =
            self.policy(table, roster, machine, runtime, from, &target, frame.permit, frame.entry)?;
        if let Some(r) = self.records.iter().find(|r| r.road == road) {
            if r.mounted
                && r.publisher == from
                && r.target.as_ref() == Some(&target)
                && r.owner == owner
                && r.permit == permit
                && r.entry
                    .is_some_and(|entry| mail::same(entry, frame.entry).unwrap_or(false))
            {
                return Ok(Reply::mount(r.mount));
            }
            return Err(Fail::Denied);
        }
        self.records.try_reserve(1).map_err(|_| Fail::Full)?;
        let kind_road = if let Target::RuntimeResource { task, kind, .. } = &target {
            let r = runtime
                .runs
                .iter_mut()
                .find(|r| r.task == *task)
                .ok_or(Fail::Denied)?;
            let kind_road = r.road.try_join(kind).ok_or(Fail::Denied)?;
            if !r.kinds.contains(&kind_road) {
                r.kinds.try_reserve(1).map_err(|_| Fail::Full)?;
                Some(kind_road)
            } else {
                None
            }
        } else {
            None
        };
        let mount = match tree.mount(&road, Some(frame.entry), permit, Some(owner), false) {
            Ok(mount) => mount,
            Err(_) => {
                if let Some(kind) = &kind_road {
                    let _ = tree.remove_empty(kind);
                }
                return Err(Fail::Unknown);
            }
        };
        if let Some(kind) = kind_road {
            runtime.runs
                .iter_mut()
                .find(|r| r.task == owner)
                .ok_or(Fail::Denied)?
                .kinds
                .push(kind);
        }
        self.records.push(Record {
            road,
            target: Some(target.clone()),
            publisher: from,
            owner,
            authority: roster.authority(),
            entry: Some(frame.entry),
            borrowed: true,
            permit,
            mount,
            mounted: true,
        });
        if let Target::Service {
            scope: Scope::Device,
            group,
            ..
        } = target
        {
            if let Permit::Identity(Selector::MemberOf(c)) = permit {
                if names
                     .register(roster, tree, &group, Object::Coalition(c), Some(from))
                    .is_err()
                {
                    self.remove(tree, self.records.len() - 1)
                        .map_err(|_| Fail::Unknown)?;
                    return Err(Fail::Denied);
                }
            }
        }
        Ok(Reply::mount(mount))
    }
    fn policy(&self, table: &Table, roster: &Roster, machine: &Machine, runtime: &Runtime,
        from: TaskId, target: &Target, requested: Permit, entry: PieToken,
    ) -> Result<(PathBuf, Permit, TaskId), Fail> {
        match target {
            Target::IdentityName { .. } => Err(Fail::Denied),
            Target::RuntimeResource { task, kind, name } =>
                runtime.policy(table, roster, from, *task, kind, name, requested),
            Target::Service { .. } => {
                let row = table.living().find(|row|
                    matches!(row.slot, Slot::Live { task, .. } if task == from)).ok_or(Fail::Denied)?;
                let program = crate::unit::PROGRAMS.iter().copied()
                    .find(|program| program.name() == row.name).ok_or(Fail::Denied)?;
                let rule = program.publication.ok_or(Fail::Denied)?;
                let mark = mail::inspect(entry).map_err(|_| Fail::Dead)?.2;
                let road = rule(program, from, target, mark, requested, machine, roster)?;
                validate_permit(roster, requested)?;
                Ok((road, requested, from))
            }
        }
    }
    fn remove(&mut self, tree: &mut Tree, at: usize) -> Result<(), &'static str> {
        let r = &self.records[at];
        tree.unmount(r.mount)?;
        self.records[at].mounted = false;
        let r = &self.records[at];
        if r.borrowed
            && let Some(entry) = r.entry
        {
            if mail::pies().any(|p| p.token == entry) {
                mail::forget(entry).map_err(|_| "publication reference cleanup")?;
            }
        }
        self.records.remove(at);
        Ok(())
    }
}
fn live(table: &Table, task: TaskId) -> bool {
    if task == runtime::env::unit::self_id() {
        return true;
    }
    table.living().any(|row| {
        matches!(row.slot, Slot::Live { task: known, .. } if known == task)
            && matches!(
                row.state,
                State::NeverStarted | State::Starting | State::Ready
            )
            && !runtime::env::unit::join(task, Wait::POLL).unwrap_or(true)
    })
}
fn valid_back(back: PieToken, from: TaskId) -> bool {
    matches!(mail::reserve(back), Ok((vestor, owner, mark)) if vestor == from && owner == from && mark == pubcall::BACK)
}
fn reply(back: PieToken, reply: Reply) {
    let mut bytes = [0; Reply::LEN];
    if let Some(n) = reply.store_at(&mut bytes, 0) {
        let _ = HolePie::from_token(back).push(&bytes[..n], Wait::POLL);
    }
    let _ = mail::release(back);
}

pub struct Landed {
    /// 落门牌那一步（`bind`）：答那一格自己的号
    pub land: Result<(), Fail>,
    /// 那一格自己的号（`land` 不成时是零号）
    pub plate: EntryId,
    /// 查回来验一遍（`token`）：**路译得回、那一枚门闩取得回来**
    pub find: Result<(), Fail>,
    /// 拿号问名：**号 ↔ 名对得上**，才算那枚号是真坐标
    pub named: Option<String>,
}

/// Publish approved service entries through Control, then verify discovery.
pub fn land(
    tree: &Face,
    family: &str,
    road: &Path,
    scope: Scope,
    group: &str,
    permit: Permit,
    faces: &[(&str, PieToken)],
    millis: Wait,
) -> Vec<Landed> {
    use protocol::system::control::publication::{Client, Target};
    let client = match Client::injected() { Ok(client) => client, Err(_) => return Vec::new() };
    let mut out = Vec::with_capacity(faces.len());
    for (name, entry) in faces {
        let target = Target::Service { scope, group: group.into(), name: (*name).into() };
        let landed = client.publish(target, *entry, permit, millis);
        let plate = landed.unwrap_or(EntryId::new(0));
        let full = road.try_join(name);
        let find = match (&landed, &full) {
            (Ok(_), Some(road)) => tree.tile(road, millis).and_then(|tile| tile.token(millis)).map(|_| ()),
            (Err(fail), _) => Err(*fail), _ => Err(Fail::Full),
        };
        let named = landed.ok().and_then(|id| tree.root().name(id, millis).ok());
        debug::put(&alloc::format!("{family}: control publication {road}/{name} land={landed:?} find={find:?}"));
        out.push(Landed { land: landed.map(|_| ()), plate, find, named });
    }
    out
}
