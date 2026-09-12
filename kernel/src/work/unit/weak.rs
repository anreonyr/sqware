// 任务弱引用的**出身标注 + 挂起自检**。
//
// 问题的形状：关机偶发报任务外壳（`ArcInner<Task>`，152 B）归还不掉，读数形态是
// `strong 0 weak 1` ——载荷已析构，外壳却因**一枚存活的弱引用**扣着。把全仓的
// `Weak<Task>` 容器逐个点名排除（名册 / 票根 / 躯壳由 `rip` 清空，`Team.tasks` 由
// 团队析构带走）之后，泄漏**仍然复现** ⇒ 持有者不在"我列举出来的容器"里。
//
// 换一条路问内存也不行：**清空但未清零的缓冲里留着陈旧指针字节**，与一枚存活弱引用在
// 字节层面无法区分。只要"存活"这件事只能从字节去猜，就永远分不清"扣着"与"曾经扣过"。
//
// 故本模块把这件事从**内存里**搬到**账上**：仓内一切 `Weak<Task>` 只经 [`TaskWeak`]
// 产生 ⇒ 每一次**生**（构造 / 抄件）与每一次**亡**（析构）各记一笔，生的时候记下
// **出身**（住哪张表，还是只是抄件）与**出生核**。于是本核在**挂起点**上就能问一句
// 总是可判的话：*此刻我栈上还压着抄件吗？*
//
// # 为什么"抄件"这一项非分不可
//
// 内核**不展开栈**（`panic = abort`，任务退场是"离核不返回"，`bury` 直接 `release`
// 掉那段栈 span —— 见 `messenger::reap` 头注）。于是**被弃帧上的 RAII 值永不析构**：
// 一枚活在"退场任务残留帧"里的 `Weak`，其 `Drop` 从此不会执行，弱计数永远挂着。
// 存进容器的那种弱引用随容器清空而死（`rip` 那一刀）；**抄出去临时用的那种**却随帧
// 一起被弃 —— 二者在观测量上长得一模一样（都表现为"外壳归还不掉"），只差一个出身。
// 这就是 [`check_block_heldout`] 每次挂起都要问的那一句：**容器里的不算，抄件算**。
//
// 代价与纪律：全程**原子、无锁**（定长数组 + CAS）。`TaskWeak` 的生/死发生在任意持锁
// 上下文里（名册锁内、团队锁内、站点锁内），这里再加一把锁就是给自己造同层嵌套。

use alloc::sync::Weak;
use core::ops::Deref;
use core::sync::atomic::{AtomicUsize, Ordering::Relaxed};

use super::task::Task;

// ── 出身 ──────────────────────────────────────────────

/// 一枚弱引用的出身：**住哪张表**，还是**只是抄件**。
///
/// 加一个容器就往这里加一项 —— 账要说得出具名，说不出的那一项会把人重新推回
/// "读代码猜"。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Site {
    /// 名册（`ROSTER`：全世界任务的 id → 弱引用；`rip` **最后**清空）
    Roster,
    /// 票根（`HOLDERS`：票 → 持票人；`rip` 清空）
    Holder,
    /// 团队簿记（`Team.tasks`：随团队析构才消失）
    TeamTasks,
    /// 血缘（`Team.sire`：随团队析构才消失）
    Sire,
    /// **抄件**：`muster` 从名册抄出去给调用方临时用（`Join` / `Hatch` 的局部）
    Muster,
    /// **抄件**：快照（`gate::snap` 每次退场钩子抄一份全世界；团队簿记快照同理）
    Snapshot,
    /// 空弱引用（`Weak::new()`：**不占任何分配**，故不记账 —— 见 `counted`）
    Empty,
}

/// 全部出身（下标即 `Site::ix`）。消费者只有 audit 档的挂起自检（它要把槽位里
/// 记下的出身下标还原成 `Site`）。
#[cfg(feature = "audit")]
const ALL: [Site; NSITE] = [
    Site::Roster,
    Site::Holder,
    Site::TeamTasks,
    Site::Sire,
    Site::Muster,
    Site::Snapshot,
    Site::Empty,
];

