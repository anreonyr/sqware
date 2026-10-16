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
use crate::core::work::unit::space::Space;
use crate::machine::sync::SpinLock;
use env::{Permission, PieFail, PieToken, TaskId};

use super::creation::Creation;
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
    /// **在途发布**的关系号：闸门下登记，撤销方看到它就不结清。
    inflight: SpinLock<Vec<usize>>,
    /// 发布与撤销的共同闸门。
    revoked: SpinLock<bool>,
    /// 这份来源登记过关系的那些目标空间（撤销方据此把名下关系收回去关）。
    spaces: SpinLock<Vec<Weak<Space>>>,
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
            spaces: SpinLock::new(Vec::new()),
            phase: SpinLock::new(Phase::Private),
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
        let alive = match &*self.phase.lock() {
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

    /// 在途登记（闸门下的短临界区）：还没被撤销才收下这个号。
    pub(crate) fn enter_publish(
        &self,
        id: usize,
        branch: Option<&Grant>,
        access: Access,
    ) -> Result<(), PieFail> {
        let gate = self.revoked.lock();
        if *gate || !self.branch_permits(branch, access) {
            return Err(PieFail::Denied);
        }
        let mut inflight = self.inflight.lock();
        inflight.try_reserve(1).map_err(|_| PieFail::OoM)?;
        inflight.push(id);
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

    /// 在途登记失效（提交或回滚都走这一步）。
    pub(crate) fn leave_publish(&self, id: usize) {
        self.inflight.lock().retain(|held| *held != id);
    }

    /// 记下这份来源登记过关系的一个目标空间（去重，容量须在发布前备好）。
    pub(crate) fn remember_space(&self, space: &Arc<Space>) -> Result<(), PieFail> {
        let mut spaces = self.spaces.lock();
        if spaces
            .iter()
            .any(|held| held.upgrade().is_some_and(|held| Arc::ptr_eq(&held, space)))
        {
            return Ok(());
        }
        spaces.try_reserve(1).map_err(|_| PieFail::OoM)?;
        spaces.push(Arc::downgrade(space));
        Ok(())
    }

    /// 这份来源登记过关系的目标空间（撤销方在锁外逐个收）。
    pub(crate) fn spaces(&self) -> Result<Vec<Arc<Space>>, PieFail> {
        let spaces = self.spaces.lock();
        let mut result = Vec::new();
        result.try_reserve(spaces.len()).map_err(|_| PieFail::OoM)?;
        result.extend(spaces.iter().filter_map(Weak::upgrade));
        Ok(result)
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
