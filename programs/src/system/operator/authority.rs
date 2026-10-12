use alloc::vec::Vec;
use env::{PieToken, TaskId};
use system_api::operator::{
    EntryTarget, GrantId, GrantSpec, GrantState, GrantSubmission, Ops, Scope,
};

const LIMIT: usize = 256;

#[derive(Clone)]
struct Record {
    identity: Identity,
    grant: Grant,
    cleanup: Cleanup,
}

#[derive(Clone)]
struct Grant {
    spec: GrantSpec,
    state: GrantState,
}

#[derive(Clone, Copy)]
struct Identity {
    key: GrantKey,
    issuer: TaskId,
    via: PieToken,
    parent: Option<GrantKey>,
}

#[derive(Clone, Default)]
struct Cleanup {
    entrance: Option<PieToken>,
    remote: Option<PieToken>,
    attached: bool,
    detached: bool,
    sealed: bool,
    released: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct GrantKey(u64);

#[derive(Clone, Copy)]
pub struct GrantRef {
    pub via: PieToken,
    pub caller: TaskId,
    pub id: GrantId,
}

pub struct RootGrant {
    pub system: TaskId,
    pub spec: GrantSpec,
    pub entrance: PieToken,
}

#[derive(Clone, Copy)]
struct Watermark {
    via: PieToken,
    caller: TaskId,
    high_water: u64,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AuthorityError {
    Denied,
    Conflict,
    Stale,
    Full,
    Unknown,
}

/// Bounded authorization history for one native Operator entrance.
pub struct Authority {
    root: TaskId,
    operator: TaskId,
    records: Vec<Record>,
    watermarks: Vec<Watermark>,
    next_key: u64,
}

impl Authority {
    pub fn new(root: TaskId, operator: TaskId) -> Self {
        Self {
            root,
            operator,
            records: Vec::new(),
            watermarks: Vec::new(),
            next_key: 1,
        }
    }

    pub fn bootstrap_root(&mut self, root: RootGrant) -> Result<(), AuthorityError> {
        let RootGrant {
            system,
            spec,
            entrance,
        } = root;
        if system != self.root
            || spec.id != GrantId(0)
            || spec.holder != system
            || spec.anchor.operator != self.operator
            || spec.anchor.target != EntryTarget::Root
            || spec.ops.bits() != Ops::ALL.bits()
            || spec.scope != Scope::Subtree
            || spec.permit.is_some()
            || spec.source.is_some()
            || spec.source_mark.is_some()
        {
            return Err(AuthorityError::Denied);
        }
        if let Some(old) = self
            .records
            .iter()
            .find(|record| record.grant.spec.id == GrantId(0))
        {
            return if old.identity.issuer == system
                && old.grant.spec == spec
                && old.cleanup.entrance == Some(entrance)
            {
                Ok(())
            } else {
                Err(AuthorityError::Conflict)
            };
        }
        if !self.records.is_empty() {
            return Err(AuthorityError::Conflict);
        }
        self.records
            .try_reserve(1)
            .map_err(|_| AuthorityError::Full)?;
        self.records.push(Record {
            identity: Identity {
                key: GrantKey(0),
                issuer: system,
                via: entrance,
                parent: None,
            },
            grant: Grant {
                spec,
                state: GrantState::Granted,
            },
            cleanup: Cleanup {
                entrance: Some(entrance),
                ..Cleanup::default()
            },
        });
        Ok(())
    }

