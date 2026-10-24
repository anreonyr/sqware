use alloc::vec::Vec;
use env::Wait;
use hub_api::{BindingState, DmaSaid, Fail, Failure, PARTY_HUB};
use hub_api::coordination::{self, Prepare, Release, Wire};

use crate::service::hub::core::Ledger;
use super::binding_rpc::BindingRpc;
use super::execution::Executions;
use super::peers::Peers;

const MS: usize = 3000;

pub(super) fn advance(
    ledger: &mut Ledger,
    peers: &Peers,
    executions: &mut Executions,
    pending: &mut Vec<BindingRpc>,
) {
    let mut at = 0;
    while at < pending.len() {
        let result = pending[at].pending.poll();
        if matches!(result, Ok(None)) {
            at += 1;
            continue;
        }
        let request = pending.swap_remove(at);
        if !ledger.binding(request.binding.serial).is_some_and(|binding| {
            binding.id == request.binding && binding.open()
        }) {
            continue;
        }
        match result {
            Ok(Some(said)) => receive(ledger, executions, request, said),
            Err(_) => note(ledger, request.binding.serial, request.op, 0),
            Ok(None) => unreachable!(),
        }
    }

    let serials: Vec<_> = ledger.open_bindings_all().map(|binding| binding.id.serial).collect();
    for serial in serials {
        let binding = ledger.binding(serial).expect("open binding snapshot");
        if binding.state == BindingState::Closing {
            executions.closed(ledger, binding.execution);
        }
        let binding = ledger.binding(serial).expect("retained binding");
        if pending.iter().any(|request| request.binding == binding.id) {
            continue;
        }
        let op = match binding.state {
            BindingState::Preparing => coordination::PREPARE,
            BindingState::Closing if executions.target_received(binding.execution) => coordination::RELEASE,
            _ => continue,
        };
        if pending.len() >= hub_api::INVENTORY_MAX {
            continue;
        }
        if pending.try_reserve(1).is_err() {
            note(ledger, serial, op, hub_api::fail_to_code(Some(Fail::Full)));
            continue;
        }
        let id = binding.id;
        let execution = binding.execution;
        let key = binding.device.key;
        let reason = binding.reason.code();
        if binding.device.source != peers.coord.host() {
            note(ledger, serial, op, hub_api::fail_to_code(Some(Fail::Denied)));
            continue;
        }
        match peers.coord.begin(Wait::AtMost(MS), |back| {
            if op == coordination::PREPARE {
                Wire::Prepare(Prepare { op, hub: id.hub, serial, key, execution, back })
            } else {
                Wire::Release(Release { op, hub: id.hub, serial, key, execution, reason, back })
            }
        }) {
            Ok(request) => pending.push(BindingRpc { binding: id, op, pending: request }),
            Err(_) => note(ledger, serial, op, 0),
        }
    }
}

fn receive(ledger: &mut Ledger, executions: &mut Executions, request: BindingRpc, said: DmaSaid) {
    let serial = request.binding.serial;
    if let Some(failure) = said.failure() {
        ledger.note_failure(serial, failure);
    }
    if said.status != wire::OK {
        if said.failure().is_none() && said.status != hub_api::fail_to_code(Some(Fail::Full)) {
            note(ledger, serial, request.op, said.status);
        }
        return;
    }
    if said.serial != serial {
        note(ledger, serial, request.op, hub_api::fail_to_code(Some(Fail::Bad)));
        return;
    }
    let Some(state) = BindingState::of_code(said.state) else {
        note(ledger, serial, request.op, hub_api::fail_to_code(Some(Fail::Bad)));
        return;
    };
    let binding = ledger.binding(serial).expect("validated binding reply");
    if request.op == coordination::PREPARE && binding.state == BindingState::Preparing
        && matches!(state, BindingState::Preparing | BindingState::Active)
        && said.client != env::PieToken::NONE
    {
        if binding.client.is_some_and(|client| client != said.client) {
            note(ledger, serial, request.op, hub_api::fail_to_code(Some(Fail::Conflict)));
            return;
        }
        ledger.binding_mut(serial).unwrap().client = Some(said.client);
    }
    let binding = ledger.binding(serial).expect("validated binding reply");
    match (request.op, state) {
        (coordination::PREPARE, BindingState::Active) if binding.state == BindingState::Preparing => {
            ledger.binding_mut(serial).unwrap().state = BindingState::Active;
            crate::debug::put(&alloc::format!("hub: binding {serial} active"));
        }
        (coordination::RELEASE, BindingState::Closed) if binding.state == BindingState::Closing => {
            let execution = binding.execution;
            let key = binding.device.key;
            ledger.finish(serial);
            executions.closed(ledger, execution);
            crate::debug::put(&alloc::format!("hub: binding {serial} closed and released device {key}"));
        }
        (coordination::PREPARE, BindingState::Closing) if binding.state == BindingState::Closing => {}
        (coordination::PREPARE, BindingState::Preparing | BindingState::Active)
        | (coordination::RELEASE, BindingState::Closing) => {}
        _ => note(ledger, serial, request.op, hub_api::fail_to_code(Some(Fail::Bad))),
    }
}

fn note(ledger: &mut Ledger, serial: u64, op: u8, code: u8) {
    if ledger.binding(serial).is_some_and(|binding| binding.failure.is_none()) {
        crate::debug::put(&alloc::format!("hub: binding {serial} coordination failed op={op} code={code} (holding)"));
    }
    ledger.note_failure(serial, Failure { party: PARTY_HUB, op, code });
}
