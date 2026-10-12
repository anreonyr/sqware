use super::super::authority::GrantRef;
use crate::system::operator::tree::Operator;
use ::resource::pile::Pile;
use ::resource::port::{self, Access, Policy};
use ::resource::raw::Hole;
use ::schedule::{Progress, Res, ResMut};
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use env::{MailCondition, PieToken, TaskId, Wait};
use ipc::rpc::request::Receiver;
use system_api::operator as api;
use system_api::operator::entry;

struct Call<'a> {
    tree: &'a mut Operator,
    pile: &'a Pile,
    via: PieToken,
    parent_key: super::super::authority::GrantKey,
    parent_id: api::GrantId,
    caller: TaskId,
    changes: Vec<crate::system::operator::tree::Change>,
}

struct Check {
    key: super::super::authority::GrantKey,
    caller: TaskId,
    anchor: api::Entry,
    op: api::Ops,
    name: alloc::string::String,
}

pub(super) fn handle(
    hit: Res<super::exchange::Hit>,
    mut tree: ResMut<Operator>,
    pile: Res<Pile>,
) -> Result<Progress, super::Fail> {
    tree.retry_import_cleanup();
    let Some(token) = hit.0 else {
        return Ok(Progress::Done);
    };
    let Some(_) = tree.authority_id(token) else {
        return Ok(Progress::Done);
    };
    let receiver = Receiver::<entry::Call>::from_raw(token, entry::AUTH_BACK, entry::reply_to);
    let mut frame = [0; entry::Request::LEN];
    let Ok(incoming) = receiver.receive(&mut frame, Wait::POLL) else {
        return Ok(Progress::Done);
    };
    let response = if let Some((actual_key, id, _)) = tree.grant_at(token, incoming.from) {
        let mut call = Call {
            tree: &mut tree,
            pile: &pile,
            via: token,
            parent_key: actual_key,
            parent_id: id,
            caller: incoming.from,
            changes: Vec::new(),
        };
        let response = execute(&mut call, incoming.request);
        let changes = core::mem::take(&mut call.changes);
        call.tree.queue_changes(changes);
        response
    } else {
        entry::Response::fail(api::Fail::Denied)
    };
    let _ = incoming.reply.send(response);
    Ok(Progress::Done)
}