    pub(super) fn grant(
        &mut self,
        request: GrantSubmission,
        relative_anchor: &system_api::operator::Path,
    ) -> Result<(GrantState, GrantKey), AuthorityError> {
        let GrantSubmission {
            caller,
            parent,
            via,
            spec,
        } = request;
        let parent_record = self
            .records
            .iter()
            .find(|record| {
                record.cleanup.entrance == Some(via)
                    && record.grant.spec.holder == caller
                    && record.grant.state == GrantState::Granted
            })
            .ok_or(AuthorityError::Denied)?;
        let parent_key = parent_record.identity.key;
        if parent != Some(parent_record.grant.spec.id) || !self.ancestors_live(parent_key) {
            return Err(AuthorityError::Denied);
        }
        if let Some(old) = self.records.iter().find(|r| {
            r.identity.via == via && r.identity.issuer == caller && r.grant.spec.id == spec.id
        }) {
            return if old.grant.spec == spec && old.identity.parent == Some(parent_key) {
                Ok((old.grant.state, old.identity.key))
            } else {
                Err(AuthorityError::Conflict)
            };
        }
        let watermark = self
            .watermarks
            .iter()
            .position(|w| w.via == via && w.caller == caller);
        if watermark.is_some_and(|at| spec.id.0 <= self.watermarks[at].high_water) {
            return Err(AuthorityError::Stale);
        }
        if self.records.len() >= LIMIT {
            return Err(AuthorityError::Full);
        }
        if !spec.ops.is_valid()
            || spec.holder == self.root
            || matches!(spec.anchor.target, EntryTarget::Root)
        {
            return Err(AuthorityError::Denied);
        }
        if !parent_record.grant.spec.ops.contains(Ops::GRANT)
            || parent_record.grant.spec.anchor.operator != spec.anchor.operator
            || parent_record
                .grant
                .spec
                .permit
                .is_some_and(|permit| spec.permit != Some(permit))
            || parent_record
                .grant
                .spec
                .source
                .is_some_and(|source| spec.source != Some(source))
            || parent_record
                .grant
                .spec
                .source_mark
                .is_some_and(|mark| spec.source_mark != Some(mark))
            || !parent_record.grant.spec.ops.contains(spec.ops)
            || !grant_scope_narrows(&parent_record.grant.spec, &spec, relative_anchor)
        {
            return Err(AuthorityError::Denied);
        }
        self.records
            .try_reserve(1)
            .map_err(|_| AuthorityError::Full)?;
        if let Some(at) = watermark {
            self.watermarks[at].high_water = spec.id.0;
        } else {
            if self.watermarks.len() >= LIMIT {
                return Err(AuthorityError::Full);
            }
            self.watermarks
                .try_reserve(1)
                .map_err(|_| AuthorityError::Full)?;
            self.watermarks.push(Watermark {
                via,
                caller,
                high_water: spec.id.0,
            });
        }
        let key = GrantKey(self.next_key);
        self.next_key = self.next_key.checked_add(1).ok_or(AuthorityError::Full)?;
        self.records.push(Record {
            identity: Identity {
                key,
                issuer: caller,
                via,
                parent: Some(parent_key),
            },
            grant: Grant {
                spec,
                state: GrantState::Pending,
            },
            cleanup: Cleanup::default(),
        });
        Ok((GrantState::Pending, key))
    }

    pub fn delivered(&mut self, key: GrantRef, remote: PieToken) -> Result<(), AuthorityError> {
        let r = self
            .records
            .iter_mut()
            .find(|r| {
                r.identity.via == key.via
                    && r.grant.spec.id == key.id
                    && r.identity.issuer == key.caller
            })
            .ok_or(AuthorityError::Unknown)?;
        if r.grant.state == GrantState::Granted {
            return if r.cleanup.remote == Some(remote) {
                Ok(())
            } else {
                Err(AuthorityError::Conflict)
            };
        }
        if r.grant.state != GrantState::Pending {
            return Err(AuthorityError::Denied);
        }
        r.cleanup.remote = Some(remote);
        r.grant.state = GrantState::Granted;
        Ok(())
    }

    pub fn remote(&self, key: GrantKey) -> Option<PieToken> {
        self.records
            .iter()
            .find(|record| record.identity.key == key && record.grant.state == GrantState::Granted)
            .and_then(|record| record.cleanup.remote)
    }

    pub fn inspect(&self, key: GrantRef) -> Result<GrantState, AuthorityError> {
        if let Some(r) = self.records.iter().find(|r| {
            r.identity.via == key.via
                && r.grant.spec.id == key.id
                && r.identity.issuer == key.caller
        }) {
            return Ok(r.grant.state);
        }
        let high_water = self
            .watermarks
            .iter()
            .find(|w| w.via == key.via && w.caller == key.caller)
            .map_or(0, |w| w.high_water);
        if key.id.0 <= high_water {
            Ok(GrantState::Closed)
        } else {
            Ok(GrantState::Unseen)
        }
    }

