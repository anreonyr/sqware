use alloc::sync::Arc;
use alloc::vec::Vec;
use env::{TaskId, TeamId};

use crate::core::work::unit::life::Life;
use crate::machine::sync::SpinLock;

use super::{Member, TeamPhase, TeamState};

pub(crate) struct TeamLife {
    pub id: TeamId,
    pub owner: TaskId,
    pub parent: Option<Arc<TeamLife>>,
    pub state: Arc<SpinLock<TeamState>>,
    pub tasks: Arc<SpinLock<Vec<Arc<Member>>>>,
    pub children: SpinLock<Vec<Arc<TeamLife>>>,
    pub life: Arc<Life>,
}

impl TeamLife {
    pub fn paused(&self) -> bool {
        let local = matches!(self.state.lock().phase, TeamPhase::Paused);
        local || self.parent.as_ref().is_some_and(|p| p.paused())
    }
    pub fn closed(&self) -> bool {
        let local = self.state.lock().terminal();
        local || self.parent.as_ref().is_some_and(|p| p.closed())
    }
    pub fn member(&self, id: TaskId) -> Option<Arc<Member>> {
        self.tasks.lock().iter().find(|m| m.id == id).cloned()
    }
    pub fn find_team(self: &Arc<Self>, id: TeamId) -> Option<Arc<Self>> {
        if self.id == id {
            return Some(self.clone());
        }
        let mut at = 0;
        loop {
            let child = self.children.lock().get(at).cloned()?;
            at += 1;
            if let Some(found) = child.find_team(id) {
                return Some(found);
            }
        }
    }
    pub fn visit(&self, run: &mut impl FnMut(&TeamLife)) {
        run(self);
        let mut after = 0;
        loop {
            let child = self
                .children
                .lock()
                .iter()
                .filter(|c| c.id.get() > after)
                .min_by_key(|c| c.id.get())
                .cloned();
            let Some(child) = child else { break };
            after = child.id.get();
            child.visit(run);
        }
    }
    pub fn all_reaped(&self) -> bool {
        let mut done = true;
        self.visit(&mut |node| {
            if node.tasks.lock().iter().any(|m| m.exit().is_none()) {
                done = false;
            }
        });
        done
    }
    pub fn next(&self, owner: TaskId) -> Option<Arc<Member>> {
        let mut found = None;
        self.visit(&mut |node| {
            if found.is_none() {
                found = node
                    .tasks
                    .lock()
                    .iter()
                    .find(|m| m.pending(owner) && m.exit().is_some())
                    .cloned();
            }
        });
        found
    }
    pub fn find(&self, id: TaskId) -> Option<Arc<Member>> {
        let mut found = None;
        self.visit(&mut |node| {
            if found.is_none() {
                found = node.member(id);
            }
        });
        found
    }
    pub fn clear(&self, owner: TaskId, root: Option<TeamId>) {
        self.visit(&mut |node| {
            for member in node.tasks.lock().iter() {
                let mut receipts = member.receipts.lock();
                for receipt in receipts.iter_mut().filter(|r| r.owner == owner) {
                    if let Some(root) = root {
                        receipt.roots.retain(|id| *id != root);
                    } else {
                        receipt.roots.clear();
                    }
                }
                receipts.retain(|r| !r.roots.is_empty());
            }
        });
    }
    pub fn prune(&self) {
        self.tasks
            .lock()
            .retain(|m| m.exit().is_none() || !m.receipts.lock().is_empty());
        let mut at = 0;
        loop {
            let child = self.children.lock().get(at).cloned();
            let Some(child) = child else { break };
            child.prune();
            let terminal = child.state.lock().phase.ousted();
            let empty = child.tasks.lock().is_empty() && child.children.lock().is_empty();
            if terminal && empty {
                self.children.lock().remove(at);
            } else {
                at += 1;
            }
        }
    }
    pub fn doom(&self) {
        let _commit = crate::core::work::commit();
        self.visit(&mut |node| {
            let mut state = node.state.lock();
            if !state.terminal() {
                state.phase = TeamPhase::Doomed;
            }
        });
    }
    pub fn notify(&self) {
        let mut node = Some(self);
        while let Some(here) = node {
            crate::core::work::room::messenger::signal(crate::core::work::room::messenger::WakeKey::Team {
                id: here.id,
            });
            node = here.parent.as_deref();
        }
    }
}
