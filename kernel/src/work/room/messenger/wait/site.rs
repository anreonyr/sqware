// 站点表（site）——挂起任务的唯一容器：唤醒源（`WakeKey`）→ 信标 + 等待者队列。
//
// 分片版（每片一把 L3 锁 + 一个 HashMap），站点寿命与信标操作（`take_beacon` /
// `prune`）都收在本模块。跨到 `messenger` 一级的条目用 `pub(in super::super)`
// （= `messenger`）——`doom.rs` 与 `messenger::rip` 要的那几个，刚好够。

use alloc::boxed::Box;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;

use env::HoleDir;
use hashbrown::HashMap;

use crate::lock::{Level, OnceLock, SpinLock};
use crate::work::unit::life::Life;
use crate::work::unit::task::{Task, TaskState};

// ── 类型 ──

/// 唤醒源：谁会把等待者叫醒。**每个可等地各占一个变体**（空间槽 / 孔 / 铃 / 任务 /
/// 权限表 / 组 / 闹钟共七个），键即身份。
///
/// **没有位打包**：`Space` 的两个字段各自完整，不再把 asid 挤进高 16 位、用户键
/// 截到低 48 位。旧 `WaitKey::compose` 的单射性靠掩码保证，还因此逼出一个
/// `#[inline(never)]` 的 mask helper 去躲 size 优化下的错联——枚举下
/// 这两样都不需要：没有 mask，就没有 mask 错联。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WakeKey {
    /// 调用方命名空间里的裸整数（`RoomCall::Wait` / `Wake`）：空间身份 + 槽位。
    Space { space: usize, slot: usize },
    /// 资源就绪（`MailCall::Wait`；hole 的 push / pull / seal 投信）。
    ///
    /// `hole` 用裸整数而非 `mail::HoleId`：依赖方向必须保持 mail → room 单向，
    /// 引 `HoleId` 就成了环。
    Hole { hole: usize, dir: HoleDir },
    /// 铃响（`MailCall::Wait`；`nole::ring` 置位并投信、`nole::hush` 清位）。
    ///
    /// **没有方向字段**：门铃只有一条方向，键只需身份。同 `Hole`，用裸整数而非
    /// `mail::NoleId`——依赖方向必须保持 mail → room 单向。
    Nole { id: usize },
    /// 目标任务回收（`UnitCall::Join`）。
    Task { id: usize },
    /// 我这枚任务的**权限表**落进一枚新孔（`Accord` 的收方那一侧）。
    ///
    /// 信标是**一次事件，不是计数**：落了三枚也只答一次（醒来自己扫表）。投信的只有
    /// 内核（`gate::accord` 落表之后）——用户态没有投这一族的动词，伪造不出"你的表变了"。
    Pies { task: usize },
    /// 一枚组（`work::mail::tole`）自己的键：**它的等待者总得有个站点可挂**。
    ///
    /// 与 `Hole{id}` / `Nole{id}` 同构：组是内核命名的新对象，键取它的全局身份。
    Tole { id: usize },
    /// 无人投信——只有期限会响（`RoomCall::Park`）。
    ///
    /// 键就是那个睡眠者本人：park 没有信号源，能唤醒它的只有它自己那次到点登记。
    Alarm { task: usize },
}

