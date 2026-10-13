use super::super::{CallError, IdentityName, IdentityObject, OperatorEntryRef};
use env::{Permission, PieKind, PieToken, TaskId, Wait};
use ipc::{rpc, time::Deadline};
use resource::raw::inspect;
use system_api::identity::export::{self as api, Call, Request};

/// A unit-scoped System endpoint for exporting one declared Operator binding.
pub struct Exporter {
    authority: TaskId,
    entry: PieToken,
}

#[derive(Clone, Copy)]
pub struct IdentityExport {
    pub object: IdentityObject,
    pub alias: IdentityName,
    pub binding: ExportBinding,
}

#[derive(Clone, Copy)]
pub struct ExportBinding {
    pub name: IdentityName,
    pub entry: OperatorEntryRef,
}

impl Exporter {
    pub fn direct(authority: TaskId, entry: PieToken) -> Result<Self, CallError> {
        let info = inspect(entry).map_err(|_| CallError::Transport)?;
        if authority.get() == 0
            || !info.alive
            || info.owner != authority
            || info.vestor != authority
            || info.permission != Permission::STORE
            || info.kind != PieKind::Hole
            || info.mark != api::ENTRY
        {
            return Err(CallError::WrongAuthority);
        }
        Ok(Self { authority, entry })
    }

    pub fn export(&self, export: IdentityExport, wait: Wait) -> Result<(), CallError> {
        let IdentityExport {
            object,
            alias,
            binding,
        } = export;
        let ExportBinding {
            name: binding,
            entry,
        } = binding;
        if object.authority().get() == 0
            || entry.operator.get() == 0
            || (entry.root && entry.id != 0)
        {
            return Err(CallError::Malformed);
        }
        let sender = rpc::request::Sender::<Call>::from_raw(self.entry, api::BACK)
            .map_err(|_| CallError::Transport)?;
        if sender.peer() != self.authority {
            return Err(CallError::WrongAuthority);
        }
        let response = sender
            .call(Deadline::new(wait), |back| Request {
                object,
                alias,
                binding,
                entry,
                back,
            })
            .map_err(|_| CallError::Transport)?;
        match response.status {
            api::OK => Ok(()),
            api::BAD => Err(CallError::Malformed),
            api::FULL => Err(CallError::Service(super::super::Fail::Full)),
            _ => Err(CallError::Service(super::super::Fail::Denied)),
        }
    }
}
