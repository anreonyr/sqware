use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;
use env::wire::Span as _;
use env::{Mark, Permission, PieToken, TaskId, Wait};
use ipc::{rpc, time::Deadline};
use resource::port::{self, Policy};
use resource::raw::{Hole, inspect, reserve};
use system_api::operator as api;
use system_api::operator::entry::{self, Call, Request, Response};

#[derive(Clone, Copy)]
pub struct GrantInfo {
    pub id: api::GrantId,
    pub entry: api::Entry,
    pub entrance: PieToken,
}

#[derive(Clone, Copy)]
pub struct PartResult {
    pub entry: api::Entry,
    pub created: bool,
}

#[derive(Clone, Copy)]
pub struct BindResult {
    pub binding: api::Binding,
    pub created: bool,
}

pub struct Capability {
    operator: TaskId,
    entrance: PieToken,
    anchor: Option<api::Entry>,
    grant: Option<GrantInfo>,
    call: rpc::request::Sender<Call>,
    pending_import_cleanup: RefCell<Vec<PieToken>>,
}

const MAX_PENDING_IMPORT_CLEANUP: usize = 8;

impl Capability {
    /// Imports the descriptor for a declared namespace and verifies both native issuers.
    pub fn injected(namespace: usize, issuer: TaskId, wait: Wait) -> Result<Self, api::Fail> {
        let mark = api::descriptor_mark(namespace).ok_or(api::Fail::Denied)?;
        let token =
            ipc::session::establish::claim(issuer, mark, wait).map_err(|_| api::Fail::Denied)?;
        let descriptor = Hole::from_raw(token);
        let descriptor_info = inspect(descriptor.token()).map_err(|_| api::Fail::Dead)?;
        if !valid_descriptor(&descriptor_info, issuer, mark) {
            return Err(api::Fail::Denied);
        }
        let mut bytes = [0; api::GrantDescriptor::LEN];
        let (len, sender) = descriptor
            .pull(&mut bytes, wait)
            .map_err(|_| api::Fail::Dead)?;
        if len != bytes.len() || sender != issuer {
            return Err(api::Fail::Denied);
        }
        let (frame, end) = api::GrantDescriptor::fetch_at(&bytes, 0).ok_or(api::Fail::Unknown)?;
        if end != bytes.len() || frame.entry_root > 1 {
            return Err(api::Fail::Unknown);
        }
        let entry = api::Entry {
            operator: frame.entry_operator,
            target: if frame.entry_root == 1 {
                api::EntryTarget::Root
            } else {
                api::EntryTarget::Node(frame.entry_id)
            },
        };
        let seed = inspect(frame.entrance).map_err(|_| api::Fail::Dead)?;
        if !seed.alive
            || seed.owner != frame.entry_operator
            || seed.vestor != frame.entry_operator
            || seed.kind != env::PieKind::Hole
            || seed.mark != entry::AUTH_ENTRY
            || seed.permission != Permission::STORE
        {
            return Err(api::Fail::Denied);
        }
        let mut capability = Self::direct_at(frame.entry_operator, frame.entrance, entry)?;
        capability.grant = Some(GrantInfo {
            id: api::GrantId(frame.id),
            entry,
            entrance: frame.entrance,
        });
        Ok(capability)
    }

    pub fn direct(operator: TaskId, entrance: PieToken) -> Result<Self, api::Fail> {
        let info = inspect(entrance).map_err(|_| api::Fail::Dead)?;
        if !info.alive
            || info.owner != operator
            || info.kind != env::PieKind::Hole
            || info.mark != entry::AUTH_ENTRY
            || info.permission != Permission::STORE
        {
            return Err(api::Fail::Denied);
        }
        let call = rpc::request::Sender::<Call>::from_raw(entrance, entry::AUTH_BACK)
            .map_err(|_| api::Fail::Unknown)?;
        if call.peer() != operator {
            return Err(api::Fail::Denied);
        }
        Ok(Self {
            operator,
            entrance,
            anchor: None,
            grant: None,
            call,
            pending_import_cleanup: RefCell::new(Vec::new()),
        })
    }

