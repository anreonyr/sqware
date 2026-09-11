// Void — 无载荷的权柄载体。
//
// 与 Hole / Pole 并列的第三种数据面**类型**，也是三者中唯一**没有数据面**的：
// 它不携带消息（那是 Hole），也不借映页（那是 Pole）。全部内容就是"存在"这个事实
// ——故它的 meta 比另两者少掉整个载荷部分，只剩身份与存活。
//
// 用途：表达**存在权**——即"你能不能做某件事"，与"你对某份资源能做什么"无关。
// 第一位消费者是建域权（`UnitCall::Build`）。
//
// 为什么不是 Permission 的一位：位说"对这份资源能做什么"，与资源同轴；而存在权
// 不属于任何资源。做成位会让**任何**资源顺带携带它（`READ|WRITE|BUILD` 这种掩码
// 一旦能出现，"这是不是那枚"就再也答不出来）。类型是身份，位不是。
//
// 为什么不是"没数据的 Hole"：那样权威长得跟普通资源一样，内核又只能靠标记去认它
// ——回到位那条路。Void **因为空，所以不可能被当成资源来使唤**。
//
// 边界（定义式）：Void 只承载**无状态**的存在权。带状态的许可（配额"最多 N 个"、
// 设备能力"哪个窗口"）不该是 Void——那种要另立 meta，因为"多少/哪个"是数据面的活。

use alloc::sync::Arc;

use crate::lock::{Level, SpinLock};

/// Void 状态。与 Hole/Pole 同词：`Seal` 之后恒 `Dead`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoidState {
    Live,
    Dead,
}

/// Void 数据面实体（Arc 持有；**无载荷**——没有 mtu、没有槽、没有映射表）。
pub struct VoidMeta {
    state: SpinLock<VoidState>,
    /// 开辟者：`UnsealVoid` 时的任务 id（构造期定型，无 setter）。0 = 内核自建。
    /// 语义同 `HoleMeta::owner`：`vestor` 管门闩的来历，`owner` 管资源的来历。
    owner: usize,
    // **没有 id**：另两者的 id 是**等待键的身份**（谁在等这条孔 / 这份页），而 Void
    // 没有等待者——没人会等一个没有数据的东西，故不需要它。少一个字段是"无数据面"
    // 的直接后果，不是省事。
}

impl VoidMeta {
    /// 造一枚 Void。**无参数**——没有大小、没有对齐、没有上限可校验，这正是它
    /// 与 `hole::meta(mtu, owner)` / `pole::allocate(bytes, owner)` 的区别。
    pub(crate) fn new(owner: usize) -> Arc<Self> {
        Arc::new(Self {
            state: SpinLock::new_level(Level::L3, VoidState::Live),
            owner,
        })
    }

    /// 资源开辟者（见字段 `owner`）。
    pub(crate) fn owner(&self) -> usize {
        self.owner
    }

    /// 资源可用（已封印 → false）。
    pub(crate) fn alive(&self) -> bool {
        matches!(*self.state.lock(), VoidState::Live)
    }
}

/// 封印 Void：置死。**不回收内存**——资源寿命由引用计数决定（同 Hole/Pole）。
///
/// 没有"唤醒等待者"这一步：没有人会等一个没有数据的东西（Hole 的 `seal` 要
/// `wipe` 两个方向的等待键，Void 一个方向都没有）。
pub(crate) fn seal(meta: &VoidMeta) {
    *meta.state.lock() = VoidState::Dead;
}