    pub fn revoke(&mut self, key: GrantRef) -> Result<GrantState, AuthorityError> {
        if let Some(r) = self.records.iter_mut().find(|r| {
            r.identity.via == key.via
                && r.grant.spec.id == key.id
                && r.identity.issuer == key.caller
        }) {
            if r.identity.key == GrantKey(0) {
                return Err(AuthorityError::Denied);
            }
            if r.grant.state == GrantState::Closed {
                return Ok(GrantState::Closed);
            }
            r.grant.state = GrantState::Revoking;
            return Ok(GrantState::Revoking);
        }
        let at = self
            .watermarks
            .iter()
            .position(|w| w.via == key.via && w.caller == key.caller);
        if at.is_some_and(|at| key.id.0 <= self.watermarks[at].high_water) {
            return Ok(GrantState::Closed);
        }
        if let Some(at) = at {
            self.watermarks[at].high_water = key.id.0;
        } else {
            if self.watermarks.len() >= LIMIT {
                return Err(AuthorityError::Full);
            }
            self.watermarks
                .try_reserve(1)
                .map_err(|_| AuthorityError::Full)?;
            self.watermarks.push(Watermark {
                via: key.via,
                caller: key.caller,
                high_water: key.id.0,
            });
        }
        Ok(GrantState::Closed)
    }

    pub fn revoke_plan(
        &mut self,
        key: GrantRef,
    ) -> Result<Vec<(GrantKey, PieToken, TaskId, GrantId, Option<PieToken>)>, AuthorityError> {
        let Some(root) = self
            .records
            .iter()
            .find(|r| {
                r.identity.via == key.via
                    && r.identity.issuer == key.caller
                    && r.grant.spec.id == key.id
            })
            .map(|r| r.identity.key)
        else {
            return Ok(Vec::new());
        };
        let mut plan = Vec::new();
        plan.try_reserve(self.records.len())
            .map_err(|_| AuthorityError::Full)?;
        for record in &self.records {
            let mut parent = record.identity.parent;
            let mut descendant = record.identity.key == root;
            let mut remaining = self.records.len();
            while !descendant && parent.is_some() && remaining > 0 {
                remaining -= 1;
                let current = parent.unwrap();
                if current == root {
                    descendant = true;
                    break;
                }
                parent = self
                    .records
                    .iter()
                    .find(|candidate| candidate.identity.key == current)
                    .and_then(|candidate| candidate.identity.parent);
            }
            if descendant && record.grant.state != GrantState::Closed {
                plan.push((
                    record.identity.key,
                    record.identity.via,
                    record.identity.issuer,
                    record.grant.spec.id,
                    record.cleanup.entrance,
                ));
            }
        }
        for record in &mut self.records {
            if plan.iter().any(|(_, record_via, issuer, grant, _)| {
                record.identity.via == *record_via
                    && record.identity.issuer == *issuer
                    && record.grant.spec.id == *grant
            }) {
                record.grant.state = GrantState::Revoking;
            }
        }
        plan.reverse();
        Ok(plan)
    }

    pub fn closed(&mut self, key: GrantRef) -> Result<(), AuthorityError> {
        let at = self
            .records
            .iter()
            .position(|r| {
                r.identity.via == key.via
                    && r.grant.spec.id == key.id
                    && r.identity.issuer == key.caller
            })
            .ok_or(AuthorityError::Unknown)?;
        let key = self.records[at].identity.key;
        let entrance = self.records[at].cleanup.entrance;
        if self.records[at].grant.state != GrantState::Revoking
            && self.records[at].grant.state != GrantState::Closed
        {
            return Err(AuthorityError::Denied);
        }
        if self
            .records
            .iter()
            .any(|r| r.identity.parent == Some(key) && r.grant.state != GrantState::Closed)
        {
            return Err(AuthorityError::Denied);
        }
        self.records.swap_remove(at);
        if let Some(entrance) = entrance {
            self.watermarks
                .retain(|watermark| watermark.via != entrance);
        }
        Ok(())
    }

    pub fn set_entrance(
        &mut self,
        key: GrantRef,
        entrance: PieToken,
    ) -> Result<(), AuthorityError> {
        let r = self
            .records
            .iter_mut()
            .find(|r| {
                r.identity.via == key.via
                    && r.grant.spec.id == key.id
                    && r.identity.issuer == key.caller
            })
            .ok_or(AuthorityError::Unknown)?;
        if r.grant.state != GrantState::Pending {
            return Err(AuthorityError::Denied);
        }
        if r.cleanup.entrance.is_some_and(|old| old != entrance) {
            return Err(AuthorityError::Conflict);
        }
        r.cleanup.entrance = Some(entrance);
        Ok(())
    }

