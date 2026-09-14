//! Bell — **Nole 的 runtime 封装**：把一枚 Nole 当门铃用（`docs/bell.md`）。
//!
//! 内核里它**仍是一枚 Nole**（`AnyPie::Nole`，没有第四种资源）：Nole 上多了"听者面"
//! ——`id` / `life` / `ring` 一位——而"怎么用"封装在这一层。与 `Channel` 包着
//! `HolePie` 同构：**种类归内核，用法归 runtime**。
//!
//! 三个动词，与 Hole 那一族同形：
//!
//! ```text
//! ring   响     置"有待取之事"（内核在 trap 上下文响；用户只在自检里响自己那枚）
//! wait   等     等铃响（有界/无界）——**不清**
//! hush   应     清掉"有待取之事"，内核随即重开本 hart 的中断闸门
//! ```
//!
//! # 为什么 `wait` 不清、要显式 `hush`
//!
//! 内核那一位同时就是中断闸门的账（响着 ⇒ 本 hart 的 SEIE 关着）。清必须与"取完"
//! 同一刻：本域醒来之后还要 claim / 投递 / complete，那些做完才轮得到"没有待取之事"。
//! 故 `wait` 只回答"响了没有"，`hush` 才是"我取走了"——与旧孔时代 `wait` + `pull`
//! 的两拍同形。
//!
//! # 为什么 `wait` 没有方向参数
//!
//! 铃只有一条方向（有事/没事）。签名少一个参数就把这件事说完了，不必写注释解释
//! "为什么只有 Pull"。

use env::{EnvResult, HoleDir, TaskId};

use crate::env::mail::{self, AnyPie as _, NolePie};

/// 门铃：一枚 Nole + "怎么用它"。
pub struct Bell {
    pie: NolePie,
}

impl Bell {
    /// 收下一枚已经在对端的 Nole 门闩（`Pier` 递过来的 token）。
    pub fn new(pie: NolePie) -> Bell {
        Bell { pie }
    }

    /// 等铃响：`millis` 毫秒（`usize::MAX` = 永久，`0` = 只探测不挂起）。
    ///
    /// 返回 `true` = 本次调用**当场就绪**（未挂起）；`false` = 未就绪（挂起过、或超时
    /// ——两者不分）。**不清**那一位，见模块头。
    pub fn wait(&self, millis: usize) -> EnvResult<bool> {
        mail::wait(self.pie.token(), HoleDir::Pull, millis)
    }

    /// 应铃：清掉"有待取之事"。未响返 `Busy`（没有可取之事）。
    pub fn hush(&self) -> EnvResult<()> {
        mail::hush(self.pie.token())
    }

    /// 自响：置"有待取之事"并唤醒听者。已响返 `Busy`。
    ///
    /// 生产路径上只有**自检**用它（`irq` 那道门铃由内核响，不走门闩）；它也是
    /// "自己叫自己"的正当写法——响者由持铃者决定。
    pub fn ring(&self) -> EnvResult<()> {
        mail::ring(self.pie.token())
    }

    /// 授出一份（走权柄轴；`AnyPie` 那一套对三种资源同形）。
    pub fn accord(&self, dst: TaskId, subset: env::Permission) -> EnvResult<env::PieToken> {
        self.pie.accord(dst, subset)
    }
}