fn execute(call: &mut Call<'_>, request: entry::Request) -> entry::Response {
    use entry::*;
    let caller = call.caller;
    if request.root > 1 || request.permit_restricted > 1 {
        return entry::Response::fail(api::Fail::Denied);
    }
    if matches!(
        request.op,
        PART | BIND | INSPECT_OPERATION | CANCEL_OPERATION
    ) && request.id == 0
    {
        return entry::Response::fail(api::Fail::Denied);
    }
    if matches!(request.op, PART | BIND) && !valid_name(&request.name) {
        return entry::Response::fail(api::Fail::Denied);
    }
    if matches!(request.op, BIND | REPLACE) && api::ResourceView::from_bits(request.view).is_none()
    {
        return entry::Response::fail(api::Fail::Denied);
    }
    let target = if request.root == 1 {
        api::EntryTarget::Root
    } else {
        api::EntryTarget::Node(request.anchor)
    };
    let anchor = api::Entry {
        operator: call.tree.operator_task(),
        target,
    };
    let authorized = match request.op {
        PART => authorize(
            call,
            Check {
                key: call.parent_key,
                caller,
                anchor,
                op: api::Ops::PART,
                name: request.name.clone(),
            },
        ),
        UNPART => authorize(
            call,
            Check {
                key: call.parent_key,
                caller,
                anchor,
                op: api::Ops::UNPART,
                name: String::new(),
            },
        ),
        BIND => authorize(
            call,
            Check {
                key: call.parent_key,
                caller,
                anchor,
                op: api::Ops::BIND,
                name: request.name.clone(),
            },
        ),
        REPLACE => authorize(
            call,
            Check {
                key: call.parent_key,
                caller,
                anchor,
                op: api::Ops::REPLACE,
                name: String::new(),
            },
        ),
        UNBIND => authorize(
            call,
            Check {
                key: call.parent_key,
                caller,
                anchor,
                op: api::Ops::UNBIND,
                name: String::new(),
            },
        ),
        GRANT => true,
        INSPECT => authorize(
            call,
            Check {
                key: call.parent_key,
                caller,
                anchor,
                op: api::Ops::INSPECT,
                name: String::new(),
            },
        ),
        REVOKE => authorize(
            call,
            Check {
                key: call.parent_key,
                caller,
                anchor,
                op: api::Ops::REVOKE,
                name: String::new(),
            },
        ),
        FIND => authorize(
            call,
            Check {
                key: call.parent_key,
                caller,
                anchor,
                op: api::Ops::FIND,
                name: request.name.clone(),
            },
        ),
        READ => authorize(
            call,
            Check {
                key: call.parent_key,
                caller,
                anchor,
                op: api::Ops::FIND,
                name: String::new(),
            },
        ),
        CANCEL_OPERATION | INSPECT_OPERATION => authorize(
            call,
            Check {
                key: call.parent_key,
                caller,
                anchor,
                op: api::Ops::CANCEL,
                name: String::new(),
            },
        ),
        _ => false,
    };
    if !authorized {
        return entry::Response::fail(api::Fail::Denied);
    }
    match request.op {
        PART => {
            let operation = crate::system::operator::tree::OperationRequest {
                via: call.via,
                caller,
                id: api::OperationId(request.id),
                op: request.op,
                anchor,
                name: request.name,
                resource_id: 0,
                permit: api::Permit::Public,
                view: env::Access::NONE,
            };
            match call.tree.begin_operation(operation.clone()) {
                Ok(Some(result)) => operation_response(result),
                Ok(None) => {
                    let result = match call.tree.part_entry_with_authority(
                        crate::system::operator::tree::PartEntry {
                            anchor,
                            name: operation.name.clone(),
                            authority: Some(call.parent_key),
                        },
                    ) {
                        Ok((entry, created, change)) => {
                            if let Some(change) = change {
                                call.changes.push(change);
                            }
                            crate::system::operator::tree::OperationResult::Part { entry, created }
                        }
                        Err(fail) => crate::system::operator::tree::OperationResult::Failed(fail),
                    };
                    if call.tree.finish_operation(&operation, result).is_err() {
                        return entry::Response::fail(api::Fail::Dead);
                    }
                    operation_response(result)
                }
                Err(fail) => entry::Response::fail(fail),
            }
        }
        UNPART => match call.tree.unpart(anchor) {
            Ok(change) => {
                if let Some(change) = change {
                    call.changes.push(change);
                }
                ok_entry(anchor)
            }
            Err(fail) => entry::Response::fail(fail),
        },
        BIND => bind(call, anchor, request),
        REPLACE => replace(call, anchor, request),
        UNBIND => {
            let binding = binding(anchor, request.anchor, request.expected);
            match call.tree.unbind_binding(binding) {
                Ok(change) => {
                    if let Some(change) = change {
                        call.changes.push(change);
                    }
                    ok_entry(anchor)
                }
                Err(fail) => entry::Response::fail(fail),
            }
        }
        GRANT => grant(call, request),
        INSPECT => grant_state(call, api::GrantId(request.id)),
        REVOKE => revoke(call, api::GrantId(request.id)),
        FIND => find_resource(
            call,
            Find {
                anchor,
                name: request.name,
                expected_permit: request.permit,
            },
        ),
        READ => read_resource(call, anchor, request.permit),
        INSPECT_OPERATION => operation_state(
            call,
            crate::system::operator::tree::OperationRef {
                via: call.via,
                caller,
                id: api::OperationId(request.id),
            },
        ),
        CANCEL_OPERATION => cancel_operation(
            call,
            crate::system::operator::tree::OperationRef {
                via: call.via,
                caller,
                id: api::OperationId(request.id),
            },
        ),
        _ => entry::Response::fail(api::Fail::Unknown),
    }
}