impl WakeKey {
    /// 折成 64 位——**只供分片**，不承载语义（相等性仍由 `Eq` 判定）。
    pub(super) fn fold(self) -> u64 {
        match self {
            WakeKey::Space { space, slot } => {
                (space as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ slot as u64
            }
            WakeKey::Hole { hole, dir } => {
                (hole as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ dir as u64
            }
            // 与 `Hole` 换一个乘数：两个 id 空间各起一份计数器、数值会重叠，
            // 分片散列因此各走一路（相等性仍由 `Eq` 判，这里只影响分片）。
            WakeKey::Nole { id } => (id as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9),
            WakeKey::Task { id } => (id as u64).wrapping_mul(0xD6E8_FEB8_6659_FD93),
            WakeKey::Alarm { task } => (task as u64).wrapping_mul(0xA24B_AED4_963E_E407),
            WakeKey::Pies { task } => (task as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F),
            WakeKey::Tole { id } => (id as u64).wrapping_mul(0x1656_67B1_9E37_79F9),
        }
    }
}

/// 一个唤醒源的等待位：遗留信号（信标）+ **等待链的两头** + 该键的存活单元。
///
/// 三种唤醒源共用本类型（旧版 `WaitSite` / `JoinSite` 字段逐个相同——各自一份是
/// 键的 Rust 类型不同逼出来的）。
///
/// **队列在等待者的载荷里**（`TaskState::Blocked::next`），这里只存两头——与
/// `SchedulerInner` 的就绪队列、`Husks` 的躯壳队列同一手法。理由也一样：入队落在
/// `block` ④（`swap()` 之后，没有失败域）与 `redeem` / `wipe` 这两条放行路径上，
/// 任何"要么扩容要么 halt"的容器在这三条路上都是地雷；而"容量需求是并发占用"与
/// 一生一次的 `try_reserve(1)` 对不上（不累加：备下的那一格可能被同键的另一个等待者
/// 先占走）。改链之后，入队 = 两次指针写。
pub(in super::super) struct Site {
    /// 遗留信号（信标）：wake 无等待者 → 置位；wait 见位 → 消费即回（防漏唤醒）。
    pub(in super::super) pend: bool,
    /// 等待链的链头（`None` = 无人在等）。
    pub(in super::super) head: Option<Arc<Task>>,
    /// 链尾（多持一个强引用，等价 `SchedulerInner::tail` 那种缓存；链的所有权在节点间）。
    pub(in super::super) tail: Option<Arc<Task>>,
    /// 本键的**存活单元**（弱引用）——站点寿命＝资源寿命的那一半（A2）。
    ///
    /// `Weak` 放**值**里而非键里：键是 `HashMap` 的 key，必须 `Copy`/`Eq`。
    /// 一个键只有一份 `Life`（其唯一强持有者是那份资源），故不管哪个入口（wait /
    /// join）写入，这枚弱引用指向的都是同一个分配——**不需要第二张表**。
    ///
    /// 只有**读**：`prune` 判据与 `block` ④ 各读一次「死没死」。room 不接受任何
    /// 来自外部的「这个键死了」的说法。
    pub(in super::super) life: Weak<Life>,
    /// **转发格**：投信本键时，也要叫醒这些组站点（裸 id——`room` 不认识 `Tole`，
    /// 依赖方向必须保持 mail → room 单向）。目标死了/站点不在就当无事，不做寿命纠缠。
    ///
    /// 定长（[`FWD_MAX`]）：`wake` 没有失败通道，**读这一格必须零分配**；容量账因此
    /// 落在登记侧（`forward` 返 `Result`，`hang` 有失败域）而不是唤醒侧。
    ///
    /// **非空即内容**：它是 [`prune`] 判据的第三项——登记本身就得有个落脚处（`Site`
    /// 是它唯一的容器，没有第二张表）。故「为转发而建」的站点不会当场被收走。
    pub(in super::super) fwd: Fwd,
}

/// 一个站点最多被几个组转发（见 [`Site::fwd`]）。
///
/// # 为什么是**定长**
///
/// 这一格的两条路都必须**不可失败**：
///   - 唤醒侧（`wake`/`wipe` → `knock`）：投信方**没有失败通道**，且读它必须在放开本分片锁
///     之后（目标键在别的分片上）⇒ 只能整格拷出来，**拷不能分配**；
///   - 撤登记侧（`unforward`，被 `unhang` 与 `Tole` 的 `Drop`/封印调用）：**`Drop` 不能失败**
///     ⇒ 摘一格也不能分配。
///
/// 固定容量让两条路都退化成"改几格数组"，于是容量账全部落在**登记侧**：`forward` 返
/// `Result`，`hang` 有失败域，满了答 `OoM` 并**把刚挂的那一格退回**（不留"挂着却叫不醒"
/// 的半截状态）。**"满"是容量，不是内存不足**——同一枚成员（孔的一个方向 / 一枚铃）已被
/// `FWD_MAX` 只组同时关心。
///
/// # 它数与共享组无关（一条曾经的误记，记在这里免得再犯）
///
/// 转发格是 **"成员键 × 组"**：一枚成员挂进同一只组多少次都只占一格（`Fwd::attach` 按组
/// id 幂等），而把那只组**复制**给再多任务也不加格——那些任务加的是**组键上的等待链**
/// （链在任务载荷里，无上限）。所以共享组是**减压**：N 个等待者共享一只组键，而不是
/// N 只组各占一格。
///
/// `pub(crate)` 是为了让这条契约**可测**（`health::permit` 按它挂满、再挂一枚看回滚）。
pub(crate) const FWD_MAX: usize = 8;

/// 转发格：定长 ⇒ 唤醒侧在锁内**拷出来**（`Clone`，零分配）即可，不必持两把分片锁。
///
/// 每一格带**目标的存活单元**：成员推可能**早于**等组的人入 `block`（组站点还不存在），
/// 那一刻要替它建一枚"只带信标"的站点，而建站点必须有一条寿命边——没有它就只能建出
/// 一个会被 `prune` 当场删掉的空壳。弱引用不延长寿命，故这一格仍不算寿命纠缠。
#[derive(Clone)]
pub(in super::super) struct Fwd {
    ids: [usize; FWD_MAX],
    lives: [Weak<Life>; FWD_MAX],
    len: usize,
}

impl Fwd {
    pub(in super::super) fn empty() -> Self {
        Self {
            ids: [0; FWD_MAX],
            lives: core::array::from_fn(|_| Weak::new()),
            len: 0,
        }
    }

    /// 登记一个组（幂等）；满了返 `Err`（登记侧有失败域）。
    pub(in super::super) fn attach(&mut self, tole: usize, life: Weak<Life>) -> Result<(), ()> {
        if self.ids[..self.len].contains(&tole) {
            return Ok(());
        }
        if self.len == FWD_MAX {
            return Err(());
        }
        self.ids[self.len] = tole;
        self.lives[self.len] = life;
        self.len += 1;
        Ok(())
    }

    /// 摘掉一个组；没登记过即无事。
    pub(in super::super) fn detach(&mut self, tole: usize) {
        if let Some(at) = self.ids[..self.len].iter().position(|&i| i == tole) {
            self.ids.copy_within(at + 1..self.len, at);
            // 弱引用不是 `Copy`：逐格挪（末格显式放掉，免得多留一份弱计数）。
            for i in at..self.len - 1 {
                self.lives.swap(i, i + 1);
            }
            self.lives[self.len - 1] = Weak::new();
            self.len -= 1;
        }
    }

    /// 一格都没登记？（定长格里 `len == 0`）——[`prune`] 的判据之一：空转发格的站点
    /// 与"没有转发格"的站点是同一件事。
    pub(in super::super) fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// 拷出当前登记：`(组号, 那一组的存活单元)`。
    pub(in super::super) fn entries(&self) -> impl Iterator<Item = (usize, &Weak<Life>)> {
        self.ids[..self.len]
            .iter()
            .copied()
            .zip(self.lives[..self.len].iter())
    }
}

// ── 簿记表（全部 L3） ──

/// 事件等待表的分片数。每片 = 一把 L3 锁 + 一个 HashMap；wait/wake/drain
/// 按 [`site_shard`] 纯函数路由到同片，跨片互不阻塞——把单点串行竞争降到
/// 1/SITE_SHARDS（典型 16）。分片数取 2 的幂：位与替代 mod。
pub(in super::super) const SITE_SHARDS: usize = 16;
const SITE_SHARDS_MASK: usize = SITE_SHARDS - 1;

/// 唤醒源 → 分片（pure function，所有路径一致：wait / wake / 投信 / 到期都经此）。
/// splitmix64 折叠 64→32 后按位与 SHARDS 掩码——高位低位的熵都被采样。
#[inline]
fn site_shard(key: WakeKey) -> usize {
    let h = key.fold().wrapping_mul(0x9E3779B97F4A7C15);
    ((h >> 32) ^ h) as usize & SITE_SHARDS_MASK
}

/// 一处分片：键 → 站点。
type Shard = SpinLock<HashMap<WakeKey, Site>>;

/// 站点表（Level::L3，绝不 3→3 嵌套）。**分片版**：每片一把
/// L3 锁 + HashMap，单一线性化点缩小到一片——wait / wake 跨片并行。
///
/// **一张表装三种唤醒源**：它们的等待者是同一种东西（任务 + 到点句柄），唤醒
/// 也是同一件事（摘出 → 放回就绪）。旧版把「等目标回收」单独放进 `joins`，理由
/// 只是键的 Rust 类型不同（`usize` vs `WaitKey`）——键成枚举之后，那个理由没了。
///
/// 锁纪律：仍是 L3、可与 timer 锁共存但**绝不 3→3 嵌套**（rip 路径循环逐片清，
/// 禁持跨片锁）。同 key 的所有等待者必落在同一分片（`site_shard` 纯函数保证），
/// 唤醒不必跨片扫描。
pub(in super::super) fn shard_at(shard: usize) -> &'static Shard {
    static SHARDS: OnceLock<Box<[Shard]>> = OnceLock::new();
    let arr: &'static [Shard] = SHARDS.get_or_init(|| {
        let mut v: Vec<Shard> = Vec::with_capacity(SITE_SHARDS);
        for _ in 0..SITE_SHARDS {
            v.push(SpinLock::new_level(Level::L3, HashMap::new()));
        }
        v.into_boxed_slice()
    });
    &arr[shard]
}

