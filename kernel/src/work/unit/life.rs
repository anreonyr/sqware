// 存活单元（life）——「键的寿命 = 资源的寿命」这条不变式的载体（A2 的后半）。
//
// 问题的形状：站点表（`work::room::messenger::wait::site`）里的站点此前**永不回收**
// ——键退役只能靠 `wipe` 留下的**墓碑**（`pend = true` 的空站点）告诉后来者「此键
// 已死」。于是每次 hole 封印、每次任务回收各留一个墓碑，而 hole id / task id 都
// 单调 ⇒ 站点表随运行单调增长。墓碑不是手滑，它是「在飞窗口」的同步点：一个正
// 飞在 `block` ①④之间的等待者，此刻站点表里可能还没有它的站点，`wipe` 不建墓碑
// 就没人接得住它。要收回墓碑只有两条路：① 观测「在飞窗口已关闭」（今天没有这个
// 量）；② **把「键已死」从墓碑搬到键/资源本身**——于是站点根本不必留。
//
// 本模块就是②：`Life` 是键背后那份资源的存活单元，**由资源的创建者持有强引用**
// （`Arc<Life>`），站点值只留一枚 `Weak<Life>`。死亡 = 强引用归零 ⇒ 站点侧
// `upgrade` 失败，**观察**得到。没有任何写路径、没有回调、没有第二张表。
//
// **为什么不需要锁**：`Life` 是**无字段标记类型**（零尺寸；`Arc<Life>` 只在
// `ArcInner` 头里有计数），故它自己**没有锁可持**——不可能是 L1/L2/L3 任何一层的
// 持有者，也就不可能参与任何锁序（连「不小心嵌套」的机会都没有）。判定「死没死」
// 只读 `ArcInner::strong` 这一个原子量（`Weak::upgrade` 的 CAS），原子读写在任何层
// 都能做，故站点锁（L3）内判死是合法的——这正是 `prune` 需要的那一条读法。
//
// **为什么不靠 `Drop` 回调**：(a) 案想在 `Drop` 里通知 room「这个键死了」，但
// `Drop` 可能发生在任意持锁上下文里（`Task.pies` 锁内 drop 门闩是既有路径），
// 那个回调一取 L3 就是自嵌。观察式判死把这个前提整个消掉。
//
// 自洽性前提（A2 裁决的硬条件）：除 `Life::new` 与资源侧**显式的** `wipe(key)`
// 外，room 不接受任何来自外部的「这个键死了」的说法——全部靠**读**。故本模块
// 只有一个构造函数、一对读法，没有任何 setter / 通知面。

use alloc::sync::{Arc, Weak};

/// 键的存活单元：**无字段标记类型**——死亡就是强引用归零。
///
/// 不变式：**它只回答「资源还在不在」**，不回答「谁在等」——后者仍归站点表。
/// 故一个键只有一份 `Life`（其唯一强持有者就是那份资源），站点值里的
/// `Weak<Life>` 不管由哪个入口写入，指向的都是同一个分配。
///
/// 零尺寸是刻意的：`Arc<Life>` 不占数据区，`Weak<Life>` 是一枚裸指针。
/// `Debug` 是为带 `#[derive(Debug)]` 的资源（`Space`）而实现——没有字段可打，
/// 故打成一个固定记号：它**不承载身份**，承载身份的是那条 `Arc`。
#[derive(Debug)]
pub(crate) struct Life;

impl Life {
    /// 新存活单元。**只由资源的创建者调用**（`Space` / `HoleMeta` / 任务各一次）。
    pub(crate) fn new() -> Arc<Life> {
        Arc::new(Life)
    }

    /// 可失败的 [`Self::new`]：内存吃紧时返回 `Err`，而不是走 std 默认的
    /// `handle_alloc_error`（那会 panic → 整机 halt）。任务装配路径用这个，
    /// 好让「生不出任务」表现为一个返回码，而不是全机陪葬。
    #[allow(dead_code)]
    pub(crate) fn try_new() -> Result<Arc<Life>, crate::memory::manager::MapError> {
        Arc::try_new_in(Life, alloc::alloc::Global)
            .map_err(|_| crate::memory::manager::MapError::OutOfMemory)
    }

    /// 活着吗：`Weak` 还能升级出强引用。资源侧的对外读法。
    pub(crate) fn live(w: &Weak<Life>) -> bool {
        w.strong_count() > 0
    }

    /// 死了吗：读法对偶，room 内部只用这一个（站点锁内判死）。
    pub(crate) fn dead(w: &Weak<Life>) -> bool {
        !Self::live(w)
    }
}

/// 任务键的存活单元：**键 + 任务自己的 `Life` 弱引用**一起交出去。
///
/// 为什么要在调度侧就配对：「键 → 存活单元」的解析在**调用方那一层**
/// （envcall / mail / scheduler），**不在 room**——room 不许认识 mail，也不该去
/// 查任务注册表。而 `WakeKey::Task { id }` 的解析点只有 `UnitCall::Join` 的入口，
/// 那里本来就握着目标的 `Arc<Task>`（授权判定要用）。
///
/// 命名：`Weak` 在值里而非键里（键是 map key，必须 `Copy`/`Eq`），故本类型是
/// 「一对」，不是键的一部分。
pub(crate) struct TaskLife {
    pub(crate) id: usize,
    pub(crate) life: Weak<Life>,
}
