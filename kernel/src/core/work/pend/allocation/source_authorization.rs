//! 一份**来源授权**：某个来源下所有普通引用与已建关系共享的唯一记录。
//!
//! 它住在工作层（与引用表、创建资格、事务同处），由执行入口与 Pole 创建共同使用；
//! 核心 `View` 不回指它。私有根 → 根 Grant 的交接只改这里的 `phase` 一处权威，
//! 不为每份引用复制一份状态再扫表更新。资格只属于这份记录，
//! 普通引用的有无靠显式语义判断，不数 `Arc` 强引用。
//!
//! 名字带 `Authorization` 是为了与 Flat 的“来源”（`Flat` / `FlatRange`）分开：
//! 这一份说的是**授权关系**，不是物理来源。

use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::core::memory::address::{PageCount, PageIndex, PageOffset};
use crate::core::memory::flat::FlatRange;
use crate::core::memory::nest::Access;
use crate::core::work::grant::Grant;
use crate::core::work::mail::pole::PoleMeta;
use crate::core::work::unit::space::{Space, ViewBinding};
use crate::machine::sync::SpinLock;
use env::{Permission, PieFail, PieToken, TaskId};

use super::creation::Creation;
use super::derivation::Derivation;
use super::geometry::Geometry;
use super::lineage::Lineage;
use super::source_phase::Phase;

pub(crate) struct SourceAuthorization {
    /// 私有阶段由它自己强持整段来源；交给根 Grant 之后这里变空，
    /// 真实范围改由 Pole 持有——普通引用因此不会在撤销后继续 pin 住帧。
    range: SpinLock<Option<FlatRange>>,
    ceiling: Access,
    owner: TaskId,
    qualification: Creation,
    /// 还活着的普通引用份数（成员资格，不是 `Arc` 强计数）。
    holds: AtomicUsize,
    /// **在途发布**的键：闸门下登记，撤销方看到它就不结清。
    ///
    /// 键来自两个分配器（关系号那一域与移交那一域），各自只保证**域内**唯一；
    /// 带窗口的那一档（移交）还把“这一段已经核过、别的发表不得插进来”一并记下。
    /// 两张域靠键自带的那一位分开，故同号不会互删。
    inflight: SpinLock<Vec<(usize, Option<Arc<Geometry>>)>>,
    /// 发布与撤销的共同闸门。
    revoked: SpinLock<bool>,
    /// 未结 CPU 关系：包含显式 Attach 控制与资源 Open 映射。
    ///
    /// 成员与 Registry 项是同一个 `Arc<ViewBinding>`，所以“还欠着谁”只有这一份
    /// 事实：只有显式结清（`Space::retire_view` / `reap_views`）才撒下成员，
    /// 不靠 `Weak` 升不上或 Drop 替它做主。撤销方也按它把名下关系收回去关。
    bindings: SpinLock<Vec<Arc<ViewBinding>>>,
    /// 这份来源自己分出的子段：几何、历史分支索引与子记录弱引用。
    ///
    /// 子记录丢了也不摘：未知只占它那一段精确范围，不误报空闲，也不封整个父段。
    derivations: SpinLock<Vec<Derivation>>,
    /// 仍压着这一段的**独立持有**：窗口与持有人。
    ///
    /// 这是“这一段不空闲”的显式事实：由 `IndependentHold` 在同一 `commit` 段里登记，
    /// 随物理保留结束而撤。不看 `is_pinned`、不数 `Arc`、也不靠 `Weak` 生存。
    /// 持有人只用于自检读口，不参与判据：豁免按**这一份登记的身份**认，不按任务。
    retentions: SpinLock<Vec<(Arc<Geometry>, TaskId)>>,
    /// 派生记录的出生证明；独立根一条也没有。
    lineage: Option<Lineage>,
    phase: SpinLock<Phase>,
}