#[cfg(feature = "audit")]
const NSITE: usize = 7;

impl Site {
    fn ix(self) -> usize {
        match self {
            Site::Roster => 0,
            Site::Holder => 1,
            Site::TeamTasks => 2,
            Site::Sire => 3,
            Site::Muster => 4,
            Site::Snapshot => 5,
            Site::Empty => 6,
        }
    }

    /// 记不记账：空弱引用（`Weak::new()`）**不指向任何 `ArcInner`**，既不扣住谁、
    /// 也就没有"析构没跑"这回事。把它排除在账外，`存 == 0` 才是一条干净的判据
    /// （否则内核团队那枚永生的空 `sire` 会让账恒差 1）。
    fn counted(self) -> bool {
        self != Site::Empty
    }
}

#[cfg(feature = "audit")]
impl Site {
    /// 出身的人话名字（挂起自检的失败消息里点名用；消费者只有那一处）。
    fn name(self) -> &'static str {
        match self {
            Site::Roster => "名册",
            Site::Holder => "票根",
            Site::TeamTasks => "团队簿记",
            Site::Sire => "血亲",
            Site::Muster => "抄件·muster",
            Site::Snapshot => "抄件·快照",
            Site::Empty => "空弱引用",
        }
    }
}

// ── 账 ────────────────────────────────────────────────

/// 存活清单的槽位数。存活弱引用的量级 = 在册任务/团队数（个位到几十）。
const SLOTS: usize = 48;

/// 槽位占用标志（0 = 空）；非 0 即已占。
static SLOT_ID: [AtomicUsize; SLOTS] = [const { AtomicUsize::new(0) }; SLOTS];
/// `出身下标 | hart << 8`（出生处只有这两个小整数，打包进一个字）——挂起自检按
/// "是不是**本核**出生的抄件"筛，故 hart 必须记下来。
static SLOT_META: [AtomicUsize; SLOTS] = [const { AtomicUsize::new(0) }; SLOTS];
/// 槽位身份序列（0 = 无效哨兵，故自 1 起）。
static NEXT_ID: AtomicUsize = AtomicUsize::new(1);

// ── 带账的弱引用 ────────────────────────────────────────

/// 一枚 `Weak<Task>` + 它的出身与槽位 id。
///
/// **`Clone` 有意不实现**：每一次抄件都得在调用点写明"这枚抄件是从哪儿出去的"
/// （`Muster` 还是 `Snapshot`）—— 编译器因此替这条纪律站岗，漏一处就编不过。
#[repr(C)]
pub(crate) struct TaskWeak {
    w: Weak<Task>,
    /// 存活清单里的槽位身份（0 = 没抢到/不计账）。
    id: usize,
    site: Site,
}

impl TaskWeak {
    /// **住进容器**：`site` 说明是哪张表（名册 / 票根 / 团队簿记 / 血亲）。
    pub(crate) fn stored(w: Weak<Task>, site: Site) -> TaskWeak {
        let id = if site.counted() { record(site) } else { 0 };
        TaskWeak { w, id, site }
    }

    /// **抄件**：从容器里抄一枚出去给调用方临时用。`site` 说明抄它的那个点。
    ///
    /// 抄件是"弃帧"泄漏的**唯一**候选形态：容器里的那些随容器清空而死，抄出去的
    /// 那些活在调用方的栈帧里。
    pub(crate) fn copy_at(&self, site: Site) -> TaskWeak {
        TaskWeak::stored(self.w.clone(), site)
    }

    /// 空弱引用（`Weak::new()`：不占分配、不扣任何外壳）。
    pub(crate) fn empty() -> TaskWeak {
        TaskWeak {
            w: Weak::new(),
            id: 0,
            site: Site::Empty,
        }
    }
}

