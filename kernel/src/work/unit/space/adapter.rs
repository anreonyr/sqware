// adapter — 适配层：[`Space`] 门 + [`SpaceBuilder`] + [`Space`] 的公开入口面。
//!
//! # 层职责（核心/适配分离的另一半）
//!
//! - [`SpaceInner`](super::core::SpaceInner) = 核心：数据 + 全部操作体，**无锁无刷**。
//! - 本文件 = 适配：`RelLock` 门（`with` / `with_flush`）+ 每操作 ≤3 行转发；
//!   锁外按本空间 ASID 刷 TLB。
//!
//! **适配层不认识领域**：`Space` 不知道栈/帧/堆/mmap 是什么——那些是窗口层
//! （`space::window::*`）的策略，本文件只提供「取锁 → 转发 → 刷」的骨架。
//!
//! # 锁约定
//!
//! 全部可变状态由 `RelLock` 互斥（跨 hart 真自旋，同 hart 可重入）；`kind`
//! 分配后不可变，故 `Space` 可放心跨线程传递。窗口事务统一经 `with` /
//! `with_flush`（锁恰好一次，闭包内直接操作 `inner`）；其余公开方法各自锁恰好
//! 一次、不重入。**`with` 闭包内不得再调用 `Space` 的任何方法。**
//!
//! # Drop
//!
//! `root`（页表树，递归归还全部表帧）、`maps` 帧随字段自动 drop 归还 frame 池
//! ——所有权驱动。非内核空间先 `fence::retire(asid)` 销账再归还 ASID（顺序契约：
//! ASID 复用后键即换主）。

use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;

use super::SpaceKind;
use super::core::SpaceInner;
use super::map::{Pending, PendingState};
use super::salvage::{Salvage, Span};
use super::segment::SegmentKind;
use crate::layout::TRAMPOLINE;
use crate::lock::{Level, RelLock};
use crate::memory::PAGE_SIZE;
use crate::memory::manager::MapError;
use crate::memory::manager::addr::{PhysAddr, VirtAddr};
use crate::memory::manager::asid::{self, Asid, Deaf};
use crate::memory::manager::entry::PteFlags;
use crate::memory::manager::flush_asid;
use crate::memory::manager::table::Frame;
use crate::work::unit::life::Life;

// SAFETY: 全部可变状态由 `RelLock` 互斥；页表树读写与 `SpaceInner` 共享同一把锁。
unsafe impl Send for Space {}
unsafe impl Sync for Space {}

/// 虚拟地址空间（布局随运行模式）。
///
/// 拥有根页表与**全部自有物理帧**。全部可变状态（`SpaceInner`：root / user /
/// kernel / maps）收进一把 [`RelLock`]。
///
/// # Concurrency
///
/// 全部可变状态收进一把 [`RelLock`]（跨 hart 自旋、同 hart 可重入）。窗口事务
/// 经 [`with`](Self::with) / [`with_flush`](Self::with_flush) 锁恰好一次。**段表
/// 并入本锁**：`Segment` 无锁，所有 `allocate`/`deallocate` 必须在事务内。
///
/// # Drop
///
/// `root`（页表树，递归归还全部表帧）、`maps` 帧随字段自动 drop 归还 frame 池
/// ——所有权驱动。
#[derive(Debug)]
pub struct Space {
    /// 全部可变状态（root / 两段 / maps）——一把可重入锁保护。
    inner: RelLock<SpaceInner>,
    /// 页表被哪个特权级使用（S 态 / U 态）。
    kind: SpaceKind,
    /// 空间身份（TLB 标记 + wait/fence 键命名空间）；0 = 内核空间。
    asid: Asid,
    /// 本空间的**存活单元**（`WakeKey::Space{space: asid}` 的寿命来源）：
    /// 强持有者是本 `Space` ⇒ 空间回收时键自然判死，站点随之可删（A2）。
    life: Arc<Life>,
}

/// 连续 VA 区间逐段翻译迭代器（`Space::segments` 产出）。
///
/// 每步：经 `Space::translate` 译当前 VA 页 → 产出 `(PA, flags, 页内余量)`。
/// 物理帧可能不连续，故按页切段；未映射页 → 迭代终止。惰性零分配。
pub struct Segments<'a> {
    space: &'a Space,
    va: usize,
    end: usize,
}