/// 私有根在根 Grant 接手前的权限幅面（上限仍由 `ceiling` 收）。
fn private_permission() -> Permission {
    Permission::FETCH | Permission::STORE | Permission::VEST
}

/// 这份权限是否覆盖这次访问：取要 FETCH，写还要 STORE。
fn covers(permission: Permission, access: Access) -> bool {
    permission.contains(Permission::FETCH)
        && (!access.contains(Access::WRITE) || permission.contains(Permission::STORE))
}

impl SourceAuthorization {
    pub(crate) fn new(
        range: FlatRange,
        ceiling: Access,
        owner: TaskId,
    ) -> Result<Arc<Self>, PieFail> {
        Arc::try_new(Self {
            range: SpinLock::new(Some(range)),
            ceiling,
            owner,
            qualification: Creation::new(),
            holds: AtomicUsize::new(0),
            inflight: SpinLock::new(Vec::new()),
            revoked: SpinLock::new(false),
            bindings: SpinLock::new(Vec::new()),
            derivations: SpinLock::new(Vec::new()),
            retentions: SpinLock::new(Vec::new()),
            lineage: None,
            phase: SpinLock::new(Phase::Private),
        })
        .map_err(|_| PieFail::OoM)
    }

    /// 派生记录：生来就在根相位。
    ///
    /// 它没有私有阶段，也就发不出任何私有创建资格（`private_owner` 恒假）；
    /// 段内的真实范围归它自己那一枚 Pole，父侧只据几何记住这一段已分出去。
    pub(crate) fn derived(
        meta: &Arc<PoleMeta>,
        token: PieToken,
        grant: Arc<Grant>,
        ceiling: Access,
        owner: TaskId,
        lineage: Lineage,
    ) -> Result<Arc<Self>, PieFail> {
        Arc::try_new(Self {
            range: SpinLock::new(None),
            ceiling,
            owner,
            qualification: Creation::new(),
            holds: AtomicUsize::new(0),
            inflight: SpinLock::new(Vec::new()),
            revoked: SpinLock::new(false),
            bindings: SpinLock::new(Vec::new()),
            derivations: SpinLock::new(Vec::new()),
            retentions: SpinLock::new(Vec::new()),
            lineage: Some(lineage),
            phase: SpinLock::new(Phase::Root {
                meta: Arc::downgrade(meta),
                token,
                grant,
            }),
        })
        .map_err(|_| PieFail::OoM)
    }

    /// 整段来源范围；已交给根 Grant 就是 `None`（真实范围在 Pole 手里）。
    pub(crate) fn own_range(&self) -> Option<FlatRange> {
        self.range.lock().clone()
    }

    pub(crate) fn ceiling(&self) -> Access {
        self.ceiling
    }

    pub(crate) fn private_owner(&self, task: TaskId) -> bool {
        self.owner == task && matches!(&*self.phase.lock(), Phase::Private)
    }

    /// 相对整段来源的页窗口解析成可用范围。
    ///
    /// 私有阶段用自己的强持范围；已接管就向 Pole 要，且要求那枚 Grant 仍活——
    /// 于是撤销后的普通引用解析不到范围，自然既不挂载也不 pin 住来源。
    pub(crate) fn window(&self, offset: PageOffset, count: PageCount) -> Option<FlatRange> {
        if let Some(range) = self.own_range() {
            return range.range(offset, count).ok();
        }
        let (meta, _, _) = self.root()?;
        self.permission()?;
        meta.source().range(offset, count).ok()
    }

    /// 整段来源范围（创建事务要用它建 Pole / 判完整）。
    pub(crate) fn full_range(&self) -> Option<FlatRange> {
        if let Some(range) = self.own_range() {
            return Some(range);
        }
        self.root().map(|(meta, _, _)| meta.source())
    }

    /// 整段来源的页起点（子范围 Acquire 要把视图的坐标换成相对本来源的坐标）。
    pub(crate) fn index(&self) -> Option<PageIndex> {
        self.full_range().map(|range| range.index())
    }

