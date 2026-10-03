use protocol::common::schedule::{Progress, Res, ResMut};
use crate::system::control::serve::{living::Living, unit::Control};
use crate::system::common::machine::Machine;
use crate::system::control::core::publication::{Publications, Record};
use crate::system::control::core::unit::{Slot, State, Table};
use crate::system::control::serve::resource::Resources;
use crate::system::identity::serve::install::Roster;
use crate::system::identity::serve::names::Names;
use crate::system::identity::serve::query::validate_permit;
use crate::system::operator::serve::install::Tree;
use env::wire::Span as _;
use env::{PieToken, TaskId, Wait};
use protocol::common::path::{Path, PathBuf};
use protocol::system::control::publication::{
    self as pubcall, Frame, Object, Reply, Scope, Target,
};
use protocol::system::identity::Selector;
use protocol::system::operator::{EntryId, Fail, Permit};
use runtime::env::mail::{self, HolePie};

impl Publications {
    pub fn internal(
        &mut self,
        tree: &mut Tree,
        road: &Path,
        entry: PieToken,
        permit: Permit,
        publisher: TaskId,
    ) -> Result<(), &'static str> {
        let previous = self
            .records
            .iter()
            .position(|r| r.road.as_str() == road.as_str());
        if let Some(at) = previous {
            let r = &self.records[at];
            if r.mount.is_none() {
                return Err("publication cleanup pending");
            }
            if r.publisher != publisher {
                return Err("internal publication requires retirement");
            }
            if r.entry != entry {
                return Err("internal publication conflict");
            }
            if r.permit == permit {
                return Ok(());
            }
            let mount = tree.mount(road, Some(entry), permit, Some(publisher), true)?;
            self.records[at].mount = Some(mount);
            self.records[at].permit = permit;
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
            entry,
            permit,
            mount: Some(mount),
        });
        Ok(())
    }

    fn answer(
        &mut self,
        table: &Table,
        roster: &Roster,
        machine: &Machine,
        runtime: &mut Resources,
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
            let team = runtime.team(task).ok_or(Fail::Unknown)?;
            return Ok(Reply {
                status: 0,
                kind: 0,
                task,
                number: team.get() as u64,
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
        let (road, permit, owner) = self.policy(
            table,
            roster,
            machine,
            runtime,
            from,
            &target,
            frame.permit,
            frame.entry,
        )?;
        if let Some(r) = self.records.iter().find(|r| r.road == road) {
            if r.mount.is_some()
                && r.publisher == from
                && r.target.as_ref() == Some(&target)
                && r.owner == owner
                && r.permit == permit
                && mail::same(r.entry, frame.entry).unwrap_or(false)
            {
                return Ok(Reply::mount(r.mount.ok_or(Fail::Unknown)?));
            }
            return Err(Fail::Denied);
        }
        self.records.try_reserve(1).map_err(|_| Fail::Full)?;
        let kind_road = if let Target::RuntimeResource { task, kind, .. } = &target {
            runtime.prepare_kind(*task, kind)?
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
            runtime.commit_kind(owner, kind)?;
        }
        self.records.push(Record {
            road,
            target: Some(target.clone()),
            publisher: from,
            owner,
            entry: frame.entry,
            permit,
            mount: Some(mount),
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
    fn policy(
        &self,
        table: &Table,
        roster: &Roster,
        machine: &Machine,
        runtime: &Resources,
        from: TaskId,
        target: &Target,
        requested: Permit,
        entry: PieToken,
    ) -> Result<(PathBuf, Permit, TaskId), Fail> {
        match target {
            Target::IdentityName { .. } => Err(Fail::Denied),
            Target::RuntimeResource { task, kind, name } => {
                runtime.policy(table, roster, from, *task, kind, name, requested)
            }
            Target::Service { .. } => {
                let row = table
                    .living()
                    .find(|row| matches!(row.slot, Slot::Live { task, .. } if task == from))
                    .ok_or(Fail::Denied)?;
                let program = crate::unit::PROGRAMS
                    .iter()
                    .copied()
                    .find(|program| program.name() == row.name)
                    .ok_or(Fail::Denied)?;
                let mark = mail::inspect(entry).map_err(|_| Fail::Dead)?.2;
                let Target::Service { scope, group, name } = target else {
                    return Err(Fail::Denied);
                };
                let mut road = None;
                for rule in program.publication {
                    match rule {
                        crate::unit::Publish::Devices if *scope == Scope::Device => {
                            road = Some(crate::service::hub::publication::device(
                                from, target, mark, requested, machine, roster,
                            )?);
                        }
                        crate::unit::Publish::Entries {
                            scope: allowed,
                            group: expected,
                            road: base,
                            entries,
                            public,
                        } => {
                            let allowed = match allowed {
                                crate::unit::PublishScope::Driver => Scope::Driver,
                                crate::unit::PublishScope::Hub => Scope::Hub,
                                crate::unit::PublishScope::Fixture => Scope::Fixture,
                            };
                            if *scope != allowed
                                || group != expected
                                || (*public && requested != Permit::Public)
                            {
                                continue;
                            }
                            if entries
                                .iter()
                                .any(|e| e.name == name && e.mark.is_none_or(|m| m == mark))
                            {
                                road = Path::new(base).try_join(name);
                            }
                        }
                        _ => {}
                    }
                }
                let road = road.ok_or(Fail::Denied)?;
                validate_permit(roster, requested)?;
                Ok((road, requested, from))
            }
        }
    }
    fn remove(&mut self, tree: &mut Tree, at: usize) -> Result<(), &'static str> {
        let r = &self.records[at];
        if let Some(mount) = r.mount {
            tree.unmount(mount)?;
        }
        self.records[at].mount = None;
        let r = &self.records[at];
        if r.target.is_some() {
            let entry = r.entry;
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
                State::NeverStarted | State::Starting | State::Ready | State::Debarked
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

pub(crate) fn inject(entry: env::PieToken, task: env::TaskId) -> Result<(), &'static str> {
    runtime::core::res::port::ship(&runtime::env::mail::HolePie::from_token(entry), task,
        env::Access::STORE, env::Policy::NONE).map(|_| ()).map_err(|_| "publication inject")
}

pub(crate) fn retire(living: Res<Living>, mut publications: ResMut<Publications>,
    mut tree: ResMut<Tree>) -> Result<Progress, &'static str> {
    let tree = &mut *tree;
    let live = |task| living.contains(task);

    let mut at = 0;
    while at < publications.records.len() {
        let r = &publications.records[at];
        if r.mount.is_none() || !live(r.publisher) || !live(r.owner) {
            publications.remove(tree, at)?;
        } else {
            at += 1;
        }
    }
    Ok(Progress::Done)
}
pub(crate) fn receive(entry: Res<PieToken>, control: Res<Control>, roster: Res<Roster>,
    machine: Res<Machine>, mut resources: ResMut<Resources>, mut names: ResMut<Names>,
    mut tree: ResMut<Tree>, mut publications: ResMut<Publications>) -> Result<Progress, &'static str> {
    let entry = *entry;
    let table = &control.table;
    let roster = &*roster;
    let machine = &*machine;
    let runtime = &mut *resources;
    let names = &mut *names;
    let tree = &mut *tree;

    let mut bytes = [0; Frame::LEN];
    {
        while let Ok((n, from)) = HolePie::from_token(entry).pull(&mut bytes, Wait::POLL) {
            let Some(frame) = Frame::take(&bytes[..n]) else {
                continue;
            };
            let source = matches!(mail::inspect(frame.entry), Ok((vestor, owner, _)) if vestor == from && owner == from);
            if !valid_back(frame.back, from) {
                if frame.op == pubcall::PUBLISH && source && !publications.owns(frame.entry) {
                    let _ = mail::forget(frame.entry);
                }
                continue;
            }
            let back = frame.back;
            let result =
                publications.answer(table, roster, machine, runtime, names, tree, from, &frame);
            if frame.op == pubcall::PUBLISH && source && !publications.owns(frame.entry) {
                let _ = mail::forget(frame.entry);
            }
            reply(back, result.unwrap_or_else(Reply::fail));
        }
    }
    Ok(Progress::Done)
}