impl Iterator for Segments<'_> {
    type Item = (crate::memory::manager::addr::PhysAddr, PteFlags, usize);

    fn next(&mut self) -> Option<Self::Item> {
        if self.va >= self.end {
            return None;
        }
        let (pa, flags) = self.space.translate(VirtAddr::from_raw(self.va))?;
        let page = self.va & !(PAGE_SIZE - 1);
        let chunk = (page + PAGE_SIZE - self.va).min(self.end - self.va);
        self.va += chunk;
        Some((pa, flags, chunk))
    }
}

/// [`Space`] 构造器。
pub struct SpaceBuilder {
    kind: SpaceKind,
    asid: Asid,
}

impl SpaceBuilder {
    /// 内核空间构造器：S 态页表 + 固定身份 ASID 0（全局唯一，永不回收）。
    pub fn kernel() -> Self {
        Self {
            kind: SpaceKind::Supervisor,
            asid: Asid::kernel(),
        }
    }

    /// supervisor 域空间构造器：S 态页表 + 独立 ASID。
    pub fn supervisor() -> Self {
        Self {
            kind: SpaceKind::Supervisor,
            asid: Asid::allocate(),
        }
    }

    /// 用户空间构造器：U 态页表 + 独立 ASID。
    pub fn user() -> Self {
        Self {
            kind: SpaceKind::User,
            asid: Asid::allocate(),
        }
    }

    /// 完成构建：分配根页表帧；非内核空间额外种入 trampoline 叶 PTE——任何会
    /// 承接陷阱的空间都必须能取指到 trampoline 页。
    pub fn build(self) -> Result<Space, MapError> {
        let mut space = Space {
            kind: self.kind,
            asid: self.asid,
            life: Life::new(),
            inner: RelLock::new_level(Level::Space, SpaceInner::durable()?),
        };
        if !self.asid.is_kernel() {
            self.seed_trampoline(&mut space)?;
        }
        Ok(space)
    }

    /// 从内核地址空间出非内核空间（`build()` 内部调用）。
    ///
    /// 不复制内核半区映射——非内核页表只含自己的映射 + trampoline 叶 PTE 复制
    /// （帧归内核，借用映射：`pending: None` + 空帧）。
    fn seed_trampoline(&self, space: &mut Space) -> Result<(), MapError> {
        let (tramp_pa, tramp_flags) = {
            let ks_inner = crate::work::unit::team::kernel()
                .expect("kernel team not initialized")
                .space
                .inner
                .lock();
            ks_inner.root.walk_ref(TRAMPOLINE)?
        };
        space.borrow(TRAMPOLINE, tramp_pa, PAGE_SIZE, tramp_flags)?;
        Ok(())
    }
}

impl Space {
    // ── 身份 ────────────────────────────────────────────────

    /// 空间种类（内核 / 用户）——任务模式判定的单一事实源。
    pub fn kind(&self) -> SpaceKind {
        self.kind
    }

    /// 页权限的**单一出口**：U 位只由空间种类决定，任何产页权限的地方都经此。
    ///
    /// - `Supervisor`（含内核空间）→ 清 U：S 态 `SUM=0`，带 U 的页自己访问就缺页。
    /// - `User` → 置 U：U 态访问必须带 U。
    ///
    /// 只作用于**本空间自有**的页（装载段 / 栈堆共享窗口 / mmap / mprotect /
    /// Pole 借映）。两类**内核自有**的页不走此函数：
    /// - `borrow_map` 借用的内核映射（trampoline / DRAM 恒等 / dock 视图）；
    /// - `FrameWindow`（trap 帧）——落在任务空间里也恒 U=0：trap 入口在 S 态
    ///   （`SUM=0`）把寄存器现场写进它，带 U 会缺页。
    ///
    /// `map` / `attach_map` / `protect` 内部已兜底；窗口与 loader 在 `with_flush`
    /// 里直调 `inner`，须自行调用本函数。
    pub fn pte_policy(&self, flags: PteFlags) -> PteFlags {
        if self.kind().is_supervisor() {
            flags - PteFlags::U
        } else {
            flags | PteFlags::U
        }
    }

    /// 空间身份（写入 `satp.ASID` / 组 wait·fence 键；0 = 内核空间）。
    pub fn asid(&self) -> Asid {
        self.asid
    }