    /// Builds a capability from trusted per-holder grant metadata.
    pub fn direct_at(
        operator: TaskId,
        entrance: PieToken,
        anchor: api::Entry,
    ) -> Result<Self, api::Fail> {
        coordinates(operator, anchor)?;
        let seed = inspect(entrance).map_err(|_| api::Fail::Dead)?;
        if seed.owner != operator || seed.vestor != operator {
            return Err(api::Fail::Denied);
        }
        let mut capability = Self::direct(operator, entrance)?;
        capability.anchor = Some(anchor);
        Ok(capability)
    }

    /// The actual anchor delivered with this authority seed.
    pub fn anchor(&self) -> Result<api::Entry, api::Fail> {
        self.anchor.ok_or(api::Fail::Denied)
    }

    pub fn grant_info(&self) -> Option<GrantInfo> {
        self.grant
    }

    pub fn entrance(&self) -> PieToken {
        self.entrance
    }

    pub fn part(
        &self,
        anchor: api::Entry,
        name: &str,
        operation: api::OperationId,
        wait: Wait,
    ) -> Result<PartResult, api::Fail> {
        if !system_api::operator::name::valid(name) {
            return Err(api::Fail::Denied);
        }
        let (anchor, is_root) = coordinates(self.operator, anchor)?;
        let request = Request {
            op: entry::PART,
            anchor,
            root: is_root,
            id: operation.0,
            parent: 0,
            holder: TaskId::new(0),
            ops: 0,
            permit: api::Permit::Public,
            permit_restricted: 0,
            source: TaskId::new(0),
            source_mark: 0,
            resource: PieToken::NONE,
            resource_id: 0,
            view: 0,
            expected: 0,
            name: String::from(name),
            scope: String::new(),
            back: PieToken::NONE,
        };
        let reply = self.call(request, wait)?;
        Ok(PartResult {
            entry: reply.entry(self.operator)?,
            created: reply.state == 1,
        })
    }

    pub fn bind(
        &self,
        anchor: api::Entry,
        name: &str,
        resource: PieToken,
        view: api::ResourceView,
        permit: api::Permit,
        operation: api::OperationId,
        wait: Wait,
    ) -> Result<BindResult, api::Fail> {
        if !system_api::operator::name::valid(name) {
            return Err(api::Fail::Denied);
        }
        let (anchor_id, anchor_root) = coordinates(self.operator, anchor)?;
        self.reserve_import_cleanup()?;
        let resource_id = resource.get() as u64;
        let imported = port::ship(resource, self.operator, view.access(), Policy::VEST)
            .map_err(|_| api::Fail::Denied)?
            .seed();
        let request = Request {
            op: entry::BIND,
            anchor: anchor_id,
            root: anchor_root,
            id: operation.0,
            parent: 0,
            holder: TaskId::new(0),
            ops: 0,
            permit,
            permit_restricted: 0,
            source: TaskId::new(0),
            source_mark: 0,
            resource: imported,
            resource_id,
            view: view.bits(),
            expected: 0,
            name: String::from(name),
            scope: String::new(),
            back: PieToken::NONE,
        };
        let reply = self.staged_import(self.call_staged(request, wait), imported)?;
        let entry = reply.entry(self.operator)?;
        Ok(BindResult {
            binding: api::Binding {
                entry,
                revision: reply.revision,
            },
            created: reply.state == 1,
        })
    }