    /// 该来源当前的权限幅面；`None` = 已撤销。
    pub(crate) fn permission(&self) -> Option<Permission> {
        let _gate = self.revoked.lock();
        self.permission_ungated()
    }

    /// 闸门已持有时读当前权限幅面（不再回取闸门）。
    fn permission_ungated(&self) -> Option<Permission> {
        match &*self.phase.lock() {
            Phase::Private => Some(private_permission()),
            Phase::Root { grant, .. } => grant.permission(),
        }
    }

    /// 同一条提交边界，但上限取自**这次引用所在的那条分支**（子 Pie 的 Grant）。
    ///
    /// 不同 Grant 分支不是同一事实：子 Pie 收窄/撤销后，它的分支不得再挂可写关系，
    /// 而另一条分支不受影响。
    pub(crate) fn branch_check(&self, branch: Option<&Grant>, access: Access) -> bool {
        let gate = self.revoked.lock();
        !*gate && self.branch_permits(branch, access)
    }

    fn branch_permits(&self, branch: Option<&Grant>, access: Access) -> bool {
        // 先把相位拷出来放锁，再升弱引用：临时 `Arc` 可能是最后一份强持，
        // 它的析构会走到 `retire_view` / `settle` 再回本文件取锁。同 `manages`。
        let phase = Phase::clone(&self.phase.lock());
        let alive = match phase {
            Phase::Private => true,
            Phase::Root { meta, .. } => meta.upgrade().is_some_and(|meta| meta.alive()),
        };
        if !alive {
            return false;
        }
        let permission = match branch {
            Some(grant) => grant.permission(),
            None => self.permission_ungated(),
        };
        permission.is_some_and(|p| covers(p, access))
    }

    /// 在途登记（闸门下的短临界区）：还没被撤销才收下这个键。
    pub(crate) fn enter_publish(
        &self,
        id: usize,
        branch: Option<&Grant>,
        access: Access,
    ) -> Result<(), PieFail> {
        self.enter_flight(id, None, branch, access)
    }

    /// 同一条在途登记，但这次发表**占住了**这一段窗口：
    ///
    /// 别的发表（派生 / Open / Accord / Attach）在这笔在途结束前不得插进这一段
    /// （见 `check_publish`）；撤销方与不带窗口的那一档一样看待它。
    pub(crate) fn enter_publish_range(
        &self,
        id: usize,
        window: Arc<Geometry>,
    ) -> Result<(), PieFail> {
        self.enter_flight(id, Some(window), None, Access::READ)
    }

    fn enter_flight(
        &self,
        id: usize,
        window: Option<Arc<Geometry>>,
        branch: Option<&Grant>,
        access: Access,
    ) -> Result<(), PieFail> {
        let gate = self.revoked.lock();
        if *gate || !self.branch_permits(branch, access) {
            return Err(PieFail::Denied);
        }
        let mut inflight = self.inflight.lock();
        inflight.try_reserve(1).map_err(|_| PieFail::OoM)?;
        inflight.push((id, window));
        Ok(())
    }