fn bind(call: &mut Call<'_>, anchor: api::Entry, request: entry::Request) -> entry::Response {
    let imported = request.resource;
    if call.tree.reserve_import_cleanup().is_err() {
        let _ = call.tree.release_import(imported);
        return entry::Response::fail(api::Fail::Full);
    }
    if request.resource_id == 0 {
        let _ = call.tree.release_import(imported);
        return entry::Response::fail(api::Fail::Denied);
    }
    let Some(view) = api::ResourceView::from_bits(request.view) else {
        let _ = call.tree.release_import(imported);
        return entry::Response::fail(api::Fail::Denied);
    };
    let Some(resource) = import_resource(call.caller, request.resource, view) else {
        let _ = call.tree.release_import(imported);
        return entry::Response::fail(api::Fail::Denied);
    };
    let Some((_, spec)) = call.tree.grant_at_by_id(call.parent_key, call.caller) else {
        let _ = call.tree.release_import(imported);
        return entry::Response::fail(api::Fail::Denied);
    };
    let Ok(info) = ::resource::raw::inspect(resource) else {
        let _ = call.tree.release_import(imported);
        return entry::Response::fail(api::Fail::Dead);
    };
    if spec.permit.is_some_and(|permit| request.permit != permit)
        || spec.source.is_some_and(|source| source != info.owner)
        || spec
            .source_mark
            .is_some_and(|source_mark| source_mark != info.mark)
    {
        let _ = call.tree.release_import(imported);
        return entry::Response::fail(api::Fail::Denied);
    }
    let operation = crate::system::operator::tree::OperationRequest {
        via: call.via,
        caller: call.caller,
        id: api::OperationId(request.id),
        op: request.op,
        anchor,
        name: request.name.clone(),
        resource_id: request.resource_id,
        permit: request.permit,
        view: view.access(),
    };
    match call.tree.begin_operation(operation.clone()) {
        Ok(Some(result)) => {
            if call.tree.release_import(imported).is_err() {
                return entry::Response::fail(api::Fail::Dead);
            }
            operation_response(result)
        }
        Ok(None) => {
            let result = match call.tree.bind_at(crate::system::operator::tree::BindEntry {
                anchor,
                name: request.name,
                pie: resource,
                resource_id: request.resource_id,
                permit: request.permit,
                owner: call.caller,
                authority: call.parent_key,
                view: view.access(),
            }) {
                Ok((binding, change)) => {
                    let created = change.is_some();
                    if let Some(change) = change {
                        call.changes.push(change);
                    }
                    crate::system::operator::tree::OperationResult::Bind { binding, created }
                }
                Err(fail) => crate::system::operator::tree::OperationResult::Failed(fail),
            };
            if call.tree.finish_operation(&operation, result).is_err() {
                return entry::Response::fail(api::Fail::Dead);
            }
            if !matches!(
                result,
                crate::system::operator::tree::OperationResult::Bind { created: true, .. }
            ) && call.tree.release_import(imported).is_err()
            {
                return entry::Response::fail(api::Fail::Dead);
            }
            operation_response(result)
        }
        Err(fail) => {
            if call.tree.release_import(imported).is_err() {
                return entry::Response::fail(api::Fail::Dead);
            }
            entry::Response::fail(fail)
        }
    }
}

struct Find {
    anchor: api::Entry,
    name: String,
    expected_permit: api::Permit,
}

