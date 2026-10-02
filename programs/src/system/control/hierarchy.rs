//! Trusted publication records, identity aliases and runtime directories.
use super::{BOOT_MS, Control};
use crate::service::operator::bridge::Tree;
use crate::system::common::life::table::{Slot, State};
use alloc::{
    string::{String, ToString},
    vec::Vec,
};
use env::wire::Span as _;
use env::{PieToken, TaskId, TeamId, Wait};
use protocol::common::path::{Path, PathBuf};
use protocol::communication::session::establish;
use protocol::service::identity::client::Face;
use protocol::service::identity::{Grant, Selector, Wire};
use protocol::service::operator::{EntryId, Fail, Permit};
use protocol::system::control::publication::{
    self as pubcall, Frame, Object, Reply, Scope, Target,
};
use runtime::env::mail::{self, HolePie};

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
struct Alias {
    name: String,
    object: Object,
    lifetime: Option<TaskId>,
    entry: PieToken,
    pane: EntryId,
    mount: EntryId,
}
struct Runtime {
    task: TaskId,
    team: TeamId,
    road: PathBuf,
    pane: EntryId,
    team_pane: EntryId,
    kinds: Vec<PathBuf>,
}
pub(crate) struct Approval {
    pub service: TaskId,
    pub task: TaskId,
    pub kind: String,
    pub name: String,
    pub permit: Permit,
}