/// 本唤醒源的站点表分片。
pub(in super::super) fn sites(key: WakeKey) -> &'static Shard {
    shard_at(site_shard(key))
}

/// 信标先探：消费本键上的遗留信号。**缺键即无信标**——不 `or_insert`：空的、
/// 无信标的站点没有语义，不该被「先探」凭空造出来。
///
/// 取走信标后**当场 [`prune`]**：消费掉最后一枚信标的站点正好是「队列空 ∧ 无信标」
/// 那一类，不删就是空壳——而这正是 `wake` / `wipe` / `redeem` 三条路都记得做、
/// 唯独这里漏掉的一步。漏掉的表现：门里挂上 `cascade` 之后 `orphan` 从 0 变 2
/// （每次「等待者后到、消费掉遗留信标」都永久留一个空壳，站点表随该事件增长）。
pub(super) fn take_beacon(key: WakeKey) -> bool {
    let mut sites = sites(key).lock();
    let taken = match sites.get_mut(&key) {
        Some(site) if site.pend => {
            site.pend = false;
            true
        }
        _ => false,
    };
    if taken {
        prune(&mut sites, key);
    }
    taken
}

/// 站点存在的判据：**链非空 ∨ （（信标 ∨ 转发格非空）∧ 键还活着）**。不成立即删
/// ——空壳站点没有语义，留着就是 A2 那条「站点永不回收」的老毛病（`park` 每次睡眠
/// 都会留一个）。前置：已持有该分片的锁。
///
/// 三项的来历：
///   - **链非空**：有任务挂在这里，站点是它的容器（原判据）；
///   - **信标 ∧ 键还活着**：信标（`wake` 在无人在等时置的遗留信号）**只对未来到达
///     的等待者有意义**，而未来的等待者只可能来自活着的键——键一死，这枚信标就再也
///     无人认领。故 `life` 已死时信标随站点一起作废：**判据从「队列空 ∧ 无信标」
///     扩成「… ∨ 键已死」**（A2 的后半），站点寿命＝资源寿命。
///   - **转发格非空 ∧ 键还活着**：站点同时是**转发登记的落脚处**——`forward` 把「投信
///     本键时也认醒这些组」写进 `site.fwd`，而那是 `Site` 的字段、没有第二张表。
///     成员键上「没有等待者、也没有信标」恰恰是**常态**：等组的人等的是**组自己的
///     键**（`Tole`），从不挂在成员键上 ⇒ 少了这一项，`hang` 刚登记完就被 `forward`
///     末尾那次 `prune` 当场连站点一起删掉，登记**静默消失**：成员孔此后一投信只落
///     一枚没人认领的信标，等组的人睡到期限（无限等则永远）。寿命口径与信标同一条：
///     键一死投信方就不存在了（`try_push` 先判 `alive`），登记随之作废；活着的键上，
///     登记的寿命由**登记方**管（`unhang` / `Tole` 封印与 `Drop` 逐个 `unforward`），
///     故这一项不新增任何"站点随运行增长"的漏口——上限是**活着的组 × 挂着的格**。
///
/// 由此 `wipe` 不再留**墓碑**（「此键已死」那张空站点）：键自己会答（死亡 = 资源
/// 的强引用归零，`Weak::upgrade` 失败；见 [`Life`](crate::work::unit::life)）。
/// 此前每次 hole 封印、每次任务回收各留一个墓碑 ⇒ 站点表随运行单调增长；本判据把
/// 这个漏口关掉（实测 `tomb` 34 → 0；**反向验证**——把本判据与 `wipe` 的删站点一并
/// 改回原样——`tomb` 原样回到 34、总数原样回到 34）。
///
/// 站点因此只剩四种形态——**名字与判据只有这一处定义**（下文与别处引的 `live`/`tomb`/
/// `orphan` 三类计数出自当时的审计探针 `messenger::probe`；那枚探针已随那一轮收尾删掉，
/// 今天要数站点得重新加一枚）：
///   - **活**（链非空）：有任务挂在这里；
///   - **墓碑**（链空 **且** 有信标）：信号留着等**未来的**等待者认领。键还活着时
///     它有语义（`wake` 的记忆），故本函数**不删**它；键一死即落到下一类；
///   - **转站**（链空 **且** 无信标 **且** 转发格非空）：登记留着等**下一次投信**
///     （见上第三项）。同样只在键还活着时有语义；
///   - **孤儿**（链空 **且** 无信标 **且** 无转发格）：没有任何语义，正是本函数该删
///     的那一类。
///
/// 不变式（判据不含挂起中的等待者，故必须为真）：**链非空 ⇒ 键还活着**——能入链
/// 就意味着 `block` ④ 在锁内读到过「键活着」，而等待链的强持有者就是那份资源。
pub(in super::super) fn prune(sites: &mut HashMap<WakeKey, Site>, key: WakeKey) {
    if let Some(site) = sites.get(&key)
        && site.head.is_none()
        && (Life::dead(&site.life) || (!site.pend && site.fwd.is_empty()))
    {
        sites.remove(&key);
    }
}