    pub fn grant_at(
        &self,
        entrance: PieToken,
        caller: TaskId,
    ) -> Option<(GrantKey, GrantId, &GrantSpec)> {
        self.records
            .iter()
            .find(|r| {
                r.cleanup.entrance == Some(entrance)
                    && r.grant.spec.holder == caller
                    && r.grant.state == GrantState::Granted
            })
            .filter(|r| self.ancestors_live(r.identity.key))
            .map(|r| (r.identity.key, r.grant.spec.id, &r.grant.spec))
    }

    pub fn entrance(&self, key: GrantRef) -> Option<PieToken> {
        self.records
            .iter()
            .find(|r| {
                r.identity.via == key.via
                    && r.identity.issuer == key.caller
                    && r.grant.spec.id == key.id
            })
            .and_then(|r| r.cleanup.entrance)
    }

    pub fn root_entrance(&self) -> Option<PieToken> {
        self.records
            .iter()
            .find(|r| r.identity.key == GrantKey(0))
            .and_then(|r| r.cleanup.entrance)
    }

    pub fn by_entrance(&self, entrance: PieToken) -> Option<GrantKey> {
        self.records
            .iter()
            .find(|r| r.cleanup.entrance == Some(entrance))
            .map(|r| r.identity.key)
    }

    pub fn live(&self, key: GrantKey) -> bool {
        self.records
            .iter()
            .any(|r| r.identity.key == key && r.grant.state == GrantState::Granted)
            && self.ancestors_live(key)
    }

    pub fn by_id(&self, key: GrantKey, caller: TaskId) -> Option<(GrantId, &GrantSpec)> {
        let record = self.records.iter().find(|r| {
            r.identity.key == key
                && r.grant.state == GrantState::Granted
                && r.grant.spec.holder == caller
        })?;
        self.ancestors_live(key)
            .then_some((record.grant.spec.id, &record.grant.spec))
    }

    pub fn cleanup_progress(&self, key: GrantKey) -> Option<(bool, bool, bool, bool)> {
        let r = self
            .records
            .iter()
            .find(|record| record.identity.key == key)?;
        Some((
            r.cleanup.attached,
            r.cleanup.detached,
            r.cleanup.sealed,
            r.cleanup.released,
        ))
    }

    pub fn attached(&mut self, key: GrantKey) -> Result<(), AuthorityError> {
        let r = self
            .records
            .iter_mut()
            .find(|record| record.identity.key == key)
            .ok_or(AuthorityError::Unknown)?;
        if r.grant.state != GrantState::Pending || r.cleanup.entrance.is_none() {
            return Err(AuthorityError::Denied);
        }
        r.cleanup.attached = true;
        Ok(())
    }

    pub fn cleanup_step(&mut self, key: GrantKey, step: u8) -> Result<(), AuthorityError> {
        let r = self
            .records
            .iter_mut()
            .find(|record| record.identity.key == key)
            .ok_or(AuthorityError::Unknown)?;
        if r.grant.state != GrantState::Revoking {
            return Err(AuthorityError::Denied);
        }
        match step {
            0 => r.cleanup.detached = true,
            1 if r.cleanup.detached => r.cleanup.sealed = true,
            2 if r.cleanup.sealed => r.cleanup.released = true,
            _ => return Err(AuthorityError::Denied),
        }
        Ok(())
    }