pub(crate) struct Hierarchy {
    pub entry: Option<PieToken>,
    records: Vec<Record>,
    aliases: Vec<Alias>,
    runs: Vec<Runtime>,
    approvals: Vec<Approval>,
}
impl Hierarchy {
    pub fn new() -> Self {
        Self {
            entry: mail::unseal_hole(pubcall::ENTRY).ok(),
            records: Vec::new(),
            aliases: Vec::new(),
            runs: Vec::new(),
            approvals: Vec::new(),
        }
    }
    pub fn inject(&self, task: TaskId) -> Result<(), &'static str> {
        let entry = self.entry.ok_or("publication entry")?;
        runtime::core::res::port::ship(
            &HolePie::from_token(entry),
            task,
            runtime::core::res::port::Access::STORE,
            runtime::core::res::port::Policy::NONE,
        )
        .map(|_| ())
        .map_err(|_| "publication inject")
    }
    pub(crate) fn runtime_road(&self, task: TaskId) -> Option<PathBuf> {
        self.runs
            .iter()
            .find(|r| r.task == task)
            .map(|r| r.road.clone())
    }
    pub fn approve(&mut self, approval: Approval) -> Result<(), &'static str> {
        self.approvals
            .try_reserve(1)
            .map_err(|_| "resource policy capacity")?;
        self.approvals.push(approval);
        Ok(())
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
    pub fn register(
        &mut self,
        control: &Control,
        tree: &mut Tree,
        name: &str,
        object: Object,
        lifetime: Option<TaskId>,
    ) -> Result<(), &'static str> {
        if !pubcall::valid_name(name) {
            return Err("identity alias name");
        }
        if self
            .aliases
            .iter()
            .any(|a| a.name == name && a.object.kind() == object.kind() && a.object != object)
        {
            return Err("identity alias conflict");
        }
        if self
            .aliases
            .iter()
            .any(|a| a.name == name && a.object == object)
        {
            return Ok(());
        }
        validate(control, object).map_err(|_| "identity alias source")?;
        self.aliases
            .try_reserve(1)
            .map_err(|_| "identity alias capacity")?;
        let road = object.road(name).ok_or("identity alias path")?;
        let pane = tree.mount(
            road.parent().ok_or("identity alias parent")?,
            None,
            Permit::Public,
            None,
            false,
        )?;
        let entry = match mail::unseal_hole(pubcall::REF) {
            Ok(entry) => entry,
            Err(_) => {
                let _ = tree.unmount(pane);
                return Err("identity alias entry");
            }
        };
        let mount = match tree.mount(
            &road,
            Some(entry),
            Permit::Public,
            Some(runtime::env::unit::self_id()),
            false,
        ) {
            Ok(mount) => mount,
            Err(why) => {
                let _ = mail::seal(entry);
                let _ = mail::release(entry);
                let _ = tree.unmount(pane);
                return Err(why);
            }
        };
        self.aliases.push(Alias {
            name: name.into(),
            object,
            lifetime,
            entry,
            pane,
            mount,
        });
        Ok(())
    }
    pub fn prepare(&mut self, control: &Control, tree: &mut Tree) -> Result<(), &'static str> {
        if current_authority(control).is_none() || tree.host().is_none() {
            return Ok(());
        }
        for row in control.table.living() {
            let Slot::Live {
                task,
                team: Some(team),
            } = row.slot
            else {
                continue;
            };
            if !live(control, task)
                || !matches!(
                    row.state,
                    State::NeverStarted | State::Starting | State::Ready
                )
                || self.runs.iter().any(|r| r.task == task)
            {
                continue;
            }
            let Some(binding) = binding(control, task).map_err(|_| "runtime identity query")?
            else {
                continue;
            };
            self.runs.try_reserve(1).map_err(|_| "runtime capacity")?;
            let team_road = Path::new("uit")
                .try_join(&team.get().to_string())
                .ok_or("runtime team path")?;
            let road = team_road
                .try_join(&task.get().to_string())
                .ok_or("runtime task path")?;
            let team_pane = tree.mount(&team_road, None, Permit::Public, None, false)?;
            let pane = match tree.mount(&road, None, Permit::Public, None, false) {
                Ok(pane) => pane,
                Err(why) => {
                    let _ = tree.unmount(team_pane);
                    return Err(why);
                }
            };
            self.runs.push(Runtime {
                task,
                team,
                road,
                pane,
                team_pane,
                kinds: Vec::new(),
            });
            if control.static_tasks.contains(&task)
                || Some(task) == tree.host()
                || Some(task) == control.roster.authority()
            {
                self.register(
                    control,
                    tree,
                    &row.name,
                    Object::Principal(binding.origin.principal),
                    Some(task),
                )?;
            }
        }
        Ok(())
    }
    pub fn poll(&mut self, control: &Control, tree: &mut Tree) -> Result<(), &'static str> {
        self.sweep(control, tree)?;
        self.prepare(control, tree)?;
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
                let result = self.answer(control, tree, from, &frame);
                if frame.op == pubcall::PUBLISH
                    && source
                    && !self.records.iter().any(|r| r.entry == Some(frame.entry))
                {
                    let _ = mail::forget(frame.entry);
                }
                reply(back, result.unwrap_or_else(Reply::fail));
            }
        }
        // Ref answers read only the verified index and never wait for Operator.
        for alias in &self.aliases {
            while let Ok((n, from)) = HolePie::from_token(alias.entry).pull(&mut bytes, Wait::POLL)
            {
                let Some(frame) = Frame::take(&bytes[..n]) else {
                    continue;
                };
                if !valid_back(frame.back, from) {
                    continue;
                }
                let accepted = frame.op == pubcall::RESOLVE
                    && frame.name == alias.name
                    && frame.kind == 10 + alias.object.kind()
                    && Some(alias.object.authority()) == current_authority(control);
                reply(
                    frame.back,
                    if accepted {
                        Reply::object(alias.object)
                    } else {
                        Reply::fail(Fail::Unjudged)
                    },
                );
            }
        }
        Ok(())
    }
    fn answer(
        &mut self,
        control: &Control,
        tree: &mut Tree,
        from: TaskId,
        frame: &Frame,
    ) -> Result<Reply, Fail> {
        let target = frame.target().ok_or(Fail::Denied)?;
        if frame.op == pubcall::RUNTIME {
            let Target::RuntimeResource { task, .. } = target else {
                return Err(Fail::Denied);
            };
            let r = self
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
        if frame.op != pubcall::PUBLISH || !live(control, from) {
            return Err(Fail::Denied);
        }
        if !matches!(mail::inspect(frame.entry), Ok((vestor, owner, _)) if vestor == from && owner == from)
        {
            return Err(Fail::Denied);
        }
        let (road, permit, owner) =
            self.policy(control, from, &target, frame.permit, frame.entry)?;
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
            let r = self
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
            self.runs
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
            authority: control.roster.authority(),
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
                if self
                    .register(control, tree, &group, Object::Coalition(c), Some(from))
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
        control: &Control,
        from: TaskId,
        target: &Target,
        requested: Permit,
        entry: PieToken,
    ) -> Result<(PathBuf, Permit, TaskId), Fail> {
        let service = control
            .table
            .living()
            .find(|row| matches!(row.slot, Slot::Live { task, .. } if task == from))
            .ok_or(Fail::Denied)?;
        let mark = mail::inspect(entry).map_err(|_| Fail::Dead)?.2;
        match target {
            Target::IdentityName { .. } => Err(Fail::Denied),
            Target::Service {
                scope: Scope::Driver,
                group,
                name,
            } => {
                let valid = match service.name.as_str() {
                    "router" | "rtc" => {
                        group.is_empty()
                            && name == &service.name
                            && mark == protocol::driver::ENTRY_MARK
                    }
                    "uart" => {
                        group == "uart"
                            && ["rx", "tx"].contains(&name.as_str())
                            && mark == env::Mark::NONE
                    }
                    _ => false,
                };
                if !valid || requested != Permit::Public {
                    return Err(Fail::Denied);
                }
                let road = if group.is_empty() {
                    protocol::driver::ROAD.to_path_buf()
                } else {
                    protocol::driver::ROAD.try_join(group).ok_or(Fail::Denied)?
                };
                Ok((
                    road.try_join(name).ok_or(Fail::Denied)?,
                    Permit::Public,
                    from,
                ))
            }
            Target::Service {
                scope: Scope::Hub,
                group,
                name,
            } => {
                if service.name != "hub" || !group.is_empty() || requested != Permit::Public {
                    return Err(Fail::Denied);
                }
                let grant = protocol::service::hub::Grant::ALL
                    .iter()
                    .find(|g| g.name() == name && g.mark() == mark)
                    .ok_or(Fail::Denied)?;
                Ok((
                    Path::new("svc/hub")
                        .try_join(grant.name())
                        .ok_or(Fail::Denied)?,
                    Permit::Public,
                    from,
                ))
            }
            Target::Service {
                scope: Scope::Device,
                group,
                name,
            } => {
                if service.name != "hub" || mark != protocol::service::hub::Grant::Claim.mark() {
                    return Err(Fail::Denied);
                }
                let Permit::Identity(Selector::MemberOf(c)) = requested else {
                    return Err(Fail::Denied);
                };
                let subject = binding(control, from)?.ok_or(Fail::Denied)?.current;
                if !subject.coalitions.contains(c) {
                    return Err(Fail::Denied);
                }
                let valid = (group == protocol::service::hub::BOOT
                    && [protocol::service::hub::DTB, protocol::service::hub::IRQ]
                        .contains(&name.as_str()))
                    || control.machine.devices().is_some_and(|devices| {
                        devices
                            .iter()
                            .any(|d| d.class.as_str() == group && d.name.as_str() == name)
                    });
                if !valid {
                    return Err(Fail::Denied);
                }
                validate(control, Object::Coalition(c))?;
                Ok((
                    Path::new("dev")
                        .try_join(group)
                        .and_then(|p| p.try_join(name))
                        .ok_or(Fail::Denied)?,
                    requested,
                    from,
                ))
            }
            Target::Service {
                scope: Scope::Fixture,
                group,
                name,
            } => {
                // Fixture policy is installed only for named harness programs in this scene.
                if !crate::harness::probe::hierarchy::fixture_allowed(
                    service.name.as_str(),
                    group,
                    name,
                ) {
                    return Err(Fail::Denied);
                }
                validate_permit(control, requested)?;
                Ok((
                    if service.name == "probe-rack-mount" {
                        Path::new("probe-rack").try_join(name)
                    } else {
                        Path::new("svc").try_join(group).and_then(|p| p.try_join(name))
                    }
                        .ok_or(Fail::Denied)?,
                    requested,
                    from,
                ))
            }
            Target::RuntimeResource { task, kind, name } => {
                let run = self
                    .runs
                    .iter()
                    .find(|r| r.task == *task)
                    .ok_or(Fail::Denied)?;
                if !live(control, *task) {
                    return Err(Fail::Denied);
                }
                let permit = if let Some(approval) = self.approvals.iter().find(|a| {
                    a.service == from && a.task == *task && a.kind == *kind && a.name == *name
                }) {
                    approval.permit
                } else if from == *task {
                    match kind.as_str() {
                        "public" => Permit::Public,
                        "hole" => Permit::Identity(Selector::Exact(
                            binding(control, from)?
                                .ok_or(Fail::Denied)?
                                .current
                                .principal,
                        )),
                        "bound" => Permit::Bound,
                        _ => return Err(Fail::Denied),
                    }
                } else {
                    return Err(Fail::Denied);
                };
                if permit != requested {
                    return Err(Fail::Denied);
                }
                validate_permit(control, permit)?;
                Ok((
                    run.road
                        .try_join(kind)
                        .and_then(|p| p.try_join(name))
                        .ok_or(Fail::Denied)?,
                    permit,
                    *task,
                ))
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
    pub fn retire(&mut self, tree: &mut Tree, authority: TaskId) -> Result<(), &'static str> {
        let mut at = 0;
        while at < self.aliases.len() {
            if self.aliases[at].object.authority() == authority {
                self.remove_alias(tree, at)?;
            } else {
                at += 1;
            }
        }
        let mut at = 0;
        while at < self.records.len() {
            if self.records[at].authority == Some(authority)
                && self.records[at].publisher == authority
            {
                self.remove(tree, at)?;
            } else {
                at += 1;
            }
        }
        Ok(())
    }
    fn remove_alias(&mut self, tree: &mut Tree, at: usize) -> Result<(), &'static str> {
        let a = &self.aliases[at];
        tree.unmount(a.mount)?;
        tree.unmount(a.pane)?;
        let _ = mail::seal(a.entry);
        let _ = mail::release(a.entry);
        self.aliases.remove(at);
        Ok(())
    }
    fn sweep(&mut self, control: &Control, tree: &mut Tree) -> Result<(), &'static str> {
        if tree.host().is_none() {
            return Ok(());
        }
        let mut at = 0;
        while at < self.records.len() {
            let r = &self.records[at];
            if !r.mounted || !live(control, r.publisher) || !live(control, r.owner) {
                self.remove(tree, at)?;
            } else {
                at += 1;
            }
        }
        let mut at = 0;
        while at < self.aliases.len() {
            if Some(self.aliases[at].object.authority()) != current_authority(control)
                || self.aliases[at]
                    .lifetime
                    .is_some_and(|task| !live(control, task))
            {
                self.remove_alias(tree, at)?;
            } else {
                at += 1;
            }
        }
        let mut at = 0;
        while at < self.runs.len() {
            if live(control, self.runs[at].task) {
                at += 1;
                continue;
            }
            let r = &self.runs[at];
            // Only this runtime's empty children may be removed, deepest first.
            for road in r.kinds.iter().rev() {
                tree.remove_empty(road)?;
            }
            tree.unmount(r.pane)?;
            if !self
                .runs
                .iter()
                .any(|other| other.task != r.task && other.team == r.team)
            {
                tree.unmount(r.team_pane)?;
            }
            self.approvals
                .retain(|a| a.task != r.task && a.service != r.task);
            self.runs.remove(at);
        }
        self.approvals
            .retain(|a| live(control, a.service) && live(control, a.task));
        Ok(())
    }
}
fn current_authority(control: &Control) -> Option<TaskId> {
    control
        .roster
        .authority()
        .filter(|authority| !runtime::env::unit::join(*authority, Wait::POLL).unwrap_or(true))
}
fn live(control: &Control, task: TaskId) -> bool {
    if task == runtime::env::unit::self_id() {
        return true;
    }
    control.table.living().any(|row| {
        matches!(row.slot, Slot::Live { task: known, .. } if known == task)
            && matches!(
                row.state,
                State::NeverStarted | State::Starting | State::Ready
            )
            && !runtime::env::unit::join(task, Wait::POLL).unwrap_or(true)
    })
}
fn face(control: &Control, grant: Grant) -> Result<Face, Fail> {
    let authority = control.roster.authority().ok_or(Fail::Unjudged)?;
    let entry = establish::find(authority, grant.mark()).ok_or(Fail::Unjudged)?;
    Face::direct(authority, grant, entry).map_err(|_| Fail::Unjudged)
}
fn binding(
    control: &Control,
    task: TaskId,
) -> Result<Option<protocol::service::identity::Binding>, Fail> {
    match face(control, Grant::Resolve)?
        .call(Wire::Resolve(task), Wait::AtMost(BOOT_MS))
        .map_err(|_| Fail::Unjudged)?
    {
        protocol::service::identity::Reply::Binding(b) => Ok(b),
        _ => Err(Fail::Unjudged),
    }
}
fn validate(control: &Control, object: Object) -> Result<(), Fail> {
    if control.roster.authority() != Some(object.authority()) {
        return Err(Fail::Unjudged);
    }
    let (grant, wire) = match object {
        Object::Principal(p) => (Grant::Heir, Wire::Heir(p, p)),
        Object::Coalition(c) => (Grant::Members, Wire::Members(c, None)),
    };
    face(control, grant)?
        .call(wire, Wait::AtMost(BOOT_MS))
        .map(|_| ())
        .map_err(|_| Fail::Unjudged)
}
fn validate_permit(control: &Control, permit: Permit) -> Result<(), Fail> {
    match permit {
        Permit::Identity(Selector::Exact(p) | Selector::DescendantOf(p)) => {
            validate(control, Object::Principal(p))
        }
        Permit::Identity(Selector::MemberOf(c)) => validate(control, Object::Coalition(c)),
        _ => Ok(()),
    }
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