// ── 内部辅助 ──

impl Site {
    /// 新站点：挂上**本键**的存活单元。站点借它判自己的寿命——资源一死，站点即
    /// 无意义（[`prune`] 当场删）。
    pub(super) fn new(life: &Weak<Life>) -> Self {
        Self {
            pend: false,
            head: None,
            tail: None,
            life: life.clone(),
            fwd: Fwd::empty(),
        }
    }

    /// 链尾入链（**纯指针写，零分配**）。
    ///
    /// 前置（由 `transform` 的断言兜底）：入链者状态为 `Blocked { next: None, .. }`
    /// ——站点只收等待者，且它不得还挂在别处的链上（否则就是一个任务两条链）。
    /// 只做法上的移动：`task` 按值进来，链尾与链头各留一份强引用。
    pub(super) fn push_back(&mut self, mut task: Arc<Task>) {
        debug_assert!(
            matches!(
                Task::exclusive(&mut task).state(),
                TaskState::Blocked { next: None, .. }
            ),
            "站点只收 Blocked 任务，且入链前不得挂在链上"
        );
        match self.tail.take() {
            // 空链：新节点即链头。
            None => self.head = Some(task.clone()),
            // 非空：接到原链尾的载荷上，再把链尾前移。
            Some(mut last) => *Task::blocked_next(&mut last) = Some(task.clone()),
        }
        self.tail = Some(task);
    }

