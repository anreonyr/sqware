//! pile — **Tole 的 runtime 封装**：把几枚**可等地**（孔的一个方向 / 一枚铃）挂到一处，
//! 等其中**任意一格**有事。
//!
//! 与 `core/bell.rs` 同一分工：门铃是"一枚无载荷信号怎么用"，这里是"多路等待怎么用"
//! ——薄封装 + 让调用的形状像一句话；envcall 转发在 `env/mail.rs`（class 9 与 class 5
//! 同住那一份通信面）。
//!
//! 为什么要有这一层：`await` 的返回是"哪一枚"（一枚号 + 一个方向），而**挂起过**的
//! 那一次读回来的是预置值 `PieToken::NONE`（内核没有第二次执行机会）。把这条契约翻成
//! `Option`（`None` = 这一轮没等到），调用方就不必自己认哨兵——但也**必须**按 deadline
//! 循环，否则 `None` 会被误当成"永远没有"。
//!
//! 组上装了[状态订阅](Sub)之后，`None` **多一义**："有来源报过事，去复核"。
//! 两义共用一个形状是有意的：通知只要求复核，从不代替判据——调用方无论如何都要
//! 重新读一遍实际状态。

use env::Wait;
use env::{HoleDir, PieToken, Source, TaskId, ToleResult};

use crate::env::mail::{Mate, TolePie};

/// **一条状态订阅的描述**——两格的**名字与内核 `work::mail::tole::Sub` 逐字相同**。
///
/// 它同时是"订阅"与"取消"的凭据：本层**不发 token**，取消就凭这同一条描述。
///
/// **第二格不带载荷**（内核那一格带 `TaskId`）：本层的目标是**自己**，由
/// [`crate::env::unit::self_id`] 就地取——调用方因此**表达不出**"观察别人的能力表"。
/// 名字对齐、载荷故意缺一格，就是这条类型义务本身。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sub {
    /// 观察某个任务的**退出收尾完成**。
    TaskCompleted(TaskId),
    /// 观察**自己**能力的可观察状态改变。
    Capabilities,
}

impl Sub {
    fn wire(self) -> (Source, TaskId) {
        match self {
            Sub::TaskCompleted(id) => (Source::TaskCompleted, id),
            Sub::Capabilities => (
                Source::CapabilitiesChanged,
                crate::env::unit::self_id(),
            ),
        }
    }
}

/// 一个组的使用面。
pub struct Pile {
    pie: TolePie,
}

impl Pile {
    /// 造一个空组。
    ///
    /// `shared` = 允不许多个使用者（造的时候定、之后不可变）：`false` = 独占组（授出即
    /// 移交、复制不出来），`true` = 共享组（可交给多个任务各持一枚；组键的唤醒是**提示
    /// 型**——放行全链，人人醒来自己按组复核）。
    pub fn unseal(shared: bool) -> ToleResult<Pile> {
        Ok(Pile {
            pie: TolePie::unseal(shared)?,
        })
    }

    /// 收下一枚已经在对端的组（调用方递过来的 token）。
    pub fn new(pie: TolePie) -> Pile {
        Pile { pie }
    }

    /// 把一枚成员的一个方向挂进来（同成员幂等）。
    pub fn attach<M: Mate>(&self, mate: &M, dir: HoleDir) -> ToleResult<()> {
        self.pie.attach(mate, dir)
    }

    /// 摘掉一格；没挂过即无事。
    pub fn detach<M: Mate>(&self, mate: &M, dir: HoleDir) -> ToleResult<()> {
        self.pie.detach(mate, dir)
    }

    /// 把一个状态来源登记进组（同描述幂等）；成功即留一次待复核提示。
    pub fn subscribe(&self, sub: Sub) -> ToleResult<()> {
        let (source, target) = sub.wire();
        self.pie.subscribe(source, target)
    }

    /// 按描述取消（同描述重复取消无事）；**不要求目标还在世**。
    pub fn unsubscribe(&self, sub: Sub) -> ToleResult<()> {
        let (source, target) = sub.wire();
        self.pie.unsubscribe(source, target)
    }

    /// 等到任意一格有事：`Some((哪一枚, 哪个方向))`；`None` = 这一轮没等到
    /// （挂起过，或期限到）——**继续等就再叫一次**，别把 `None` 当成终局。
    pub fn await_(&self, millis: Wait) -> ToleResult<Option<(PieToken, HoleDir)>> {
        let (token, dir) = self.pie.await_(millis)?;
        Ok((token != PieToken::NONE).then_some((token, dir)))
    }

    pub fn token(&self) -> PieToken {
        self.pie.token()
    }
}
