//! 目标 Space 的**视图控制记录**：一段已发布关系归谁控制。
//!
//! 它是**控制授权的事实**，不是 CPU 页槽的副本：真实关系与页表仍在 `Nest` 手里。
//! 每项绑定真实 `View` 所在的那个 `Nest`、它的精确身份、这次 `Attach` 选的控制范围
//! 与它的来源授权。同 Space 的另一个 Task 猜中号也控不动它。

use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::core::memory::nest::View;
use crate::core::work::pend::SourceAuthorization;
use crate::machine::sync::SpinLock;
use env::{PieFail, PieToken, TaskId, ViewControl};

use super::{Space, ViewEntry};

pub(crate) struct ViewRegistry {
    entries: SpinLock<Vec<(usize, ViewEntry)>>,
}

impl ViewRegistry {
    pub(crate) fn new() -> Self {
        Self {
            entries: SpinLock::new(Vec::new()),
        }
    }

    /// 发布前先占住这个号的位置：容量与槽都在同一把锁下备齐，
    /// 之后的提交只替换已备位置，不再分配。
    pub(crate) fn claim(
        &self,
        id: usize,
        space: &Arc<Space>,
        controller: TaskId,
        scope: ViewControl,
        source: Arc<SourceAuthorization>,
        branch: Option<PieToken>,
    ) -> Result<(), PieFail> {
        let mut entries = self.entries.lock();
        if entries.iter().any(|(held, _)| *held == id) {
            return Err(PieFail::Busy);
        }
        entries.try_reserve(1).map_err(|_| PieFail::OoM)?;
        entries.push((
            id,
            ViewEntry {
                space: Arc::downgrade(space),
                view: None,
                scope,
                controller,
                source,
                branch,
            },
        ));
        Ok(())
    }

    /// 提交：只把已备槽填上真实关系，不分配。
    pub(crate) fn commit(&self, id: usize, view: Arc<View>) {
        let mut entries = self.entries.lock();
        if let Some((_, entry)) = entries.iter_mut().find(|(held, _)| *held == id) {
            entry.view = Some(view);
        }
    }

    /// 放弃这次发布：拿掉那个在途槽。
    pub(crate) fn cancel(&self, id: usize) {
        self.entries.lock().retain(|(held, _)| *held != id);
    }

    /// 按号取回控制登记；在途槽不算可用，号本身不授予任何东西。
    pub(crate) fn locate(&self, id: usize) -> Option<ViewEntry> {
        self.entries
            .lock()
            .iter()
            .find(|(held, entry)| *held == id && entry.view.is_some())
            .map(|(_, entry)| entry.clone())
    }

    /// 最后一次安全清理完成后回收这一段关系的号。
    pub(crate) fn retire(&self, id: usize) {
        self.entries.lock().retain(|(held, _)| *held != id);
    }

    pub(crate) fn take_settled(
        &self,
        nest: &crate::core::memory::nest::Nest,
    ) -> Option<(usize, ViewEntry)> {
        let mut entries = self.entries.lock();
        let at = entries
            .iter()
            .position(|(id, entry)| entry.view.is_some() && !nest.contains_reservation(*id))?;
        Some(entries.remove(at))
    }

    pub(crate) fn hand_over(&self, id: usize, controller: TaskId) {
        let mut entries = self.entries.lock();
        let (_, entry) = entries
            .iter_mut()
            .find(|(held, entry)| *held == id && entry.view.is_some())
            .expect("prepared staged view");
        entry.controller = controller;
        entry.branch = None;
    }

    /// 一份来源名下已登记的项（撤销时取出，在锁外关）。
    pub(crate) fn owned_by(
        &self,
        source: &Arc<SourceAuthorization>,
    ) -> Result<Vec<(usize, ViewEntry)>, PieFail> {
        let entries = self.entries.lock();
        let mut result = Vec::new();
        result
            .try_reserve(entries.len())
            .map_err(|_| PieFail::OoM)?;
        result.extend(
            entries
                .iter()
                .filter(|(_, entry)| entry.view.is_some() && Arc::ptr_eq(&entry.source, source))
                .cloned(),
        );
        Ok(result)
    }
}

unsafe impl Send for ViewRegistry {}
unsafe impl Sync for ViewRegistry {}