    fn ancestors_live(&self, key: GrantKey) -> bool {
        let mut next = self
            .records
            .iter()
            .find(|r| r.identity.key == key)
            .and_then(|r| r.identity.parent);
        let mut remaining = self.records.len();
        while let Some(parent) = next {
            if remaining == 0 {
                return false;
            }
            remaining -= 1;
            let Some(record) = self
                .records
                .iter()
                .find(|r| r.identity.key == parent && r.grant.state == GrantState::Granted)
            else {
                return false;
            };
            next = record.identity.parent;
        }
        true
    }
}

fn grant_scope_narrows(
    parent: &GrantSpec,
    child: &GrantSpec,
    relative_anchor: &system_api::operator::Path,
) -> bool {
    if relative_anchor.is_empty() {
        parent.anchor == child.anchor && parent.scope.narrows(&child.scope)
    } else {
        parent
            .scope
            .narrows_at(relative_anchor.as_str(), &child.scope)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use system_api::operator::{Entry, EntryId, EntryTarget, PathBuf, Permit};

    fn path(value: &str) -> PathBuf {
        PathBuf::try_new(value).unwrap()
    }
    struct SpecInput {
        id: u64,
        holder: usize,
        anchor: Entry,
        ops: Ops,
        scope: Scope,
    }
    fn spec(input: SpecInput) -> GrantSpec {
        GrantSpec {
            id: GrantId(input.id),
            holder: TaskId::new(input.holder),
            anchor: input.anchor,
            scope: input.scope,
            ops: input.ops,
            permit: Some(Permit::Public),
            source: None,
            source_mark: None,
        }
    }
    fn submit(
        authority: &mut Authority,
        request: GrantSubmission,
        relative: &str,
    ) -> Result<(GrantState, GrantKey), AuthorityError> {
        let path = path(relative);
        authority.grant(request, path.as_path())
    }
    fn root(auth: &mut Authority) -> (TaskId, PieToken) {
        let system = TaskId::new(1);
        let entrance = PieToken::mint(90);
        let mut grant = spec(SpecInput {
            id: 0,
            holder: 1,
            anchor: Entry {
                operator: system,
                target: EntryTarget::Root,
            },
            ops: Ops::ALL,
            scope: Scope::Subtree,
        });
        grant.permit = None;
        auth.bootstrap_root(RootGrant {
            system,
            spec: grant,
            entrance,
        })
        .unwrap();
        (system, entrance)
    }
    fn node(operator: TaskId, id: usize) -> Entry {
        Entry {
            operator,
            target: EntryTarget::Node(EntryId::new(id)),
        }
    }

    #[test]
    fn grant_replay_narrowing_and_parent_revoke_block_descendants() {
        let mut auth = Authority::new(TaskId::new(1), TaskId::new(1));
        let (system, root_entry) = root(&mut auth);
        assert_eq!(
            auth.revoke(GrantRef {
                via: root_entry,
                caller: system,
                id: GrantId(0)
            }),
            Err(AuthorityError::Denied)
        );
        let first = spec(SpecInput {
            id: 1,
            holder: 2,
            anchor: node(system, 7),
            ops: Ops::GRANT.union(Ops::BIND),
            scope: Scope::Subtree,
        });
        let submission = GrantSubmission {
            caller: system,
            via: root_entry,
            parent: Some(GrantId(0)),
            spec: first.clone(),
        };
        let (_, key) = submit(&mut auth, submission.clone(), "svc").unwrap();
        assert_eq!(
            submit(&mut auth, submission, "svc").unwrap(),
            (GrantState::Pending, key)
        );
        let first_entry = PieToken::mint(91);
        auth.set_entrance(
            GrantRef {
                via: root_entry,
                caller: system,
                id: GrantId(1),
            },
            first_entry,
        )
        .unwrap();
        let first_remote = PieToken::mint(191);
        let first_ref = GrantRef {
            via: root_entry,
            caller: system,
            id: GrantId(1),
        };
        auth.delivered(first_ref, first_remote).unwrap();
        assert_eq!(auth.remote(key), Some(first_remote));
        assert_eq!(auth.delivered(first_ref, first_remote), Ok(()));
        assert_eq!(
            auth.delivered(first_ref, PieToken::mint(193)),
            Err(AuthorityError::Conflict)
        );

        let child = spec(SpecInput {
            id: 1,
            holder: 3,
            anchor: node(system, 8),
            ops: Ops::BIND,
            scope: Scope::Names(alloc::vec![path("a")]),
        });
        let child_submission = GrantSubmission {
            caller: TaskId::new(2),
            via: first_entry,
            parent: Some(GrantId(1)),
            spec: child,
        };
        let (_, child_key) = submit(&mut auth, child_submission, "driver").unwrap();
        let child_entry = PieToken::mint(92);
        auth.set_entrance(
            GrantRef {
                via: first_entry,
                caller: TaskId::new(2),
                id: GrantId(1),
            },
            child_entry,
        )
        .unwrap();
        auth.delivered(
            GrantRef {
                via: first_entry,
                caller: TaskId::new(2),
                id: GrantId(1),
            },
            PieToken::mint(192),
        )
        .unwrap();
        assert!(auth.grant_at(child_entry, TaskId::new(3)).is_some());
        assert!(auth.grant_at(child_entry, TaskId::new(4)).is_none());
        let root_ref = GrantRef {
            via: root_entry,
            caller: system,
            id: GrantId(1),
        };
        assert_eq!(auth.revoke(root_ref), Ok(GrantState::Revoking));
        assert!(auth.grant_at(child_entry, TaskId::new(3)).is_none());
        assert!(auth.by_id(child_key, TaskId::new(3)).is_none());
        let plan = auth.revoke_plan(root_ref).unwrap();
        assert_eq!(
            plan.iter().map(|(_, _, _, id, _)| *id).collect::<Vec<_>>(),
            alloc::vec![GrantId(1), GrantId(1)]
        );
        for (_, via, issuer, id, _) in plan {
            auth.closed(GrantRef {
                via,
                caller: issuer,
                id,
            })
            .unwrap();
        }
        assert_eq!(auth.inspect(root_ref), Ok(GrantState::Closed));
    }

    #[test]
    fn unseen_cancel_and_id_watermarks_are_scoped_to_actual_entrance() {
        let mut auth = Authority::new(TaskId::new(1), TaskId::new(1));
        let (system, root_entry) = root(&mut auth);
        let mut make_parent = |id, holder, token| {
            let request = spec(SpecInput {
                id,
                holder,
                anchor: node(system, 7),
                ops: Ops::ALL,
                scope: Scope::Subtree,
            });
            submit(
                &mut auth,
                GrantSubmission {
                    caller: system,
                    via: root_entry,
                    parent: Some(GrantId(0)),
                    spec: request,
                },
                "svc",
            )
            .unwrap();
            auth.set_entrance(
                GrantRef {
                    via: root_entry,
                    caller: system,
                    id: GrantId(id),
                },
                token,
            )
            .unwrap();
            auth.delivered(
                GrantRef {
                    via: root_entry,
                    caller: system,
                    id: GrantId(id),
                },
                PieToken::mint((190 + id) as usize),
            )
            .unwrap();
        };
        let a = PieToken::mint(91);
        let b = PieToken::mint(92);
        make_parent(1, 2, a);
        make_parent(2, 2, b);
        let cancelled = GrantRef {
            via: a,
            caller: TaskId::new(2),
            id: GrantId(5),
        };
        assert_eq!(auth.revoke(cancelled), Ok(GrantState::Closed));
        assert_eq!(auth.revoke(cancelled), Ok(GrantState::Closed));
        let child_a = spec(SpecInput {
            id: 5,
            holder: 3,
            anchor: node(system, 8),
            ops: Ops::BIND,
            scope: Scope::Subtree,
        });
        assert_eq!(
            submit(
                &mut auth,
                GrantSubmission {
                    caller: TaskId::new(2),
                    via: a,
                    parent: Some(GrantId(1)),
                    spec: child_a
                },
                "x"
            ),
            Err(AuthorityError::Stale)
        );
        let child_b = spec(SpecInput {
            id: 5,
            holder: 3,
            anchor: node(system, 8),
            ops: Ops::BIND,
            scope: Scope::Subtree,
        });
        assert_eq!(
            submit(
                &mut auth,
                GrantSubmission {
                    caller: TaskId::new(2),
                    via: b,
                    parent: Some(GrantId(2)),
                    spec: child_b
                },
                "x"
            )
            .unwrap()
            .0,
            GrantState::Pending
        );
        let changed = spec(SpecInput {
            id: 5,
            holder: 4,
            anchor: node(system, 8),
            ops: Ops::BIND,
            scope: Scope::Subtree,
        });
        assert_eq!(
            submit(
                &mut auth,
                GrantSubmission {
                    caller: TaskId::new(2),
                    via: b,
                    parent: Some(GrantId(2)),
                    spec: changed
                },
                "x"
            ),
            Err(AuthorityError::Conflict)
        );
    }

    #[test]
    fn names_scope_covers_only_actual_descendants() {
        let scope = Scope::Names(alloc::vec![path("svc")]);
        assert!(scope.allows("svc"));
        assert!(scope.allows("svc/driver"));
        assert!(!scope.allows("svcx"));
        assert!(!scope.allows("driver/svc"));
    }

    #[test]
    fn root_holder_and_canonical_entry_issuer_are_distinct() {
        let system = TaskId::new(1);
        let operator = TaskId::new(9);
        let entrance = PieToken::mint(90);
        let grant = GrantSpec {
            id: GrantId(0),
            holder: system,
            anchor: Entry {
                operator,
                target: EntryTarget::Root,
            },
            scope: Scope::Subtree,
            ops: Ops::ALL,
            permit: None,
            source: None,
            source_mark: None,
        };
        let mut authority = Authority::new(system, operator);
        authority
            .bootstrap_root(RootGrant {
                system,
                spec: grant.clone(),
                entrance,
            })
            .unwrap();
        let mut forged = Authority::new(system, operator);
        let mut wrong = grant;
        wrong.anchor.operator = system;
        assert_eq!(
            forged.bootstrap_root(RootGrant {
                system,
                spec: wrong,
                entrance
            }),
            Err(AuthorityError::Denied)
        );
    }

    #[test]
    fn root_policy_can_narrow_once_and_children_cannot_expand_it() {
        let mut auth = Authority::new(TaskId::new(1), TaskId::new(1));
        let (system, root_entry) = root(&mut auth);
        let mut parent = spec(SpecInput {
            id: 1,
            holder: 2,
            anchor: node(system, 7),
            ops: Ops::GRANT.union(Ops::BIND),
            scope: Scope::Subtree,
        });
        parent.permit = Some(Permit::Bound);
        parent.source = Some(TaskId::new(9));
        parent.source_mark = Some(env::Mark::new(77));
        submit(
            &mut auth,
            GrantSubmission {
                caller: system,
                via: root_entry,
                parent: Some(GrantId(0)),
                spec: parent.clone(),
            },
            "svc",
        )
        .unwrap();
        let entrance = PieToken::mint(91);
        auth.set_entrance(
            GrantRef {
                via: root_entry,
                caller: system,
                id: GrantId(1),
            },
            entrance,
        )
        .unwrap();
        auth.delivered(
            GrantRef {
                via: root_entry,
                caller: system,
                id: GrantId(1),
            },
            PieToken::mint(191),
        )
        .unwrap();
        let base = spec(SpecInput {
            id: 1,
            holder: 3,
            anchor: node(system, 8),
            ops: Ops::BIND,
            scope: Scope::Names(alloc::vec![path("device")]),
        });
        let mut narrow = base.clone();
        narrow.permit = parent.permit;
        narrow.source = parent.source;
        narrow.source_mark = parent.source_mark;
        let make_submit = |spec| GrantSubmission {
            caller: TaskId::new(2),
            via: entrance,
            parent: Some(GrantId(1)),
            spec,
        };
        let mut no_permit = base.clone();
        no_permit.permit = None;
        let mut wider_permit = base.clone();
        wider_permit.permit = Some(Permit::Public);
        let mut no_source = base.clone();
        no_source.source = None;
        let mut wider_source = base.clone();
        wider_source.source = Some(TaskId::new(10));
        let mut no_mark = base.clone();
        no_mark.source_mark = None;
        let mut wider_mark = base.clone();
        wider_mark.source_mark = Some(env::Mark::new(78));
        let mut more_ops = base;
        more_ops.ops = Ops::ALL;
        for candidate in [
            no_permit,
            wider_permit,
            no_source,
            wider_source,
            no_mark,
            wider_mark,
            more_ops,
        ] {
            assert_eq!(
                submit(&mut auth, make_submit(candidate), "driver"),
                Err(AuthorityError::Denied)
            );
        }
        let (_, child_key) = submit(&mut auth, make_submit(narrow), "driver").unwrap();
        assert_ne!(child_key, GrantKey(0));
    }

    #[test]
    fn relocating_anchor_preserves_remaining_parent_scope() {
        let parent = Scope::Names(alloc::vec![path("svc/only"), path("svc/other/deep/x")]);
        assert!(parent.narrows_at("svc/only", &Scope::Subtree));
        assert!(parent.narrows_at("svc/other/deep", &Scope::Names(alloc::vec![path("x/y")])));
        assert!(!parent.narrows_at("svc/other/deep", &Scope::Subtree));
        assert!(!parent.narrows_at(
            "svc",
            &Scope::Names(alloc::vec![path("only"), path("outside")])
        ));
        assert!(!parent.narrows_at("svcx", &Scope::Names(alloc::vec![path("only")])));
    }
}
