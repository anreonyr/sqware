//! 目标空间的 CPU 关系登记；仅显式控制记录授予 Memory 视图操作。
//! 来源与登记项共享帧无关绑定，真实页表与失效责任仍由 Nest 承担。

use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::core::memory::nest::View;
use crate::core::work::pend::SourceAuthorization;
use crate::machine::sync::SpinLock;
use env::{PieFail, PieToken, TaskId, ViewControl};

use super::{Space, ViewBinding, ViewControlRecord, ViewEntry};

pub(crate) struct ViewRegistry {
    entries: SpinLock<Vec<ViewEntry>>,
}

impl ViewRegistry {
    pub(crate) fn new() -> Self {
        Self {
            entries: SpinLock::new(Vec::new()),
        }
    }

    /// 占住这个号，并**同时**把这段关系登到来源名下：两表在同一个提交段里写。
    ///
    /// 可失败的准备（造绑定、查重号、两表各自备容量）全部先做完，之后两次短锁
    /// 只做写入——写进去就不再失败，不留只有一边的半条登记。
    pub(crate) fn claim(
        &self,
        id: usize,
        space: &Arc<Space>,
        controller: TaskId,
        scope: ViewControl,
        source: Arc<SourceAuthorization>,
        branch: Option<PieToken>,
    ) -> Result<(), PieFail> {
        let control = ViewControlRecord::new(controller, scope, branch)?;
        self.claim_entry(id, space, source, Some(control))
    }

    pub(crate) fn claim_mapping(
        &self,
        id: usize,
        space: &Arc<Space>,
        source: Arc<SourceAuthorization>,
    ) -> Result<(), PieFail> {
        self.claim_entry(id, space, source, None)
    }

    fn claim_entry(
        &self,
        id: usize,
        space: &Arc<Space>,
        source: Arc<SourceAuthorization>,
        control: Option<Arc<ViewControlRecord>>,
    ) -> Result<(), PieFail> {
        let _commit = crate::core::work::commit();
        let binding = ViewBinding::new(id, space)?;
        {
            let mut entries = self.entries.lock();
            if entries.iter().any(|entry| entry.binding.id() == id) {
                return Err(PieFail::Busy);
            }
            entries.try_reserve(1).map_err(|_| PieFail::OoM)?;
        }
        source.reserve_binding()?;
        source.register_binding(binding.clone());
        self.entries.lock().push(ViewEntry {
            binding,
            control,
            view: None,
            source,
        });
        Ok(())
    }

    /// 提交：只把已备槽填上真实关系，不分配。
    pub(crate) fn commit(&self, id: usize, view: Arc<View>) {
        let mut entries = self.entries.lock();
        let entry = entries
            .iter_mut()
            .find(|entry| entry.binding.id() == id)
            .expect("prepared view registration");
        entry.view = Some(view);
    }

    /// 按号取回任意已登记关系；不以登记的存在代替控制授权。
    ///
    /// 不要拆成 `locate` + `pending` 两次查询：中间一旦提交，两次都会落空。
    pub(crate) fn registered(&self, id: usize) -> Option<ViewEntry> {
        self.entries
            .lock()
            .iter()
            .find(|entry| entry.binding.id() == id)
            .cloned()
    }

    /// 按号取回可用控制；在途槽不算可用，号本身不授予任何东西。
    pub(crate) fn locate(&self, id: usize) -> Option<ViewEntry> {
        self.registered(id)
            .filter(|entry| entry.view.is_some() && entry.control.is_some())
    }

    /// 还没提交的发布登记：可能仍在发布，也可能把清理责任留在 Nest 等结清。
    ///
    /// 与 `locate` 分开：等在途槽不是可用控制，但它同样持有来源与 controller。
    #[cfg(debug_assertions)]
    pub(crate) fn pending(&self, id: usize) -> Option<ViewEntry> {
        self.registered(id).filter(|entry| entry.view.is_none())
    }

    /// 取回这一段关系的登记（连同它持有的关系与来源），由调用方在锁外结清。
    pub(crate) fn retire(&self, id: usize) -> Option<ViewEntry> {
        let mut entries = self.entries.lock();
        let at = entries.iter().position(|entry| entry.binding.id() == id)?;
        Some(entries.remove(at))
    }

    /// 取出下一笔 CPU 关系已结清的登记；占用身份与全空间关闭由 Nest 判定。
    ///
    /// 在途槽同样适用：发布失败的清理链走完后，它也要退表并释放 controller。
    pub(crate) fn take_settled(&self, nest: &crate::core::memory::nest::Nest) -> Option<ViewEntry> {
        let mut entries = self.entries.lock();
        let at = entries
            .iter()
            .position(|entry| nest.settled(entry.binding.id()))?;
        Some(entries.remove(at))
    }

    /// 转交显式控制记录，CPU 关系身份与来源成员不变。
    /// 两把短锁不重叠，保持发布段的持锁预算。
    pub(crate) fn hand_over(&self, id: usize, controller: TaskId) {
        let control = {
            let entries = self.entries.lock();
            entries
                .iter()
                .find(|entry| entry.binding.id() == id && entry.view.is_some())
                .expect("prepared staged view")
                .control
                .as_ref()
                .expect("staged view control")
                .clone()
        };
        control.hand_over(controller);
    }

    /// 分支改指：只改这一份来源自己那些仍带着被忘掉那枚 token 的记录。
    ///
    /// 授权快照与控制身份都不动——这里只改历史分支索引。
    pub(crate) fn retarget_branch(
        &self,
        source: &Arc<SourceAuthorization>,
        forgotten: PieToken,
        parent: PieToken,
    ) {
        let entries = self.entries.lock();
        for entry in entries
            .iter()
            .filter(|entry| Arc::ptr_eq(&entry.source, source))
        {
            if let Some(control) = &entry.control {
                control.retarget(forgotten, parent);
            }
        }
    }

    /// 一份来源名下已登记的项（撤销时取出，在锁外关）。
    pub(crate) fn owned_by(
        &self,
        source: &Arc<SourceAuthorization>,
    ) -> Result<Vec<ViewEntry>, PieFail> {
        let entries = self.entries.lock();
        let mut result = Vec::new();
        result
            .try_reserve(entries.len())
            .map_err(|_| PieFail::OoM)?;
        result.extend(
            entries
                .iter()
                .filter(|entry| entry.view.is_some() && Arc::ptr_eq(&entry.source, source))
                .cloned(),
        );
        Ok(result)
    }
}

unsafe impl Send for ViewRegistry {}
unsafe impl Sync for ViewRegistry {}
