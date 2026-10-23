//! Revocation and cleanup of a namespace installation receipt.

use system_api::operator::{self as api, GrantState};

impl super::System {
    /// Revokes first, then removes only this install's exact binding revisions and entries.
    /// On any failure, the remaining receipt stays intact for a later retry.
    pub fn retire(&self, installed: &mut super::Installed) -> Result<bool, api::Fail> {
        if let Some(id) = installed.grant {
            // Grant IDs are issuer-local; the installed directory may already be absent.
            let state = self
                .capability
                .revoke(self.root, id, installed.wait)
                .map_err(|why| {
                    crate::debug::put(&alloc::format!(
                        "system: namespace revoke id={} issuer={:?} failed={why:?}",
                        id.0, self.root
                    ));
                    why
                })?;
            if state != GrantState::Closed {
                return Err(api::Fail::Dead);
            }
            installed.authority = None;
            installed.grant = None;
        }
        while let Some(binding) = installed.bindings.last().copied() {
            match self.capability.unbind(binding, installed.wait) {
                Ok(()) | Err(api::Fail::Unknown) => {
                    installed.bindings.pop();
                }
                Err(fail) => {
                    crate::debug::put(&alloc::format!(
                        "system: namespace unbind binding={binding:?} failed={fail:?}"
                    ));
                    return Err(fail);
                }
            }
        }
        while let Some(entry) = installed.entries.last().copied() {
            match self.capability.unpart(entry, installed.wait) {
                Ok(()) | Err(api::Fail::Unknown) => {
                    installed.entries.pop();
                }
                Err(fail) => {
                    crate::debug::put(&alloc::format!(
                        "system: namespace unpart entry={entry:?} failed={fail:?}"
                    ));
                    return Err(fail);
                }
            }
        }
        Ok(true)
    }
}