    pub fn replace(
        &self,
        binding: api::Binding,
        resource: PieToken,
        view: api::ResourceView,
        permit: api::Permit,
        wait: Wait,
    ) -> Result<api::Binding, api::Fail> {
        let (anchor, is_root) = coordinates(self.operator, binding.entry)?;
        self.reserve_import_cleanup()?;
        let resource_id = resource.get() as u64;
        let imported = port::ship(resource, self.operator, view.access(), Policy::VEST)
            .map_err(|_| api::Fail::Denied)?
            .seed();
        let request = Request {
            op: entry::REPLACE,
            anchor,
            root: is_root,
            id: 0,
            parent: 0,
            holder: TaskId::new(0),
            ops: 0,
            permit,
            permit_restricted: 0,
            source: TaskId::new(0),
            source_mark: 0,
            resource: imported,
            resource_id,
            view: view.bits(),
            expected: binding.revision,
            name: String::new(),
            scope: String::new(),
            back: PieToken::NONE,
        };
        let reply = self.staged_import(self.call_staged(request, wait), imported)?;
        let entry = reply.entry(self.operator)?;
        Ok(api::Binding {
            entry,
            revision: reply.revision,
        })
    }

    pub fn unbind(&self, binding: api::Binding, wait: Wait) -> Result<(), api::Fail> {
        let (anchor, is_root) = coordinates(self.operator, binding.entry)?;
        let request = Request {
            op: entry::UNBIND,
            anchor,
            root: is_root,
            id: 0,
            parent: 0,
            holder: TaskId::new(0),
            ops: 0,
            permit: api::Permit::Public,
            permit_restricted: 0,
            source: TaskId::new(0),
            source_mark: 0,
            resource: PieToken::NONE,
            resource_id: 0,
            view: 0,
            expected: binding.revision,
            name: String::new(),
            scope: String::new(),
            back: PieToken::NONE,
        };
        self.call(request, wait).map(|_| ())
    }

    pub fn unpart(&self, entry: api::Entry, wait: Wait) -> Result<(), api::Fail> {
        let (anchor, is_root) = coordinates(self.operator, entry)?;
        let request = Request {
            op: entry::UNPART,
            anchor,
            root: is_root,
            id: 0,
            parent: 0,
            holder: TaskId::new(0),
            ops: 0,
            permit: api::Permit::Public,
            permit_restricted: 0,
            source: TaskId::new(0),
            source_mark: 0,
            resource: PieToken::NONE,
            resource_id: 0,
            view: 0,
            expected: 0,
            name: String::new(),
            scope: String::new(),
            back: PieToken::NONE,
        };
        self.call(request, wait).map(|_| ())
    }

    pub fn find(
        &self,
        anchor: api::Entry,
        name: &str,
        permit: api::Permit,
        wait: Wait,
    ) -> Result<api::BoundResource, api::Fail> {
        if !system_api::operator::name::valid(name) {
            return Err(api::Fail::Denied);
        }
        let (anchor_id, is_root) = coordinates(self.operator, anchor)?;
        let mut request = at(entry::FIND, anchor, 0);
        request.anchor = anchor_id;
        request.root = is_root;
        request.name = String::from(name);
        request.permit = permit;
        let reply = self.call(request, wait)?;
        self.bound(reply)
    }

    /// Reads a known binding by its actual Entry reference, without resolving a second path.
    pub fn read(
        &self,
        entry: api::Entry,
        permit: api::Permit,
        wait: Wait,
    ) -> Result<api::BoundResource, api::Fail> {
        let (anchor, is_root) = coordinates(self.operator, entry)?;
        let mut request = at(entry::READ, entry, 0);
        request.anchor = anchor;
        request.root = is_root;
        request.permit = permit;
        self.bound(self.call(request, wait)?)
    }