fn find_resource(call: &mut Call<'_>, find: Find) -> entry::Response {
    let Find {
        anchor,
        name,
        expected_permit,
    } = find;
    let Ok((binding, resource, view)) = call.tree.binding_at(anchor, &name) else {
        return entry::Response::fail(api::Fail::Unknown);
    };
    let Some((_, spec)) = call.tree.grant_at_by_id(call.parent_key, call.caller) else {
        return entry::Response::fail(api::Fail::Denied);
    };
    let Some(relative) = call.tree.relative_to(spec.anchor, anchor) else {
        return entry::Response::fail(api::Fail::Denied);
    };
    let relative = if relative.as_str().is_empty() {
        name
    } else {
        alloc::format!("{}/{}", relative.as_str(), name)
    };
    let permit = call.tree.permit(binding.entry_id());
    if !spec.scope.allows(&relative) || permit != expected_permit || !permit_allows(call, permit) {
        return entry::Response::fail(api::Fail::Denied);
    }
    match port::ship(resource, call.caller, view.access(), Policy::NONE) {
        Ok(remote) => entry::Response {
            status: api::OK,
            state: 0,
            root: 0,
            entry: binding.entry_id(),
            revision: binding.revision,
            grant: 0,
            token: remote.seed(),
            view: view.bits(),
            permit,
        },
        Err(_) => entry::Response::fail(api::Fail::Dead),
    }
}

fn read_resource(
    call: &mut Call<'_>,
    anchor: api::Entry,
    expected_permit: api::Permit,
) -> entry::Response {
    let id = match anchor.target {
        api::EntryTarget::Node(id) => id,
        api::EntryTarget::Root => return entry::Response::fail(api::Fail::NotATile),
    };
    let Ok(binding) = call.tree.binding(id) else {
        return entry::Response::fail(api::Fail::Unknown);
    };
    let Some((_, spec)) = call.tree.grant_at_by_id(call.parent_key, call.caller) else {
        return entry::Response::fail(api::Fail::Denied);
    };
    let Some(relative) = call.tree.relative_to(spec.anchor, anchor) else {
        return entry::Response::fail(api::Fail::Denied);
    };
    let permit = call.tree.permit(id);
    if !spec.scope.allows(relative.as_str())
        || permit != expected_permit
        || !permit_allows(call, permit)
    {
        return entry::Response::fail(api::Fail::Denied);
    }
    let mut token = PieToken::NONE;
    let mut view = None;
    if let Err(fail) = call.tree.find(id, |pie, access| {
        token = pie;
        view = api::ResourceView::new(access);
    }) {
        return entry::Response::fail(fail);
    }
    let Some(view) = view else {
        return entry::Response::fail(api::Fail::Denied);
    };
    match port::ship(token, call.caller, view.access(), Policy::NONE) {
        Ok(remote) => entry::Response {
            status: api::OK,
            state: 0,
            root: 0,
            entry: binding.entry_id(),
            revision: binding.revision,
            grant: 0,
            token: remote.seed(),
            view: view.bits(),
            permit,
        },
        Err(_) => entry::Response::fail(api::Fail::Dead),
    }
}

fn permit_allows(call: &Call<'_>, permit: api::Permit) -> bool {
    use api::Permit;
    match permit {
        Permit::Public => true,
        Permit::Opener(entry) => call.tree.opens(entry).is_ok_and(|task| task == call.caller),
        Permit::Bound => call.tree.identity_query().is_some_and(|query| {
            query
                .resolve(call.caller, Wait::AtMost(1000))
                .is_ok_and(|binding| binding.is_some())
        }),
        Permit::Identity(selector) => call.tree.identity_query().is_some_and(|query| {
            query
                .matches(call.caller, selector, Wait::AtMost(1000))
                .is_ok_and(|answer| answer == system_api::identity::Match::Yes)
        }),
    }
}

