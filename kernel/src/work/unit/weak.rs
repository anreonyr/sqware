// 任务弱引用的**生/死账** —— `leak: task 1`（`strong 0 weak 1`）的定案仪。
//
// 问题的形状：关机审计偶发报 `[audit] leak: task 1`，读数形态是 `strong 0 weak 1`
// ——载荷已析构，`ArcInner<Task>`（152 B）却因**一枚存活的弱引用**归还不掉。把全仓的
// `Weak<Task>` 容器逐个点名排除（名册 / 票根 / 躯壳 / 站点由 `rip` 清空，`Team.tasks` /
// `Team.sire` 由逐团队普查给出 0）之后，泄漏**仍然复现** ⇒ 持有者不在"我列举出来的
// 容器"里。
//
// 换一条路问内存也不行：**清空但未清零的缓冲里留着陈旧指针字节**，与一枚存活弱引用在
// 字节层面无法区分（本轮已被它骗过一次：命中落在名册的哈希缓冲里，而名册当时报 0 条）。
// 只要"存活"这件事只能从字节去猜，就永远分不清"扣着"与"曾经扣过"。
//
// 故本模块把这件事从**内存里**搬到**账上**：仓内一切 `Weak<Task>` 只经本类型产生
// ⇒ 每一次**生**（构造 / 抄件）与每一次**亡**（析构）各记一笔，生的时候再把**出身**
// （哪个点、哪个核、当时的内核栈指针）写进定长槽位。于是判据变成一条纯算术：
//
//     存 = 生 − 亡 == 0        （没有一枚弱引用对象被丢在"析构不会跑"的地方）
//
// 而没有归零的那一枚，其出身把"谁扣着"说成**具名**：出身是容器（名册 / 票根 / 团队
// 簿记 / 血亲）还是**抄件**（`muster` 出的临时弱引用 / 快照）。
//
// # 为什么"抄件"这一项非分不可
//
// 内核**不展开栈**（`panic = abort`，任务退场是"离核不返回"，`bury` 直接 `release`
// 掉那段栈 span —— 见 `messenger::reap` 头注）。于是**被弃帧上的 RAII 值永不析构**：
// 一枚活在"退场任务残留帧"里的 `Weak`，其 `Drop` 从此不会执行，弱计数永远挂着。
// 存进容器的那种弱引用随容器清空而死（`rip` 那一刀）；**抄出去临时用的那种**却随帧
// 一起被弃 —— 二者在观测量上长得一模一样（都表现为"外壳归还不掉"），只差一个出身。
// 故槽位里连**出生时的栈指针**都记下来：关机时若那一帧已经不在手（栈已归还），
// 就是**弃帧**，一行字定案。
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

/// 全部出身（下标即 `Site::ix`）。
const ALL: [Site; NSITE] = [
    Site::Roster,
    Site::Holder,
    Site::TeamTasks,
    Site::Sire,
    Site::Muster,
    Site::Snapshot,
    Site::Empty,
];

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

    #[cfg_attr(not(feature = "audit"), allow(dead_code))]
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

    /// 记不记账：空弱引用（`Weak::new()`）**不指向任何 `ArcInner`**，既不扣住谁、
    /// 也就没有"析构没跑"这回事。把它排除在账外，`存 == 0` 才是一条干净的判据
    /// （否则内核团队那枚永生的空 `sire` 会让账恒差 1）。
    fn counted(self) -> bool {
        self != Site::Empty
    }
}

// ── 账 ────────────────────────────────────────────────

static BORN: [AtomicUsize; NSITE] = [const { AtomicUsize::new(0) }; NSITE];
static DIED: [AtomicUsize; NSITE] = [const { AtomicUsize::new(0) }; NSITE];

/// 存活清单的槽位数。存活弱引用的量级 = 在册任务/团队数（个位到几十），
/// 槽满只是"出身未记"（计数仍然准），不影响判据。
const SLOTS: usize = 48;

static SLOT_ID: [AtomicUsize; SLOTS] = [const { AtomicUsize::new(0) }; SLOTS];
/// `出身下标 | hart << 8`（出生处只有这两个小整数，打包进一个字）。
static SLOT_META: [AtomicUsize; SLOTS] = [const { AtomicUsize::new(0) }; SLOTS];
/// 出生时的**内核栈指针**（记录出身现场：哪颗核的哪一趟调用造的）。
static SLOT_SP: [AtomicUsize; SLOTS] = [const { AtomicUsize::new(0) }; SLOTS];
/// 出生那一刻目标**还活着吗**（`Weak::strong_count`，纯 load）。关机时读它出判词：
/// 0 ⇒ 载荷已析构而弱计数还挂着 = **扣着外壳的那一枚**。
static SLOT_ALIVE: [AtomicUsize; SLOTS] = [const { AtomicUsize::new(0) }; SLOTS];
/// 抢不到槽位的枚数（只影响"出身"，不影响生/亡计数）。
static SLOT_LOST: AtomicUsize = AtomicUsize::new(0);
/// 槽位身份序列（0 = 无效哨兵，故自 1 起）。
static NEXT_ID: AtomicUsize = AtomicUsize::new(1);