impl Deref for TaskWeak {
    type Target = Weak<Task>;
    fn deref(&self) -> &Weak<Task> {
        &self.w
    }
}

impl Drop for TaskWeak {
    fn drop(&mut self) {
        if self.id != 0 {
            for i in 0..SLOTS {
                if SLOT_ID[i].load(Relaxed) == self.id {
                    // 先撤身份再清元数据：别的读者要么看不到这一格，要么看到完整的一格。
                    SLOT_ID[i].store(0, Relaxed);
                    SLOT_META[i].store(0, Relaxed);
                    return;
                }
            }
        }
    }
}

/// 把出身写进槽位；返回槽位身份（0 = 槽满）。
///
/// 抢槽用 CAS：`SLOT_ID` 为 0 是唯一空态，非 0 即已占。多核同时抢同一格只会有
/// 一个成功（失败者继续扫下一格）。槽满只是"这一枚没记下出身"（挂起自检漏看它），
/// 不影响任何分配语义。
fn record(site: Site) -> usize {
    let id = NEXT_ID.fetch_add(1, Relaxed);
    let meta = site.ix() | (crate::machine::hart_id() << 8);
    for i in 0..SLOTS {
        if SLOT_ID[i]
            .compare_exchange(0, id, Relaxed, Relaxed)
            .is_ok()
        {
            SLOT_META[i].store(meta, Relaxed);
            return id;
        }
    }
    0
}

// ── 挂起自检：跨挂起的"抄件" ────────────────────────────

/// **挂起前自检**：本核此刻还活着的**抄件**有几枚 —— 它们只可能活在**本核当前还压着
/// 的栈帧**里（抄件不落任何容器；已经返回的帧里的抄件早就析构了）。
///
/// 为什么这条判据是"总是可判"的：它不依赖偶发。`block` 每次挂起都问一次，问的是
/// **当下这个核的栈**；正常实现下答案恒为 0（要挂起的任务已把它的强引用交进队列、
/// 把弱引用交进站点）。答案非 0 就说明有一条**引用被留在调用链的局部量里跨过了挂起**
/// —— 而这条链一旦被弃（被别核判死 / 收尾时就地冻住），那个局部量的 `Drop` 永不执行：
/// `Arc` 会把整棵团队/空间钉住，`Weak` 会把 `ArcInner` 外壳钉住（`strong 0 weak 1`）。
///
/// 与 `block` 头注里那条既有纪律（"跨挂起不得持强引用"）是**同一条**，这里把它
/// 从"实现者记得"升级成"每次挂起都自检"，并且把**弱引用**也纳进来：弱引用不钉住
/// 载荷、但钉住外壳，而外壳照样是一笔还不掉的账。
///
/// 判据是**当场断言**（不是记账供关机看）：跨挂起引用是设计上不该出现的形态，
/// 出现即 stop-the-world —— 记账式观测在关机钩子撤掉之后就没有读者了。
///
/// # Panics
///
/// 本核栈上存在出身是抄件的存活弱引用 → panic（fail-fast，crash scene 里能看到
/// `block` 的挂起点与这里的出身）。
#[cfg(feature = "audit")]
pub(crate) fn check_block_heldout() {
    let me = crate::machine::hart_id();
    for i in 0..SLOTS {
        if SLOT_ID[i].load(Relaxed) == 0 {
            continue;
        }
        let meta = SLOT_META[i].load(Relaxed);
        let site = ALL[meta & 0xff];
        // "抄件" = 不落容器的两种出身（`Muster` 抄出即用 / `Snapshot` 快照）。
        // 只算**本核**出生的：别的核栈上的抄件归那次自检管。
        if matches!(site, Site::Muster | Site::Snapshot) && (meta >> 8) == me {
            panic!(
                "[weak] 挂起自检：本核栈上仍有抄件（出身：{}）—— 跨挂起的弱引用会让外壳 \
                 永远归还不掉（`strong 0 weak 1`）",
                site.name()
            );
        }
    }
}