    /// 本空间的存活单元（弱引用）——`WakeKey::Space{space: asid}` 的寿命来源。
    ///
    /// 调用方（envcall 的 `Wait` / `Wake`）随键一起把它交给等待机：站点值只留这枚
    /// 弱引用，判死全在 room 侧靠 `upgrade` 观察。见 [`Life`]。
    pub fn life(&self) -> Weak<Life> {
        Arc::downgrade(&self.life)
    }

    /// 返回根页表页号（写入 `satp` 用）。
    pub fn root(&self) -> usize {
        self.with(|inner| inner.root.ppn())
    }

    // ── 事务入口 ────────────────────────────────────────────

    /// 簿记事务：锁恰好一次，闭包内直接操作 [`SpaceInner`]；锁外不刷 TLB。
    ///
    /// 纪律：闭包内**不得再调用 `Space` 的任何方法**（重入双 guard，UB）。
    /// **段表并入本锁**——`allocate`/`deallocate` 只能在此闭包内发生。
    pub(crate) fn with<R>(&self, op: impl FnOnce(&mut SpaceInner) -> R) -> R {
        let mut inner = self.inner.lock();
        let r = op(&mut inner);
        drop(inner);
        r
    }

    /// 写页表事务（新增 / 放宽 / 换帧）：同 [`with`](Self::with)，锁外按本空间
    /// ASID 刷 TLB。**无远核义务**——远核最坏持陈旧无效条目或旧窄权限条目，
    /// 会吃一次伪缺页，由 `fault` 的 re-walk 判 resolved + trap 两侧整表刷自愈。
    pub(crate) fn with_flush<R>(&self, op: impl FnOnce(&mut SpaceInner) -> R) -> R {
        let r = self.with(op);
        // SAFETY: sfence.vma 见 flush_asid
        unsafe {
            flush_asid(self.asid.get());
        }
        r
    }

    /// 写页表事务（**收紧**：降权 / 只读化）：同 [`with`](Self::with)，锁外
    /// **就地跨核清退**——远核仍持旧宽权限条目 = 写不缺页 = 丢失更新。
    ///
    /// # Errors
    ///
    /// [`Deaf`] = RFENCE 清退失败（致命级，适配层裁定策略）。
    pub(crate) fn with_shootdown<R>(
        &self,
        op: impl FnOnce(&mut SpaceInner) -> R,
    ) -> Result<R, Deaf> {
        let r = self.with(op);
        asid::shootdown(self.asid)?;
        Ok(r)
    }

    // ── 适配层（转发 inner 原语 + 刷）───────────────────────

    /// 只登记簿记（懒/守卫/借用占位），不装 PTE，不需刷 TLB。
    pub(crate) fn map(
        &self,
        va: VirtAddr,
        size: usize,
        flags: PteFlags,
        pending: Option<Pending>,
    ) -> Result<(), MapError> {
        let flags = self.pte_policy(flags);
        self.with(|inner| inner.map(va, size, flags, pending))
    }

    /// 借帧连续映射（DRAM 恒等 / trampoline / dock·ring 视图）。
    pub fn borrow(
        &self,
        vaddr: VirtAddr,
        paddr: PhysAddr,
        size: usize,
        flags: PteFlags,
    ) -> Result<(), MapError> {
        self.with_flush(|inner| inner.borrow(vaddr, paddr, size, flags))
    }

    /// 已备帧装配（loader 逐段 / hart 帧 / health 压测）。
    pub(crate) fn attach(
        &self,
        vaddr: VirtAddr,
        frames: Vec<Frame>,
        flags: PteFlags,
    ) -> Result<(), MapError> {
        let flags = self.pte_policy(flags);
        self.with_flush(|inner| inner.attach(vaddr, frames, flags))
    }

    /// 统一拆除（munmap 后端 / guard 打洞）：清叶 + 摘/裂 Map + 刷本核 + 结清
    /// （清退到齐后帧才归还）。不还段——段由 [`Self::release`] 收。
    pub fn unmap(&self, vaddr: VirtAddr, size: usize) {
        let mut salvage = Salvage::new();
        self.with_flush(|inner| inner.unmap(vaddr, size, &mut salvage));
        salvage.reclaim(self).expect("unmap: shootdown deaf");
    }