/// 出生时的栈指针（`sp`）。**只读出栈上地址**，不改任何状态。
fn sp() -> usize {
    let sp: usize;
    // SAFETY: 只读 `sp` 寄存器到一个通用寄存器，无内存访问（nomem）、不动栈
    // （nostack）、不改标志位。
    unsafe {
        core::arch::asm!("mv {}, sp", out(reg) sp, options(nomem, nostack, preserves_flags));
    }
    sp
}

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
        let id = if site.counted() {
            record(site, w.strong_count())
        } else {
            0
        };
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
        if self.site.counted() {
            DIED[self.site.ix()].fetch_add(1, Relaxed);
        }
        if self.id != 0 {
            for i in 0..SLOTS {
                if SLOT_ID[i].load(Relaxed) == self.id {
                    // 先撤身份再清元数据：别的读者要么看不到这一格，要么看到完整的一格。
                    SLOT_ID[i].store(0, Relaxed);
                    SLOT_SP[i].store(0, Relaxed);
                    SLOT_META[i].store(0, Relaxed);
                    SLOT_ALIVE[i].store(0, Relaxed);
                    return;
                }
            }
        }
    }
}

/// 记一笔"生"，并把出身写进槽位；返回槽位身份（0 = 槽满）。
///
/// 抢槽用 CAS：`SLOT_ID` 为 0 是唯一空态，非 0 即已占。多核同时抢同一格只会有
/// 一个成功（失败者继续扫下一格）。
fn record(site: Site, alive: usize) -> usize {
    BORN[site.ix()].fetch_add(1, Relaxed);
    let id = NEXT_ID.fetch_add(1, Relaxed);
    let meta = site.ix() | (crate::machine::hart_id() << 8);
    for i in 0..SLOTS {
        if SLOT_ID[i]
            .compare_exchange(0, id, Relaxed, Relaxed)
            .is_ok()
        {
            SLOT_META[i].store(meta, Relaxed);
            SLOT_SP[i].store(sp(), Relaxed);
            SLOT_ALIVE[i].store(alive, Relaxed);
            return id;
        }
    }
    SLOT_LOST.fetch_add(1, Relaxed);
    0
}

// ── 挂起自检：跨挂起的"抄件" ────────────────────────────

/// 挂起点上**还活着的"抄件"**枚数（累计，>0 即违例）。
static HELD_OUT: AtomicUsize = AtomicUsize::new(0);
/// 头几次违例的出身（打印用；上限 4，避免把挂起热路径变成刷屏源）。
static HELD_OUT_SITE: [AtomicUsize; 4] = [const { AtomicUsize::new(0) }; 4];

/// **挂起前自检**：本核此刻还活着、且出身是**抄件**的弱引用有几枚 —— 它们只可能活在
/// **本核当前还压着的栈帧**里（抄件不落任何容器；已经返回的帧里的抄件早就析构了）。
///
/// 为什么这条判据是"总是可判"的：它不依赖偶发。`block` 每次挂起都问一次，问的是
/// **当下这个核的栈**；正常实现下答案恒为 0（要挂起的任务已把它的强引用交进队列、
/// 把弱引用交进站点）。答案非 0 就说明有一条**引用被留在调用链的局部量里跨过了挂起**
/// —— 而这条链一旦被弃（被别核判死 / 收尾时就地冻住），那个局部量的 `Drop` 永不执行：
/// `Arc` 会把整棵团队/空间钉住，`Weak` 会把 `ArcInner` 外壳钉住（`leak: task 1`，
/// `strong 0 weak 1` —— 关机审计那条的由来）。
///
/// 与 `block` 头注里那条既有纪律（"跨挂起不得持强引用"）是**同一条**，这里把它
/// 从"实现者记得"升级成"每次挂起都自检"，并且把**弱引用**也纳进来：弱引用不钉住
/// 载荷、但钉住外壳，而外壳照样是一笔还不掉的账。
#[cfg(feature = "audit")]
pub(crate) fn check_block_heldout() {
    let me = crate::machine::hart_id();
    let mut n = 0usize;
    for i in 0..SLOTS {
        if SLOT_ID[i].load(Relaxed) == 0 {
            continue;
        }
        let meta = SLOT_META[i].load(Relaxed);
        let site = ALL[meta & 0xff];
        // "抄件" = 不落容器的两种出身（`Muster` 抄出即用 / `Snapshot` 快照）。
        if !matches!(site, Site::Muster | Site::Snapshot) || (meta >> 8) != me {
            continue;
        }
        n += 1;
        let k = HELD_OUT.fetch_add(1, Relaxed);
        let _ = k;
        if n <= 4 {
            let slot = HELD_OUT_SITE.iter().position(|x| x.load(Relaxed) == 0);
            if let Some(slot) = slot {
                HELD_OUT_SITE[slot].store(site.ix() + 1, Relaxed);
            }
        }
    }
}