fn replace(call: &mut Call<'_>, anchor: api::Entry, request: entry::Request) -> entry::Response {
    let imported = request.resource;
    let replay = call
        .tree
        .replace_replay(&crate::system::operator::tree::ReplaceEntry {
            binding: binding(anchor, request.anchor, request.expected),
            pie: request.resource,
            resource_id: request.resource_id,
            permit: request.permit,
            owner: call.caller,
            authority: call.parent_key,
            view: api::ResourceView::from_bits(request.view)
                .map_or(env::Access::NONE, |view| view.access()),
        });
    if call.tree.reserve_import_cleanup().is_err() {
        let _ = call.tree.release_import(imported);
        return entry::Response::fail(api::Fail::Full);
    }
    let response = replace_checked(call, anchor, request);
    if response.status != api::OK {
        if call.tree.release_import(imported).is_err() {
            return entry::Response::fail(api::Fail::Dead);
        }
    } else if replay && call.tree.release_import(imported).is_err() {
        return entry::Response::fail(api::Fail::Dead);
    }
    response
}

fn replace_checked(
    call: &mut Call<'_>,
    anchor: api::Entry,
    request: entry::Request,
) -> entry::Response {
    if request.resource_id == 0 {
        return entry::Response::fail(api::Fail::Denied);
    }
    let Some(view) = api::ResourceView::from_bits(request.view) else {
        return entry::Response::fail(api::Fail::Denied);
    };
    let Some(resource) = import_resource(call.caller, request.resource, view) else {
        return entry::Response::fail(api::Fail::Denied);
    };
    let Some((_, spec)) = call.tree.grant_at_by_id(call.parent_key, call.caller) else {
        return entry::Response::fail(api::Fail::Denied);
    };
    let Ok(info) = ::resource::raw::inspect(resource) else {
        return entry::Response::fail(api::Fail::Dead);
    };
    if spec.permit.is_some_and(|permit| request.permit != permit)
        || spec.source.is_some_and(|source| source != info.owner)
        || spec
            .source_mark
            .is_some_and(|source_mark| source_mark != info.mark)
    {
        return entry::Response::fail(api::Fail::Denied);
    }
    match call
        .tree
        .replace_binding(crate::system::operator::tree::ReplaceEntry {
            binding: binding(anchor, request.anchor, request.expected),
            pie: resource,
            resource_id: request.resource_id,
            permit: request.permit,
            owner: call.caller,
            authority: call.parent_key,
            view: view.access(),
        }) {
        Ok((binding, change)) => {
            if let Some(change) = change {
                call.changes.push(change);
            }
            entry::Response {
                status: api::OK,
                state: 0,
                root: 0,
                entry: binding.entry_id(),
                revision: binding.revision,
                grant: 0,
                token: PieToken::NONE,
                view: 0,
                permit: request.permit,
            }
        }
        Err(fail) => entry::Response::fail(fail),
    }
}

fn import_resource(
    caller: TaskId,
    resource: PieToken,
    view: api::ResourceView,
) -> Option<PieToken> {
    let info = ::resource::raw::inspect(resource).ok()?;
    if !info.alive
        || info.vestor != caller
        || resource == PieToken::NONE
        || !view.fits(info.permission)
    {
        return None;
    }
    Some(resource)
}