    /// Span 回收门：校验段 + 统一拆除 + 刷本核 + 结清（清退到齐后**才**还段与帧
    /// ——还段即 VA 可复用，远核旧条目会污染新映射）。
    ///
    /// `Span` 由 claim/allocate/mmap 产出（`release` 只收它——分配与回收同一
    /// 类型，杜绝 re-find）。失败域：`MapError::SegmentMismatch` = Span 与段状态
    /// 不一致（调用方 bug——绝大多数调用方用 `.expect()` 保留 panic 语义）。
    ///
    /// # 这是**唯一**的释放实现
    ///
    /// 内核里"释放一段"只有这一处拆装代码。此前 `HeapWindow::deallocate` 与
    /// `ShareWindow::munmap` **各自复制了一遍**（`holds` 校验 + `unmap` +
    /// `take_span` + `reclaim`，逐字同构），于是同一件事有三份实现、三套失败语义
    /// ——而那正是"一份资源有两个归还者"的温床（本仓实测过：`Munmap` 能拆到用户
    /// 堆页却**不注销账目**，因为 `ShareWindow::munmap` 与堆共用 `Seg::User`，
    /// 见 `docs/allocator-diagnosis.md` §8）。
    ///
    /// 现在两个 window 都经 [`Self::release_addr`] 收敛到这里：**一个入口、
    /// 一个失败域、一个注销点**。
    pub(crate) fn release(&self, span: Span) -> Result<(), MapError> {
        let mut salvage = Salvage::new();
        self.with_flush(|inner| {
            // 1. 只读校验（失败即 SegmentMismatch，状态未动）
            if !inner.holds(span.seg, span.va.as_usize(), span.size.get()) {
                return Err(MapError::SegmentMismatch);
            }
            // 2. 统一拆除（清叶 / 摘·裂 map，帧交料箱）+ 段一并入箱
            inner.unmap(span.va, span.size.get(), &mut salvage);
            salvage.take_span(span);
            Ok(())
        })?;
        // 3. **这里不销账**（曾经销过，是错的——本条是那次错误的墓志铭）。
        //
        // 用户堆页的账目键是 `(asid, 页索引)`，而账目的**主人**是
        // `MemoryCall::Deallocate` 的臂（`envcall.rs` 的 `on_free`），它按
        // `(key, size, Kind::UserHeap)` 精确注销；空间作废时则由 `Space::drop` 的
        // `fence::retire(asid)` 整片收走。本条释放门夹在两者**之间**——走到这里时
        // 那笔账**还是活的、且应当保持活着**，因为 `on_free` 正要读它来核
        // `SizeMismatch`、canary 与种类。
        //
        // 上一轮我在这个位置加过 `retire_range`，理由是"任何拆掉一块用户区间的
        // 路径都得销账"（当时怀疑 `Munmap` → `ShareWindow::munmap` 不销账）。那个
        // 怀疑**不成立**：`Mmap`/`Munmap` 在用户态没有调用方，而唯一真在跑的路径
        // 是堆的 `Deallocate`——于是这个"兜底"恰好打在**唯一真路径**上：先把记录
        // 销掉，`on_free` 随后 `unmark` 便查无此账，`report(UnregisteredFree)`
        // 是 `-> !`，**当场 panic 掉整机**。实测（`--features audit`）：
        //
        // ```text
        // [audit] release retires 1 user-heap records @ 0x2d000+0x1000
        // [integrity] UnregisteredFree at 0x60000000002d: unmark: no record
        // ```
        //
        // 教训是这条：**"一个资源两个归还者"的解法不是再加一个归还者**，而是把
        // 归还点收敛掉。此处已收敛（两个 window 都走本条），销账点则本来就只有一个。
        salvage.reclaim(self).expect("release: shootdown deaf");
        Ok(())
    }

    /// **地址形态**的释放门：把用户面送来的 `(addr, size)` 还原成 `Span` 后走
    /// [`Self::release`]。
    ///
    /// # 为什么需要这一层
    ///
    /// 栈与 trap 帧的 Span 由内核自己持有（存进 `TaskIdent`），退场时直接
    /// `release(span)`。但**堆与 mmap 是用户面**：`EnvCall::Memory::Deallocate`
    /// 与 `Munmap` 只送来地址与长度，内核手上没有那块区域的身份，故必须由地址
    /// 还原。
    ///
    /// 前置校验放在这里（而不是在两个 window 里各写一遍）：地址不对是**用户输入
    /// 问题**，不是调用方 bug，所以回 `false`（"未登记"语义），而 `release` 的
    /// `SegmentMismatch` 留给内核内部调用方（它们用 `.expect()`）。
    ///
    /// 返回：找到并释放 → `true`；该区间不是本段的已分配块 → `false`（状态未动）。
    pub(crate) fn release_addr(&self, seg: SegmentKind, addr: VirtAddr, size: usize) -> bool {
        if !self.with_flush(|inner| inner.holds(seg, addr.as_usize(), size)) {
            return false;
        }
        self.release(Span::new(seg, addr, size, None)).is_ok()
    }