    /// 链头出链（**摘链 + 清空离开者的 `next`**）；空链 → `None`。
    ///
    /// 清空那一步是硬要求：不清就等于"被取走的任务还持有它原来的后继"，同一段链
    /// 会有两个所有者。
    pub(super) fn pop_front(&mut self) -> Option<Arc<Task>> {
        let mut head = self.head.take()?;
        self.head = Task::blocked_next(&mut head).take();
        if self.head.is_none() {
            self.tail = None;
        }
        Some(head)
    }

    /// 走链摘掉**第一个满足 `hit` 的一环**（票对不上 / 人不对都由 `hit` 回答）；空链或
    /// 无人命中 → `None`。返回**被摘下的那一环**——调用方要什么就从它身上读：到期认领
    /// 要人（直接用），扑杀要票（`Task::blocked_ticket`）。
    ///
    /// 这是站点唯一的摘除入口。曾经有两个名字（`remove_ticket` / `remove_task`）：它们
    /// 各自只有一行、各自只有一个调用点，只是把「按什么找」与「要回答什么」包了一层
    /// ——那两句话本来就该由调用方说（`hit` 是**数据**，不是策略）。
    ///
    /// 走链逐节 clone 只为比较身份，不动链；命中时把后继接到前驱的载荷上，并清空
    /// 离开者（与 `Scheduler::starved_remove` 同形）。
    ///
    /// **链尾**必须跟着改：摘掉的若是最后一环（`next` 为空），新链尾就是它的前驱——
    /// 只在新链为空时才清 `tail` 是不够的，摘掉"非头的尾"会留下一个指着链外的
    /// `tail`，下一次 `push_back` 就把新等待者接到链外的节点上（链头压根到不了它）。
    ///
    /// 被摘下的那一环如果在锁内就地释放：调用方必持一枚强引用（杀路径）或它本来就在
    /// 锁外（到期路径），故它不可能是最后一个所有者——锁内不会触发 `Task::drop` 的
    /// drop 链（那会取 L2 空间锁）。
    pub(in super::super) fn remove_if(
        &mut self,
        hit: &mut dyn FnMut(&mut Arc<Task>) -> bool,
    ) -> Option<Arc<Task>> {
        let mut prev: Option<Arc<Task>> = None;
        let mut cur = self.head.clone();
        while let Some(mut node) = cur {
            if hit(&mut node) {
                let next = Task::blocked_next(&mut node).take();
                let was_tail = next.is_none();
                match &mut prev {
                    Some(p) => *Task::blocked_next(p) = next,
                    None => self.head = next,
                }
                if was_tail {
                    self.tail = prev;
                }
                return Some(node);
            }
            prev = Some(node.clone());
            cur = Task::blocked_next(&mut node).clone();
        }
        None
    }
}