/// 挂起自检的关机判词（`report` 里打）。
#[cfg(feature = "audit")]
fn held_out_line() {
    let n = HELD_OUT.load(Relaxed);
    if n == 0 {
        crate::putln!("[audit] 挂起自检：每次挂起时本核栈上都没有抄件（跨挂起的引用 = 0）");
        return;
    }
    let mut who = alloc::string::String::new();
    for x in HELD_OUT_SITE.iter() {
        let v = x.load(Relaxed);
        if v != 0 {
            who.push_str(ALL[v - 1].name());
            who.push(' ');
        }
    }
    crate::putln!(
        "[audit] 挂起自检：**{n} 次挂起时本核栈上仍有抄件**（出身：{who}）—— \
         这些引用跨过了挂起，帧被弃则计数永不回落（`leak: task 1` 的由来）"
    );
}

// ── 观测面（只读；audit 档） ─────────────────────────────

/// 打印生/亡账与存活清单。**只在关机钩子里调**（那一刻没有别的东西在动，读到的
/// 是静止画面）。
///
/// 两行判词：
/// - `存 == 0` ⇒ 每一枚弱引用的析构都跑到了，"外壳归还不掉"不可能来自弱引用对象。
/// - 存活清单里某一枚的出生帧**已不在手** ⇒ 那一枚被弃在**已经归还的栈**上（内核不
///   展开栈，故它的析构从此不会执行）—— 这就是 `leak: task 1` 的持有者。
#[cfg(feature = "audit")]
pub(crate) fn report() {
    let mut born = 0usize;
    let mut died = 0usize;
    let mut per = alloc::string::String::new();
    for s in ALL {
        let b = BORN[s.ix()].load(Relaxed);
        let d = DIED[s.ix()].load(Relaxed);
        born += b;
        died += d;
        if s.counted() {
            per.push_str(&alloc::format!("{} {b}/{d} ｜", s.name()));
        }
    }
    crate::putln!(
        "[audit] 弱引用收支：生 {born} 亡 {died} **存 {}**（存≠0 ⇒ 有弱引用对象的析构没跑）\
         ｜逐点 生/亡：{per}",
        born - died
    );

    let mut n = 0usize;
    for i in 0..SLOTS {
        let id = SLOT_ID[i].load(Relaxed);
        if id == 0 {
            continue;
        }
        n += 1;
        let meta = SLOT_META[i].load(Relaxed);
        let hart = meta >> 8;
        let site = ALL[meta & 0xff];
        let at = SLOT_SP[i].load(Relaxed);
        // **判词**：这枚弱引用还指着活对象吗？
        //   `strong == 0` ⇒ 载荷已析构、弱计数却还挂着 —— **正是它扣着那 152 B 外壳**
        //   （`leak: task 1` 的持有者本人）；`strong > 0` ⇒ 目标还活着，那一块算在别处，
        //   本枚不是元凶。判据只做一次 `strong_count`（纯 load、不升强引用——观测不许
        //   改变被观测的事实）。
        let strong = SLOT_ALIVE[i].load(Relaxed);
        crate::putln!(
            "[audit]   存活 #{n} id={id} 出身={} hart={hart} sp={at:#x} 指着 strong={strong}{}",
            site.name(),
            if strong == 0 {
                " ← **就是它扣着外壳**（载荷已析构、弱计数未归零）"
            } else {
                ""
            }
        );
        // 它**现在躺在哪段内存里**：在册块（说出了谁分配的那张表）／不在账上
        // （那段内存已被拆掉——比泄漏更严重）／整个找不到（已被覆写）。
        // 只在存活清单非空时跑（一次线性扫），健康轮一分钱不花。
        use crate::memory::allocator::fence::audit::Where;
        match crate::memory::allocator::fence::audit::locate_weak(id, site.ix()) {
            Some((p, Where::Ledger(base, size, kind, s))) => crate::putln!(
                "[audit]     物件 @ {p:#x} 躺在**在册**块里：{kind:?} base={base:#x} size={size} 偏移={} \
                 site={s:#x}（内存还活着 ⇒ 只是没人去析构它）",
                p - base
            ),
            Some((p, Where::TrapStack(h))) => crate::putln!(
                "[audit]     物件 @ {p:#x} 躺在 hart {h} 的 **trap 栈**上（一段恒映射的常驻栈）"
            ),
            Some((p, Where::Image)) => crate::putln!(
                "[audit]     物件 @ {p:#x} 躺在**内核镜像**里（静态 / BSS / ROOT 栈）"
            ),
            Some((p, Where::PoolUnaccounted)) => crate::putln!(
                "[audit]     物件 @ {p:#x} **不在任何在册块里** ⇒ 它所在的那段内存已被释放/复用 \
                 （活着就被拆掉：析构永不执行）"
            ),
            None => crate::putln!(
                "[audit]     物件**已不在任何可读内存里** ⇒ 所在分配已被拆掉并覆写"
            ),
        }
    }
    if n == 0 {
        crate::putln!("[audit]   存活清单：空（每一枚弱引用的析构都跑到了）");
    }
    let lost = SLOT_LOST.load(Relaxed);
    if lost > 0 {
        crate::putln!("[audit]   槽位不足：{lost} 枚没记下出身（生/亡计数不受影响）");
    }
    held_out_line();
}