fn grant(call: &mut Call<'_>, request: entry::Request) -> entry::Response {
    let tree = &mut *call.tree;
    let pile = call.pile;
    let via = call.via;
    let parent_key = call.parent_key;
    let parent = call.parent_id;
    let caller = call.caller;
    let child_anchor = api::Entry {
        operator: tree.operator_task(),
        target: if request.root == 1 {
            api::EntryTarget::Root
        } else {
            api::EntryTarget::Node(request.anchor)
        },
    };
    let scope = match decode_scope(&request.scope) {
        Some(scope) => scope,
        None => return entry::Response::fail(api::Fail::Denied),
    };
    if request.parent != parent.0 {
        return entry::Response::fail(api::Fail::Denied);
    }
    let spec = api::GrantSpec {
        id: api::GrantId(request.id),
        holder: request.holder,
        anchor: child_anchor,
        scope,
        ops: match api::Ops::from_bits(request.ops) {
            Some(ops) => ops,
            None => return entry::Response::fail(api::Fail::Denied),
        },
        permit: (request.permit_restricted == 1).then_some(request.permit),
        source: (request.source.get() != 0).then_some(request.source),
        source_mark: (request.source_mark != 0).then_some(env::Mark::new(request.source_mark)),
    };
    let Some((_, parent_spec)) = tree.grant_at_by_id(parent_key, caller) else {
        return entry::Response::fail(api::Fail::Denied);
    };
    let Some(relative) = tree.relative_to(parent_spec.anchor, child_anchor) else {
        return entry::Response::fail(api::Fail::Denied);
    };
    let scope_contains_anchor = if child_anchor == parent_spec.anchor {
        parent_spec.scope.narrows(&spec.scope)
    } else {
        parent_spec.scope.narrows_at(relative.as_str(), &spec.scope)
    };
    if !parent_spec.ops.contains(api::Ops::GRANT)
        || !parent_spec.ops.contains(spec.ops)
        || !scope_contains_anchor
        || parent_spec
            .permit
            .is_some_and(|permit| spec.permit != Some(permit))
        || parent_spec
            .source
            .is_some_and(|source| spec.source != Some(source))
        || parent_spec
            .source_mark
            .is_some_and(|mark| spec.source_mark != Some(mark))
    {
        return entry::Response::fail(api::Fail::Denied);
    }
    let (state, grant_key) = match tree.grant(api::GrantSubmission {
        caller,
        via,
        parent: Some(parent),
        spec: spec.clone(),
    }) {
        Ok(result) => result,
        Err(_) => return entry::Response::fail(api::Fail::Denied),
    };
    if !matches!(state, api::GrantState::Pending | api::GrantState::Granted) {
        return entry::Response::fail(api::Fail::Denied);
    }
    let grant_ref = GrantRef {
        via,
        caller,
        id: spec.id,
    };
    if state == api::GrantState::Granted {
        if !grant_active(tree, grant_ref) {
            return entry::Response::fail(api::Fail::Dead);
        }
        return tree.grant_remote(grant_key).map_or_else(
            || entry::Response::fail(api::Fail::Dead),
            |remote| grant_response(spec, child_anchor, remote),
        );
    }
    let local = match tree.entrance(grant_ref) {
        Some(local) => local,
        None => {
            let entrance = match Hole::unseal(entry::AUTH_ENTRY) {
                Ok(hole) => hole,
                Err(_) => {
                    let _ = tree.revoke_grant(grant_ref);
                    let _ = tree.grant_closed(grant_ref);
                    return entry::Response::fail(api::Fail::Full);
                }
            };
            let local = entrance.token();
            if tree.set_entrance(grant_ref, local).is_err() {
                let _ = env::pie::seal(local);
                let _ = env::pie::release(local, env::ReleaseMode::Revoke);
                let _ = tree.revoke_grant(grant_ref);
                let _ = tree.grant_closed(grant_ref);
                return entry::Response::fail(api::Fail::Denied);
            }
            local
        }
    };
    let remote = match port::ship(local, spec.holder, Access::STORE, Policy::NONE) {
        Ok(remote) => remote.seed(),
        Err(_) => {
            let _ = revoke(call, spec.id);
            return entry::Response::fail(api::Fail::Denied);
        }
    };
    if !tree
        .cleanup_progress(grant_key)
        .is_some_and(|progress| progress.0)
    {
        if pile
            .attach(env::Source::Mail {
                pie: local,
                condition: MailCondition::Pull,
            })
            .is_err()
            || tree.grant_attached(grant_key).is_err()
        {
            let _ = revoke(call, spec.id);
            return entry::Response::fail(api::Fail::Denied);
        }
    }
    if tree.grant_delivered(grant_ref, remote).is_err() {
        let _ = revoke(call, spec.id);
        return entry::Response::fail(api::Fail::Denied);
    }
    grant_response(spec, child_anchor, remote)
}

