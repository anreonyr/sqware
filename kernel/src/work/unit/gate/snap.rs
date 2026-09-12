// snap — 全世界任务快照 + 沿 sire 的查询面。
//
// 能力面**只存一条边**（`Pie.sire`）。另两个方向都靠查询还原：
//   - `vestor(pie, snap)` —— 向上：父门闩的持有者（= 授与人）
//   - `heirs(token, snap)` —— 向下：`sire` 指向我的那些子门闩
//   - `vestable(pie, dst, snap)` —— BACK 守门
//   - `find(tid, snap)` —— 按 id 取任务（退出钩子用）
//
// **快照由适配层提供**（boot 注入 provider）：本模块不依赖 scheduler。快照是
// `Weak<Task>` 列表——死条目升级失败即跳过，故查询无副作用、无需清理。
//
// 锁序：本模块逐任务取放 `Task.pies`（L3），**绝不嵌套**——调用方不得持 L3 调用。

use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;

use crate::lock::OnceLock;
use crate::work::unit::task::Task;
use crate::work::unit::weak::TaskWeak;

use super::pie::{AnyPie, Permission};

/// 全世界任务快照：查询与级联的唯一输入。只读、不增删。
///
/// 快照条目是**抄件**（`Site::Snapshot`）：容器里那些随容器清空而死，抄出来的
/// 这些却活在调用方的栈帧里 —— 二者在"外壳归还不掉"这个观测量上长得一样，只差
/// 一个出身（见 `work::unit::weak` 头注）。
pub(crate) type Snap = [TaskWeak];

/// 快照提供者（boot 注入；`gate` 不依赖 scheduler）。
fn provider() -> &'static OnceLock<fn() -> Vec<TaskWeak>> {
    static P: OnceLock<fn() -> Vec<TaskWeak>> = OnceLock::new();
    &P
}

/// 注入快照提供者（boot 一次性调用）。
pub(crate) fn install(f: fn() -> Vec<TaskWeak>) {
    let _ = provider().set(f);
}

/// 拍一张快照。未注入 → 空（查询退化为「找不到」，即不级联、不认亲）。
pub(crate) fn snap() -> Vec<TaskWeak> {
    provider().get().copied().map(|f| f()).unwrap_or_default()
}

/// 按 task id 取任务（升级 Weak；不在 → None）。
///
/// # 走坏指针不得 panic（本函数在退场钩子里，没有错误通道）
///
/// 调用者 [`cull::doom`](super::cull::doom) 是 `Hook = fn(usize)`：**每个任务退场
/// 都跑一次**，签名里没有返回值、没有 `Result`。所以这里任何 panic 都等于
/// **一个任务的退场把整机带走**。
///
/// 而 `Weak::upgrade` 会**无条件**解引用 `self.ptr`——只要快照里混进一条坏指针，
/// 页错误就发生在内核态，落到 `trap.rs` 的「内核自身缺页 = 内核 bug → panic」。
///
/// 实测（64M，`churn` 约 2260–4000 轮；`stval` 每轮不同：`0x1078` / `0x0` /
/// `0x8`）：`sepc` 一次次指在 `snap::find` 的升级处，**坏指针来自快照**。那种
/// "故障地址每次都不一样"的形状是**内存被写坏**，不是逻辑分支——不是本函数能
/// 修的东西（根源在别处）。
///
/// 故本函数只做一件事：**核对再升级**。指针明显非法（空 / 低位地址）⇒ 跳过该条
/// 并记一笔，**不 deref**。找不到 = "不级联"，与 [`snap`] 头注里「空快照 ⇒
/// 不认亲、不级联」同一条语义；而拿坏指针换一次整机 halt 不是任何语义。
///
/// 边界要说清：本核对只挡**明显非法**的指针，挡不住"指向已释放/被覆写但地址
/// 合法"的那种——那种要靠修根因。它的价值是把"整机死"降级成"少级联一次 + 留证据"。
pub(crate) fn find(tid: usize, snap: &Snap) -> Option<Arc<Task>> {
    for w in snap {
        if !plausible(w) {
            static BAD: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
            let n = BAD.fetch_add(1, core::sync::atomic::Ordering::Relaxed) + 1;
            if n <= 8 {
                crate::putln!("snap::find: skip implausible weak ({n}) tid={tid}");
            }
            continue;
        }
        if let Some(t) = w.upgrade()
            && t.ident.id == tid
        {
            return Some(t);
        }
    }
    None
}

/// 弱引用指针是否**可能**合法：非空、且不在低地址页（实测坏值是 `0x8` /
/// `0x1078` 这类，都是"近空指针 + 字段偏移"）。
///
/// 这是**粗筛**：只用于"别拿它去 deref"，不承担"它一定活着"的断言。
fn plausible(w: &Weak<Task>) -> bool {
    (Weak::as_ptr(w) as usize) >= 0x1000
}

/// 向上：这枚门闩的授与人（= 父门闩所在任务的 id）。原始自持 → None。
pub(crate) fn vestor(pie: &AnyPie, snap: &Snap) -> Option<usize> {
    holder(pie.sire()?, snap)
}

/// 某枚门闩（按 token）的持有者：快照里谁的表里有它。
fn holder(token: usize, snap: &Snap) -> Option<usize> {
    for w in snap {
        let Some(t) = w.upgrade() else { continue };
        // 显式作用域：guard 必须在取 id 之前释放（不跨表）。
        let found = {
            let pies = t.pies.lock();
            pies.iter().any(|p| p.token() == token)
        };
        if found {
            return Some(t.ident.id);
        }
    }
    None
}

/// 向下：`sire == token` 的全部子门闩（持有者 + 子 token）。
///
/// **返回 `None` = 容量备不出来 ⇒ 视为没有后代**（放弃级联）。调用方是 `cull`
/// 的 BFS，它在**退场钩子里**跑（`Hook = fn(usize)`，无错误通道）：这里的
/// 不可失败分配等于"一个任务退场时堆一紧 ⇒ 整机 halt"。
///
/// 逐个 `w` 备容量、而不是先扫一遍数总数：总数要再扫一次表，而这里**在持锁
/// 迭代**——两次读之间表可能变，数出来的总数不保证够。每次 `out.try_reserve(1)`
/// 在容量够时是纯比较，够快；不够时才真去扩，失败即放弃。
pub(crate) fn heirs(token: usize, snap: &Snap) -> Option<Vec<(Arc<Task>, usize)>> {
    let mut out: Vec<(Arc<Task>, usize)> = Vec::new();
    for w in snap {
        let Some(t) = w.upgrade() else { continue };
        // 子 token 先落本地：`try_reserve` 用得上，且避免在持 `pies` 锁时扩 `out`。
        let mut kids: Vec<usize> = Vec::new();
        {
            let pies = t.pies.lock();
            if kids.try_reserve(pies.len()).is_err() {
                return None;
            }
            kids.extend(pies.iter().filter(|p| p.sire() == Some(token)).map(|p| p.token()));
        }
        for k in kids {
            if out.try_reserve(1).is_err() {
                return None;
            }
            out.push((t.clone(), k));
        }
    }
    Some(out)
}

/// BACK 守门：带 BACK 的源只能授给 `sire` 的持有者；不带 BACK 恒真；
/// 原始自持（sire = None）带 BACK 不受限（回授目标自由）。
pub(crate) fn vestable(pie: &AnyPie, dst: usize, snap: &Snap) -> bool {
    if !pie.permission().contains(Permission::BACK) {
        return true;
    }
    match pie.sire() {
        None => true,
        Some(sire) => holder(sire, snap) == Some(dst),
    }
}