    pub(crate) fn begin_withdraw(
        &self,
    ) -> Result<crate::machine::sync::spin::SpinLockGuard<'_, bool>, PieFail> {
        let gate = self.revoked.lock();
        if !self.inflight.lock().is_empty() {
            return Err(PieFail::Busy);
        }
        Ok(gate)
    }

    pub(crate) fn pause_publish(&self) -> Result<(), PieFail> {
        let mut gate = self.begin_withdraw()?;
        if *gate || !self.branch_permits(None, Access::READ) {
            return Err(PieFail::Denied);
        }
        *gate = true;
        Ok(())
    }

    pub(crate) fn resume_publish(&self) {
        let mut gate = self.revoked.lock();
        if self.branch_permits(None, Access::READ) {
            *gate = false;
        }
    }

    /// 在途登记失效（提交或回滚都走这一步）：同一笔键的两条一次撤完。
    pub(crate) fn leave_publish(self: &Arc<Self>, id: usize) {
        let empty = {
            let mut inflight = self.inflight.lock();
            inflight.retain(|(held, _)| *held != id);
            inflight.is_empty()
        };
        if empty {
            self.settle();
        }
    }

    /// 备下一条成员的位置（可失败的准备；调用方在同一提交段里紧接着写入）。
    pub(crate) fn reserve_binding(&self) -> Result<(), PieFail> {
        let mut members = self.bindings.lock();
        members.try_reserve(1).map_err(|_| PieFail::OoM)
    }

    /// 写入一条成员：容量已在同一提交段备好，这一步只推入。
    pub(crate) fn register_binding(&self, binding: Arc<ViewBinding>) {
        self.bindings.lock().push(binding);
    }

    /// 撒下一条成员：这份绑定已结清（`Space::retire_view` / `reap_views` 的唯一收尾）。
    ///
    /// 最后一条成员撤下就是显式的“结清”事件：来源不再占着任何东西时顺带向上收尾，
    /// 不在 `Drop` 里猜。
    pub(crate) fn release_binding(self: &Arc<Self>, binding: &Arc<ViewBinding>) {
        let empty = {
            let mut members = self.bindings.lock();
            members.retain(|held| !Arc::ptr_eq(held, binding));
            members.is_empty()
        };
        if empty {
            self.settle();
        }
    }

    /// 备下一条子段分配的位置（可失败的准备；调用方在同一提交段里紧接着发表）。
    pub(crate) fn reserve_derivation(&self) -> Result<(), PieFail> {
        self.derivations
            .lock()
            .try_reserve(1)
            .map_err(|_| PieFail::OoM)
    }

    /// 发表一条子段分配：容量已在同一提交段备好，这一步只推入。
    pub(crate) fn publish_derivation(&self, derivation: Derivation) {
        self.derivations.lock().push(derivation);
    }

    /// 备下一条独立持有登记的位置（可失败的准备；与物理保留同在一个提交段）。
    pub(crate) fn reserve_retention(&self) -> Result<(), PieFail> {
        self.retentions
            .lock()
            .try_reserve(1)
            .map_err(|_| PieFail::OoM)
    }

    /// 写入一条独立持有登记：容量已在同一提交段备好，这一步只推入。
    pub(crate) fn register_retention(&self, window: Arc<Geometry>, holder: TaskId) {
        self.retentions.lock().push((window, holder));
    }

    /// 撤下一条独立持有登记：按窗口身份认这一条，不当授权、也不动别的条。
    ///
    /// 这是显式的“这一段不再被独立持有”事件：这一份来源自己不再占着任何东西时，
    /// 顺带从父分配槽里向上收尾（否则父侧那一槽要等到子记录消失才无主可依）。
    pub(crate) fn release_retention(self: &Arc<Self>, window: &Arc<Geometry>) {
        self.retentions
            .lock()
            .retain(|(held, _)| !Arc::ptr_eq(held, window));
        self.settle();
    }

    /// 子段占用判据：这一段已经作为另一份资源交出去了，**任何形式**的再用都不许。
    ///
    /// 取得又一份持有（`Acquire` 四个来源都算）只走这一条：Residency 不是互斥授权，
    /// 同一份合法来源上再落一份保留并不把这一段交出去。
    ///
    /// 调用方必须**在同一 `commit` 段里**把它与发表一起做：本函数只读当场事实，
    /// 不自己取提交锁——单拿谓词去判、再在段外发表，中间会有人插进一条新占用。
    /// 已可靠结束的分配槽在这一步里顺带收掉（摘槽也取提交段，见 `settle`）。
    pub(crate) fn check_use(&self, offset: PageOffset, count: PageCount) -> Result<(), PieFail> {
        #[cfg(debug_assertions)]
        crate::core::work::assert_held();
        self.reap_derivations();
        if self
            .derivations
            .lock()
            .iter()
            .any(|slot| slot.overlaps(offset, count))
        {
            return Err(PieFail::Busy);
        }
        Ok(())
    }

    /// 发表准入：在子段占用之外，**别人的**独立持有与**在途已占的那一段**也挡。
    ///
    /// 派生 / Open / Accord 是新授权，Attach / Transfer 是新映射，两者都算发表；
    /// `own` 是这次发表自己那条独立持有的窗口——只有它自己那一份登记不挡
    /// （持有人就靠这条引用拿帧）。带窗口的在途登记是别人已经核过的移动范围，
    /// 一律不得插进去（持有人自己那份不含在内，它不占范围）。
    pub(crate) fn check_publish(
        &self,
        offset: PageOffset,
        count: PageCount,
        own: Option<&Arc<Geometry>>,
    ) -> Result<(), PieFail> {
        self.check_use(offset, count)?;
        let held = self.retentions.lock().iter().any(|(window, _)| {
            !own.is_some_and(|own| Arc::ptr_eq(own, window)) && window.overlaps(offset, count)
        });
        let claimed = self.inflight.lock().iter().any(|(_, window)| {
            window
                .as_ref()
                .is_some_and(|window| window.overlaps(offset, count))
        });
        if held || claimed {
            return Err(PieFail::Busy);
        }
        Ok(())
    }

    /// 只收槽、不判准入：收掉**可证明结束**的分配槽，不可证明的留在原地。
    ///
    /// 由准入与显式结清事件（最后一条成员、最后一枚授权、显式 Seal、
    /// 独立持有结束）调用。它取提交段：新授权（`accord` 的登记）与新关系的发表
    /// 都在各自的段里做，“末份判定 → 摘槽”必须与它们互斥，否则判定读到空、
    /// 随后又落下一枚新授权或新成员，父槽已经摘了。
    pub(crate) fn reap_derivations(&self) {
        let _commit = crate::core::work::commit();
        let mut slots = self.derivations.lock();
        let mut index = 0;
        while index < slots.len() {
            if self.derivation_done(&slots[index]) {
                slots.remove(index);
                continue;
            }
            index += 1;
        }
    }

    /// 这一条子段是不是**可证明地**结束了：子记录还在，且它自己不再占着任何东西。
    ///
    /// 升不上就是未知：未知照旧占住它那一段，既不按空闲处理，也不封整个父段。
    /// 子记录自己那份独立持有也是它占着这一段的事实（见 `finished`）：
    /// 帧仍被独立持有时，父侧那一槽不摘，父段也不许再分出去。
    fn derivation_done(&self, slot: &Derivation) -> bool {
        slot.child().upgrade().is_some_and(|child| child.finished())
    }

    /// 授权、CPU 关系、子段、保留及在途发表均已结束；普通引用的存在不参与结清。
    fn finished(&self) -> bool {
        let empty = {
            let bindings = self.bindings.lock();
            let derivations = self.derivations.lock();
            let retentions = self.retentions.lock();
            let inflight = self.inflight.lock();
            bindings.is_empty()
                && derivations.is_empty()
                && retentions.is_empty()
                && inflight.is_empty()
        };
        empty && !self.holds_authority()
    }

    /// 还有没有可用授权：取这一份来源自己那枚 Pole 的活状态与它名下的授权记录，
    /// 不看相位里那一枚 root Grant（Seal 之后死 token 不该永久占位）。
    fn holds_authority(&self) -> bool {
        // 同 `manages`：相位拷出来先放锁，升弱引用（可能拿到最后一份强持）
        // 与它的析构都不在相位锁里做，否则析构会反锁同一把。
        let phase = Phase::clone(&self.phase.lock());
        match phase {
            Phase::Private => true,
            Phase::Root { meta, .. } => meta.upgrade().is_some_and(|meta| meta.authorizing()),
        }
    }

    /// 派生记录的出生证明：父记录与同一份几何。独立根为 `None`。
    pub(crate) fn lineage(&self) -> Option<(Arc<SourceAuthorization>, Arc<Geometry>)> {
        self.lineage
            .as_ref()
            .map(|lineage| (lineage.parent().clone(), lineage.geometry().clone()))
    }

    /// 由**显式事件**发起的向上收尾：这一份来源真的不再占着任何东西时，从父分配槽里
    /// 摘掉自己，再沿同一条链逐代向上。迭代而不递归，不在长链上同时持多把锁。
    ///
    /// 它取提交段，与“新授权 / 新成员 / 新关系发表”互斥：末份判定与摘槽必须原子，
    /// 否则判定读到空、随后又落下一枚新授权或新成员，而父槽已经摘了。析构路径上走它
    /// 也安全：`Pie`/`Hold` 的掉落点要么在顶层，要么本就持着这一把（`pend` 串行锁是
    /// 豁免级，不参与锁序）；`Space::retire_view` 那条本就已持提交段。
    pub(crate) fn settle(self: &Arc<Self>) {
        let _commit = crate::core::work::commit();
        let mut node = self.clone();
        loop {
            if !node.finished() {
                return;
            }
            let Some((parent, geometry)) = node.lineage() else {
                return;
            };
            parent.release_derivation(&geometry);
            node = parent;
        }
    }

    /// 按几何身份摘掉一条子段分配：不当授权，也不动别的槽。
    fn release_derivation(&self, geometry: &Arc<Geometry>) {
        self.derivations
            .lock()
            .retain(|slot| !Arc::ptr_eq(slot.geometry(), geometry));
    }

    /// Forget 改指：这一份来源里以 `forgotten` 为分支索引的分配槽与显式控制记录
    /// 一并换成新父分支。权限仍是创建时的快照，不重算、也不部分复制。
    pub(crate) fn retarget_branch(
        self: &Arc<Self>,
        forgotten: PieToken,
        parent: PieToken,
    ) -> Result<(), PieFail> {
        // 可失败的准备在前：目标空间快照备不下就整条不动。
        let spaces = self.spaces()?;
        for slot in self.derivations.lock().iter_mut() {
            slot.retarget(forgotten, parent);
        }
        for space in spaces {
            space.views.retarget_branch(self, forgotten, parent);
        }
        Ok(())
    }

    /// 这份来源名下未结的独立持有条数（自检读口）。
    #[cfg(debug_assertions)]
    pub(crate) fn retention_count(&self) -> usize {
        self.retentions.lock().len()
    }

    /// 这份来源名下未结的子段分配数（自检读口）。
    #[cfg(debug_assertions)]
    pub(crate) fn derivation_count(&self) -> usize {
        self.derivations.lock().len()
    }

    /// 这份来源的子段表里有没有这一份几何（自检读口：两边是不是同一个 `Arc`）。
    #[cfg(debug_assertions)]
    pub(crate) fn owns_geometry(&self, geometry: &Arc<Geometry>) -> bool {
        self.derivations
            .lock()
            .iter()
            .any(|slot| Arc::ptr_eq(slot.geometry(), geometry))
    }

    /// 这一份几何槽里记的历史分支索引（自检读口）。
    #[cfg(debug_assertions)]
    pub(crate) fn derivation_token(&self, geometry: &Arc<Geometry>) -> Option<PieToken> {
        self.derivations
            .lock()
            .iter()
            .find(|slot| Arc::ptr_eq(slot.geometry(), geometry))
            .map(Derivation::token)
    }

    /// 未结 CPU 身份的快照；不复制关系或物理持有。
    pub(crate) fn bindings(&self) -> Result<Vec<Arc<ViewBinding>>, PieFail> {
        let held = self.bindings.lock();
        let mut members = Vec::new();
        members.try_reserve(held.len()).map_err(|_| PieFail::OoM)?;
        members.extend(held.iter().cloned());
        Ok(members)
    }

    /// 仍可追溯的派生记录；槽在而记录丢失时不能宣称子图已关。
    pub(crate) fn children(&self) -> Result<Vec<Arc<Self>>, PieFail> {
        let mut children = Vec::new();
        {
            let slots = self.derivations.lock();
            children
                .try_reserve(slots.len())
                .map_err(|_| PieFail::OoM)?;
            for slot in slots.iter() {
                children.push(slot.child().upgrade().ok_or(PieFail::Busy)?);
            }
        }
        Ok(children)
    }

    /// 已失活来源仍须保留补关入口的义务。
    pub(crate) fn pending_close(&self) -> bool {
        !self.bindings.lock().is_empty()
            || !self.derivations.lock().is_empty()
            || !self.inflight.lock().is_empty()
    }

    /// 这份来源登过关系的目标空间：由成员快照派生并去重。
    ///
    /// 先在源锁下克隆成员再放锁升级：最后一份 `Space` 强引用不在源锁里析构。
    pub(crate) fn spaces(&self) -> Result<Vec<Arc<Space>>, PieFail> {
        let members = self.bindings()?;
        let mut spaces = Vec::new();
        spaces
            .try_reserve(members.len())
            .map_err(|_| PieFail::OoM)?;
        for binding in &members {
            if let Some(space) = binding.space().upgrade()
                && !spaces.iter().any(|held| Arc::ptr_eq(held, &space))
            {
                spaces.push(space);
            }
        }
        Ok(spaces)
    }

    /// 这份来源名下**未结**成员数（自检读口；不另存计数）。
    #[cfg(debug_assertions)]
    pub(crate) fn binding_count(&self) -> usize {
        self.bindings.lock().len()
    }

    /// 这份来源名下是否仍有这一份未结绑定。
    pub(crate) fn owns_binding(&self, binding: &Arc<ViewBinding>) -> bool {
        self.bindings
            .lock()
            .iter()
            .any(|held| Arc::ptr_eq(held, binding))
    }

    /// 撤销这份来源：在途发布未结清时返回 `Busy`，由调用方重试。
    /// 具名拿闸门，不在 match 侦式里再取同一把锁。
    pub(crate) fn revoke(&self) -> Result<(), PieFail> {
        let mut gate = self.revoked.lock();
        if !self.inflight.lock().is_empty() {
            return Err(PieFail::Busy);
        }
        *gate = true;
        Ok(())
    }

    /// 这份来源是否已被撤销：**短读**一次闸门，不把闸门留到本次操作结束。
    ///
    /// 授权已经交出去的引用（根 Grant）看的是同一条闸门，所以迁移不需要
    /// 另持一条锁跨整个执行：真正的串行由调用者的 pend 负责。
    pub(crate) fn revoked(&self) -> bool {
        *self.revoked.lock()
    }

    /// 这份来源当前的**管理者**是不是 `task`：它当场能不能替这份来源作主。
    ///
    /// 私有阶段只有真正的属主；交给根 Grant 之后，调用者表里那枚 **root token**
    /// （`sire` 为空的原始一枚）必须与相位记的是同一枚，pie、Pole 与根 Grant 都还活，
    /// 且尚未交出（`heir` 为空）——“已交出去的那一枚”照旧不可用，这里不靠 ONLY
    /// 权限位，也不看 qualification 或 `Arc` 强引用计数。
    ///
    /// 相位里的 Pole 只按弱身份比对；活与同一枚都由表里那一枚 `pie` 自己给出，
    /// 所以调用方须在同 Task 的 pend 之下调它：表不变，才有“临时强引用不会是
    /// 最后一份”这条持有链。
    pub(crate) fn manages(&self, task: &crate::core::work::unit::task::Task) -> bool {
        if self.revoked() {
            return false;
        }
        // 短锁里克隆相位再判：放锁之后才取 pie 表与 Grant 各自的锁，不与
        // “先 Grant 后相位”的读法（`Hold::resolve`）撞成逆序。
        let phase = Phase::clone(&self.phase.lock());
        match phase {
            Phase::Private => self.owner == task.ident.id,
            Phase::Root { meta, token, grant } => {
                // 表里没有那一枚：调用者不持 root token，当场不是管理者。
                let Some(pie) = super::super::locate(task, token) else {
                    return false;
                };
                pie.sire().is_none()
                    && pie.heir().is_none()
                    // `pie.alive()` 含这份 Mail（Pole）自己活着；它与下面“同 Arc”
                    // 合起来就是“同一枚 Pole 且活”，不只是 Grant 还在答话。
                    && pie.alive()
                    && Arc::ptr_eq(pie.grant(), &grant)
                    // 那一下克隆出的临时强引用不会是最后一份：表里这一枚 `pie` 自己
                    // 就持着它，而这张表在调用者的 pend 之下不会变。
                    && pie
                        .mail()
                        .pole()
                        .is_some_and(|held| Weak::as_ptr(&meta) == Arc::as_ptr(&held))
            }
        }
    }

    /// 已接手的根 Grant、它的 token 与**根 Grant 自己的**那枚 `Grant`。
    /// 还没接手就是 `None`。三段来自同一份相位，调用方不再各自去表里另找一遍。
    pub(crate) fn root(&self) -> Option<(Arc<PoleMeta>, PieToken, Arc<Grant>)> {
        match &*self.phase.lock() {
            Phase::Root { meta, token, grant } => Some((meta.upgrade()?, *token, grant.clone())),
            Phase::Private => None,
        }
    }

    /// 登记一份普通引用。
    pub(crate) fn claim_hold(&self) {
        self.holds.fetch_add(1, Ordering::AcqRel);
    }

    /// 解除一份普通引用。最后一份普通引用消失时，**未消费的一次性资格显式结束**。
    ///
    /// 这一定不靠 `Arc` 计数也不再靠授权记录被回收：接上 View 登记后仍有人持着
    /// 这份记录，资格该结束还是结束。
    pub(crate) fn release_hold(&self) {
        if self.holds.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.qualification.consume();
        }
    }

    /// 一次性创建资格：单次 `claim` / `consume`，协议与既有 `Creation` 同一套。
    pub(crate) fn claim(&self) -> Result<(), PieFail> {
        self.qualification.claim()
    }

    pub(crate) fn release_claim(&self) {
        self.qualification.release();
    }

    /// 一次性资格用掉了（Pole 已接手）。
    pub(crate) fn consume(&self) {
        self.qualification.consume();
    }

    /// 把私有根交给新根 Grant：真实范围同时从授权记录转出（从此由 Pole 持有），
    /// 并返回私有阶段已建的关系，让创建事务把它们登到新 token 下。
    /// 已经接手过就拒（一个来源只有一条根）。
    pub(crate) fn adopt(
        &self,
        meta: &Arc<PoleMeta>,
        token: PieToken,
        grant: Arc<Grant>,
    ) -> Result<(), PieFail> {
        let mut phase = self.phase.lock();
        if matches!(&*phase, Phase::Root { .. }) {
            return Err(PieFail::Denied);
        }
        // 真实范围先交给 Pole，再改相位：交接期间授权记录不再强持来源。
        *self.range.lock() = None;
        *phase = Phase::Root {
            meta: Arc::downgrade(meta),
            token,
            grant,
        };
        Ok(())
    }

    /// 事务中途失败：把刚交出去的范围放回私有阶段。
    pub(crate) fn restore_private(&self, range: FlatRange) {
        let mut phase = self.phase.lock();
        *self.range.lock() = Some(range);
        *phase = Phase::Private;
    }
}

unsafe impl Send for SourceAuthorization {}
unsafe impl Sync for SourceAuthorization {}