fn grant_response(spec: api::GrantSpec, anchor: api::Entry, token: PieToken) -> entry::Response {
    entry::Response {
        status: api::OK,
        state: 2,
        root: u8::from(matches!(anchor.target, api::EntryTarget::Root)),
        entry: match anchor.target {
            api::EntryTarget::Root => api::EntryId::new(0),
            api::EntryTarget::Node(id) => id,
        },
        revision: 0,
        grant: spec.id.0,
        token,
        view: 0,
        permit: api::Permit::Public,
    }
}

fn grant_state(call: &Call<'_>, id: api::GrantId) -> entry::Response {
    let tree = &*call.tree;
    let via = call.via;
    let caller = call.caller;
    let key = GrantRef { via, caller, id };
    let state = tree.inspect_grant(key).unwrap_or(api::GrantState::Unseen);
    if state == api::GrantState::Granted && id.0 != 0 && !grant_active(tree, key) {
        return entry::Response::fail(api::Fail::Dead);
    }
    entry::Response {
        status: api::OK,
        state: state as u8,
        root: 0,
        entry: api::EntryId::new(0),
        revision: 0,
        grant: id.0,
        token: PieToken::NONE,
        view: 0,
        permit: api::Permit::Public,
    }
}

fn grant_active(tree: &Operator, key: GrantRef) -> bool {
    let Some(entrance) = tree.entrance(key) else {
        return false;
    };
    let Ok(info) = ::resource::raw::inspect(entrance) else {
        return false;
    };
    info.alive
        && info.owner == tree.operator_task()
        && info.kind == env::PieKind::Hole
        && info.mark == entry::AUTH_ENTRY
        && info
            .permission
            .contains(env::Permission::STORE | env::Permission::VEST)
        && ipc::session::alive(entrance)
}

fn revoke(call: &mut Call<'_>, id: api::GrantId) -> entry::Response {
    let tree = &mut *call.tree;
    let pile = call.pile;
    let via = call.via;
    let caller = call.caller;
    let key = GrantRef { via, caller, id };
    let Ok(state) = tree.revoke_grant(key) else {
        return entry::Response::fail(api::Fail::Denied);
    };
    if state == api::GrantState::Closed {
        return grant_state(call, id);
    }
    let Ok(plan) = tree.revoke_plan(key) else {
        return entry::Response::fail(api::Fail::Full);
    };
    for (grant_key, grant_via, issuer, grant_id, entrance) in plan {
        if let Some(token) = entrance {
            let Some((attached, detached, sealed, released)) = tree.cleanup_progress(grant_key)
            else {
                return entry::Response::fail(api::Fail::Dead);
            };
            if !detached {
                if attached
                    && pile
                        .detach(env::Source::Mail {
                            pie: token,
                            condition: MailCondition::Pull,
                        })
                        .is_err()
                {
                    return entry::Response::fail(api::Fail::Dead);
                }
                if tree.cleanup_step(grant_key, 0).is_err() {
                    return entry::Response::fail(api::Fail::Dead);
                }
            }
            if !sealed {
                if env::pie::seal(token).is_err() {
                    return entry::Response::fail(api::Fail::Dead);
                }
                if tree.cleanup_step(grant_key, 1).is_err() {
                    return entry::Response::fail(api::Fail::Dead);
                }
            }
            if !released {
                if env::pie::release(token, env::ReleaseMode::Revoke).is_err() {
                    return entry::Response::fail(api::Fail::Dead);
                }
                if tree.cleanup_step(grant_key, 2).is_err() {
                    return entry::Response::fail(api::Fail::Dead);
                }
            }
        }
        if tree.cleanup_authority(grant_key).is_err() {
            return entry::Response::fail(api::Fail::Dead);
        }
        tree.forget_operations(grant_via);
        if tree
            .grant_closed(GrantRef {
                via: grant_via,
                caller: issuer,
                id: grant_id,
            })
            .is_err()
        {
            return entry::Response::fail(api::Fail::Dead);
        }
    }
    grant_state(call, id)
}