    pub fn grant(
        &self,
        spec: api::GrantSpec,
        parent: Option<api::GrantId>,
        wait: Wait,
    ) -> Result<GrantInfo, api::Fail> {
        let (anchor, root) = coordinates(self.operator, spec.anchor)?;
        let scope = encode_scope(&spec.scope).ok_or(api::Fail::Full)?;
        let reply = self.call(
            Request {
                op: entry::GRANT,
                anchor,
                root,
                id: spec.id.0,
                parent: parent.map_or(0, |id| id.0),
                holder: spec.holder,
                ops: u64::from(spec.ops.bits()),
                permit: spec.permit.unwrap_or(api::Permit::Public),
                permit_restricted: u8::from(spec.permit.is_some()),
                source: spec.source.unwrap_or(TaskId::new(0)),
                source_mark: spec.source_mark.unwrap_or(Mark::NONE).get(),
                resource: PieToken::NONE,
                resource_id: 0,
                view: 0,
                expected: 0,
                name: String::new(),
                scope,
                back: PieToken::NONE,
            },
            wait,
        )?;
        let entry = reply.entry(self.operator)?;
        if reply.token == PieToken::NONE {
            return Err(api::Fail::Unknown);
        }
        Ok(GrantInfo {
            id: api::GrantId(reply.grant),
            entry,
            entrance: reply.token,
        })
    }

    pub fn inspect_grant(
        &self,
        anchor: api::Entry,
        id: api::GrantId,
        wait: Wait,
    ) -> Result<api::GrantState, api::Fail> {
        coordinates(self.operator, anchor)?;
        self.call(at(entry::INSPECT, anchor, id.0), wait)?
            .grant_state()
    }

    pub fn revoke(
        &self,
        anchor: api::Entry,
        id: api::GrantId,
        wait: Wait,
    ) -> Result<api::GrantState, api::Fail> {
        coordinates(self.operator, anchor)?;
        self.call(at(entry::REVOKE, anchor, id.0), wait)?
            .grant_state()
    }

    /// Retries cleanup of imports whose pre-send revocation previously failed.
    pub fn retry_import_cleanup(&self) -> Result<(), api::Fail> {
        let mut pending = self
            .pending_import_cleanup
            .try_borrow_mut()
            .map_err(|_| api::Fail::Unjudged)?;
        let mut index = 0;
        while index < pending.len() {
            if env::pie::revoke(self.operator, pending[index]).is_ok() {
                pending.swap_remove(index);
            } else {
                index += 1;
            }
        }
        if pending.is_empty() {
            Ok(())
        } else {
            Err(api::Fail::Unjudged)
        }
    }

    fn reserve_import_cleanup(&self) -> Result<(), api::Fail> {
        let _ = self.retry_import_cleanup();
        let mut pending = self
            .pending_import_cleanup
            .try_borrow_mut()
            .map_err(|_| api::Fail::Unjudged)?;
        if pending.len() >= MAX_PENDING_IMPORT_CLEANUP {
            return Err(api::Fail::Full);
        }
        pending.try_reserve(1).map_err(|_| api::Fail::Full)
    }

    fn staged_import(
        &self,
        result: Result<Response, (api::Fail, bool)>,
        imported: PieToken,
    ) -> Result<Response, api::Fail> {
        let pending = &self.pending_import_cleanup;
        revoke_unaccepted(
            result,
            self.operator,
            imported,
            |operator, seed| env::pie::revoke(operator, seed).map_err(|error| error.source),
            |seed| pending.borrow_mut().push(seed),
        )
    }

    pub fn inspect_operation(
        &self,
        anchor: api::Entry,
        id: api::OperationId,
        wait: Wait,
    ) -> Result<api::OperationState, api::Fail> {
        if id.0 == 0 {
            return Err(api::Fail::Denied);
        }
        coordinates(self.operator, anchor)?;
        self.call(at(entry::INSPECT_OPERATION, anchor, id.0), wait)?
            .operation_state()
    }

    pub fn cancel_operation(
        &self,
        anchor: api::Entry,
        id: api::OperationId,
        wait: Wait,
    ) -> Result<api::OperationState, api::Fail> {
        if id.0 == 0 {
            return Err(api::Fail::Denied);
        }
        coordinates(self.operator, anchor)?;
        self.call(at(entry::CANCEL_OPERATION, anchor, id.0), wait)?
            .operation_state()
    }

    fn call(&self, request: Request, wait: Wait) -> Result<Response, api::Fail> {
        self.call_staged(request, wait).map_err(|(fail, _)| fail)
    }

