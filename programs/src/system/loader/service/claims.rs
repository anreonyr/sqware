use alloc::vec::Vec;
use env::{Access, Mark, Permission, PieKind, PieToken, TaskId};
use system_api::loader::Grant;

const ROOT_MARK: Mark = Mark::of("loader/claim-root");
fn root_access() -> Permission {
    Permission::FETCH | Permission::STORE | Permission::VEST
}

#[derive(Clone, Copy)]
struct Receipt {
    task: TaskId,
    root: PieToken,
    seed: PieToken,
}

#[derive(Default)]
pub(crate) struct Claims {
    receipts: Vec<Receipt>,
}

impl Claims {
    pub(crate) fn grant(&mut self, source: PieToken, task: TaskId) -> Result<(), &'static str> {
        if self
            .receipts
            .iter()
            .any(|receipt| receipt.task == task && receipt.seed != PieToken::NONE)
        {
            return Ok(());
        }
        if task.get() == 0 || task == env::unit::self_id() {
            return Err("Loader Claim target");
        }
        self.receipts
            .try_reserve(1)
            .map_err(|_| "Loader Claim receipt capacity")?;
        let source_info = resource::raw::inspect(source).map_err(|_| "Loader Claim source")?;
        let owner = env::unit::self_id();
        let access = root_access();
        if !source_info.alive
            || source_info.owner != owner
            || (source_info.vestor.get() != 0 && source_info.vestor != owner)
            || source_info.kind != PieKind::Hole
            || source_info.mark != Grant::Claim.mark()
            || !source_info.permission.contains(access)
        {
            return Err("Loader Claim source provenance");
        }
        let root = if let Some(receipt) = self.receipts.iter().find(|receipt| receipt.task == task)
        {
            receipt.root
        } else {
            let root = env::pie::accord(source, owner, access, ROOT_MARK)
                .map_err(|_| "Loader Claim private root")?;
            self.receipts.push(Receipt {
                task,
                root,
                seed: PieToken::NONE,
            });
            let info = resource::raw::inspect(root).map_err(|_| "Loader Claim private root")?;
            if !info.alive
                || info.owner != owner
                || info.vestor != owner
                || info.kind != PieKind::Hole
                || info.mark != ROOT_MARK
                || info.permission != access
            {
                if env::pie::release(root, env::ReleaseMode::Revoke).is_ok() {
                    self.receipts.retain(|receipt| receipt.task != task);
                }
                return Err("Loader Claim private root provenance");
            }
            root
        };
        let index = self
            .receipts
            .iter()
            .position(|receipt| receipt.task == task)
            .ok_or("Loader Claim receipt missing")?;
        if self.receipts[index].seed != PieToken::NONE {
            return Ok(());
        }
        let root_info = resource::raw::inspect(root).map_err(|_| "Loader Claim private root")?;
        if !root_info.alive
            || root_info.owner != owner
            || root_info.vestor != owner
            || root_info.kind != PieKind::Hole
            || root_info.mark != ROOT_MARK
            || root_info.permission != access
        {
            return Err("Loader Claim private root provenance");
        }
        let seed = env::pie::accord(root, task, Access::STORE.bits(), Grant::Claim.mark())
            .map_err(|_| "Loader Claim delivery")?;
        self.receipts[index].seed = seed;
        Ok(())
    }

    pub(crate) fn retire(&mut self, task: TaskId) -> Result<(), &'static str> {
        let Some(index) = self
            .receipts
            .iter()
            .position(|receipt| receipt.task == task)
        else {
            return Ok(());
        };
        env::pie::release(self.receipts[index].root, env::ReleaseMode::Revoke)
            .map_err(|_| "Loader Claim revoke")?;
        self.receipts.swap_remove(index);
        Ok(())
    }

    pub(crate) fn retire_all(&mut self) -> Result<(), &'static str> {
        while let Some(task) = self.receipts.first().map(|receipt| receipt.task) {
            self.retire(task)?;
        }
        Ok(())
    }
}