fn authorize(call: &Call<'_>, check: Check) -> bool {
    let tree = &*call.tree;
    let Check {
        key,
        caller,
        anchor,
        op,
        name,
    } = check;
    let Some((_, spec)) = tree.grant_at_by_id(key, caller) else {
        return false;
    };
    if !spec.ops.contains(op) {
        return false;
    }
    let Some(base) = tree.relative_to(spec.anchor, anchor) else {
        return false;
    };
    let relative = if name.is_empty() {
        base.as_str().to_string()
    } else if base.is_empty() {
        name
    } else {
        alloc::format!("{}/{}", base.as_str(), name)
    };
    spec.scope.allows(&relative)
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 31
        && !name.contains('/')
        && name != "."
        && name != ".."
        && api::PathBuf::try_new(name).is_some_and(|path| path.as_str() == name)
}

fn binding(anchor: api::Entry, id: api::EntryId, revision: u64) -> api::Binding {
    api::Binding {
        entry: api::Entry {
            operator: anchor.operator,
            target: api::EntryTarget::Node(id),
        },
        revision,
    }
}

fn response_entry(entry: api::Entry) -> entry::Response {
    entry::Response {
        status: api::OK,
        state: 0,
        root: u8::from(matches!(entry.target, api::EntryTarget::Root)),
        entry: match entry.target {
            api::EntryTarget::Root => api::EntryId::new(0),
            api::EntryTarget::Node(id) => id,
        },
        revision: 0,
        grant: 0,
        token: PieToken::NONE,
        view: 0,
        permit: api::Permit::Public,
    }
}

fn operation_response(result: crate::system::operator::tree::OperationResult) -> entry::Response {
    match result {
        crate::system::operator::tree::OperationResult::Part { entry, created } => {
            let mut response = response_entry(entry);
            response.state = u8::from(created);
            response
        }
        crate::system::operator::tree::OperationResult::Bind { binding, created } => {
            entry::Response {
                status: api::OK,
                state: u8::from(created),
                root: 0,
                entry: binding.entry_id(),
                revision: binding.revision,
                grant: 0,
                token: PieToken::NONE,
                view: 0,
                permit: api::Permit::Public,
            }
        }
        crate::system::operator::tree::OperationResult::Failed(fail) => entry::Response::fail(fail),
    }
}

fn operation_state(
    call: &Call<'_>,
    request: crate::system::operator::tree::OperationRef,
) -> entry::Response {
    operation_state_response(call.tree.operation_state(request))
}

fn cancel_operation(
    call: &mut Call<'_>,
    request: crate::system::operator::tree::OperationRef,
) -> entry::Response {
    match call.tree.cancel_operation(request) {
        Ok(state) => operation_state_response(state),
        Err(fail) => entry::Response::fail(fail),
    }
}

fn operation_state_response(state: api::OperationState) -> entry::Response {
    let state = match state {
        api::OperationState::Unseen => 0,
        api::OperationState::Pending => 1,
        api::OperationState::Applied => 2,
        api::OperationState::Closed => 3,
        api::OperationState::Failed => 4,
    };
    entry::Response {
        status: api::OK,
        state,
        root: 0,
        entry: api::EntryId::new(0),
        revision: 0,
        grant: 0,
        token: PieToken::NONE,
        view: 0,
        permit: api::Permit::Public,
    }
}

fn ok_entry(entry: api::Entry) -> entry::Response {
    response_entry(entry)
}

fn decode_scope(value: &str) -> Option<api::Scope> {
    if value == "*" {
        return Some(api::Scope::Subtree);
    }
    let mut rest = value.strip_prefix('=')?;
    let mut names = alloc::vec::Vec::new();
    while !rest.is_empty() {
        let (len, tail) = rest.split_once(':')?;
        let len: usize = len.parse().ok()?;
        if len == 0 || len > tail.len() {
            return None;
        }
        let (name, next) = tail.split_at(len);
        names.try_reserve(1).ok()?;
        names.push(api::PathBuf::try_new(name)?);
        rest = next;
    }
    (!names.is_empty()).then_some(api::Scope::Names(names))
}