    fn call_staged(&self, mut request: Request, wait: Wait) -> Result<Response, (api::Fail, bool)> {
        if !matches!(reserve(self.entrance), Ok((_, owner, mark)) if owner == self.operator && mark == entry::AUTH_ENTRY)
        {
            return Err((api::Fail::Dead, false));
        }
        let reply = self
            .call
            .send(Deadline::new(wait), |back| {
                request.back = back;
                request
            })
            .map_err(|_| (api::Fail::Unknown, false))?;
        reply.receive().map_err(|_| (api::Fail::Unknown, true))
    }

    fn bound(&self, reply: Response) -> Result<api::BoundResource, api::Fail> {
        let binding = api::Binding {
            entry: reply.entry(self.operator)?,
            revision: reply.revision,
        };
        let view = api::ResourceView::from_bits(reply.view).ok_or(api::Fail::Unknown)?;
        if reply.token == PieToken::NONE {
            return Err(api::Fail::Unknown);
        }
        Ok(api::BoundResource {
            binding,
            resource: reply.token,
            view,
            permit: reply.permit,
        })
    }
}

pub(super) fn revoke_unaccepted<T, E>(
    result: Result<T, (api::Fail, bool)>,
    operator: TaskId,
    imported: PieToken,
    revoke: impl FnOnce(TaskId, PieToken) -> Result<(), E>,
    defer: impl FnOnce(PieToken),
) -> Result<T, api::Fail> {
    match result {
        Ok(value) => Ok(value),
        Err((fail, accepted)) => {
            if !accepted {
                if revoke(operator, imported).is_err() {
                    defer(imported);
                }
            }
            Err(fail)
        }
    }
}

pub(super) fn valid_descriptor(info: &env::PieInfo, issuer: TaskId, mark: Mark) -> bool {
    info.alive
        && info.owner == issuer
        && info.vestor == issuer
        && info.kind == env::PieKind::Hole
        && info.mark == mark
        && info.permission == Permission::FETCH
}

fn id(entry: api::Entry) -> api::EntryId {
    match entry.target {
        api::EntryTarget::Root => api::EntryId::new(0),
        api::EntryTarget::Node(id) => id,
    }
}
fn root(entry: api::Entry) -> u8 {
    u8::from(matches!(entry.target, api::EntryTarget::Root))
}
pub(super) fn coordinates(
    operator: TaskId,
    entry: api::Entry,
) -> Result<(api::EntryId, u8), api::Fail> {
    if entry.operator != operator {
        return Err(api::Fail::Denied);
    }
    Ok((id(entry), root(entry)))
}
pub(super) fn at(op: u8, anchor: api::Entry, grant: u64) -> Request {
    Request {
        op,
        anchor: id(anchor),
        root: root(anchor),
        id: grant,
        parent: 0,
        holder: TaskId::new(0),
        ops: 0,
        permit: api::Permit::Public,
        permit_restricted: 0,
        source: TaskId::new(0),
        source_mark: 0,
        resource: PieToken::NONE,
        resource_id: 0,
        view: 0,
        expected: 0,
        name: String::new(),
        scope: String::new(),
        back: PieToken::NONE,
    }
}
pub(super) fn encode_scope(scope: &api::Scope) -> Option<String> {
    match scope {
        api::Scope::Subtree => Some(String::from("*")),
        api::Scope::Names(names) => {
            if names.is_empty() {
                return None;
            }
            let mut size = 1usize;
            for name in names {
                let value = name.as_str();
                if value.is_empty() {
                    return None;
                }
                let mut digits = 1;
                let mut remaining = value.len();
                while remaining >= 10 {
                    digits += 1;
                    remaining /= 10;
                }
                size = size.checked_add(digits + 1 + value.len())?;
                if size > u8::MAX as usize {
                    return None;
                }
            }
            let mut out = String::new();
            out.try_reserve(size).ok()?;
            out.push('=');
            for name in names {
                let value = name.as_str();
                out.push_str(&alloc::format!("{}:{}", value.len(), value));
            }
            Some(out)
        }
    }
}