    /// 懒页物化（缺页处理：分配零页装叶注入 + 刷 TLB）。
    pub fn materialize(&self, vaddr: VirtAddr, size: usize) -> Result<(), MapError> {
        self.with_flush(|inner| inner.materialize(vaddr, size))
    }

    /// 修改保护标志（mprotect 后端）：收紧类，就地跨核清退。
    pub fn protect(&self, vaddr: VirtAddr, size: usize, flags: PteFlags) -> Result<(), MapError> {
        let flags = self.pte_policy(flags);
        self.with_shootdown(|inner| inner.protect(vaddr, size, flags))
            .expect("protect: shootdown deaf")
    }

    // ── 查询 ────────────────────────────────────────────────

    /// 将虚拟地址翻译为物理地址和标志位（页表读路径）。
    pub fn translate(&self, vaddr: VirtAddr) -> Option<(PhysAddr, PteFlags)> {
        self.with(|inner| inner.translate(vaddr))
    }

    /// 查询 `vaddr` 所属映射的物化态（缺页分派用）。
    pub fn pending_state(&self, vaddr: VirtAddr) -> PendingState {
        self.with(|inner| match inner.resolve_ref(vaddr) {
            Some(m) => match m.pending {
                None => PendingState::Materialized,
                Some(Pending::Lazy) => PendingState::Lazy,
                Some(Pending::Guard) => PendingState::Guard,
            },
            None => PendingState::Absent,
        })
    }

    /// 连续 VA 区间 → 逐段物理翻译（[`Segments`] 迭代器）。
    ///
    /// 逐页步进（物理帧可能不连续），产出 `(物理地址, 标志, 段字节数)`；
    /// 某页未映射即停。
    pub fn segments(&self, va: VirtAddr, len: usize) -> Segments<'_> {
        Segments {
            space: self,
            va: va.as_usize(),
            end: va.as_usize() + len,
        }
    }

    /// 页表树节点总数（自测用，非审计链）。
    ///
    /// 门跟着**用户**走：`debug_assertions`（旧 `health` 验收跑在 debug）与
    /// `framework`（测试档用例要它；framework profile 恒开 debug-assertions，但
    /// `--features framework` 也能落在 release 上 —— 那时不带此门就编不过）。
    #[cfg(any(debug_assertions, feature = "framework"))]
    pub fn table_count(&self) -> usize {
        self.with(|inner| inner.root.count())
    }

    /// 簿记↔页表一致性审计（boot / 压力测试后调用；不一致即 panic）。
    ///
    /// 同 [`Self::table_count`]：门跟着用户走 —— 用例里的 `space.audit()` 是它的第二个
    /// 调用点，而 `--features framework` 未必同时带 `audit`。
    #[cfg(any(feature = "audit", feature = "framework"))]
    pub(crate) fn audit(&self) {
        self.with(|inner| inner.audit());
    }
}

impl Drop for Space {
    fn drop(&mut self) {
        if !self.asid.is_kernel() {
            // 1. 注销本空间名下的用户堆账：账的所有者是空间（键含 asid），任务
            //    退出不 unwind、退出时仍持堆是常态（TLS 由构造决定永不释放）
            //    ——账只能随空间作废。必须先于 ASID 归还：复用后键即换主。
            crate::memory::allocator::fence::retire(self.asid.get());
            // 2. 释放 ASID（内含清退：ASID 立即可被复用，残留条目会让新空间同
            //    VA 命中旧映射）。此路径恒走快路径——Arc 归零 ⇒ 无任务持有本
            //    空间 ⇒ 没有任何核驻留该 ASID。
            asid::deallocate(self.asid).expect("space drop: shootdown deaf");
        }
        // `inner` 随字段自动 drop：root（页表树）/maps 帧全部归还 frame 池。
    }
}
