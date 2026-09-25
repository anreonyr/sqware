//! Bell — **Nole 的 runtime 封装**：把一枚 Nole 当门铃用。
//!
//! 内核里它**仍是一枚 Nole**（`AnyPie::Nole`，没有第四种资源）：Nole 上多了"听者面"
//! ——`id` / `life` / `ring` 一位——而"怎么用"封装在这一层。与 `Port` 包着
//! `HolePie` 同构：**种类归内核，用法归 runtime**。
//!
//! 三个动词，与 Hole 那一族同形：
//!
//! ```text
//! ring   响     置"有待取之事"（内核在 trap 上下文响；用户侧今天没有调用者）
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
//! # 闸门是**按 hart** 记账的
//!
//! 内核那一位（`SEIE`）不是"铃的全局状态"，是**每颗 hart 各一份**：外部中断的 trap
//! 只在接了 PLIC context 的那颗 hart 上取到，`ring` 答 `Busy` 时关的是**取到 trap 的
//! 那颗**；而 `hush` 重开的是**调用者自己那颗**。两者可能不是同一颗 ⇒ 被关闸门的那颗
//! 得自己再开：内核的空闲循环（`kernel/src/work/room/scheduler/core/fetch.rs`）每轮
//! 无条件重开本核的 `SEIE`，并在 `SEIP` 还挂着时替控制器**再摇一次铃**——"跑任务"与
//! "空闲"两种长驻态因此各有振铃点，`raise_irq` 没有"没人可调"的窗口。
//!
//! # 为什么 `wait` 没有方向参数
//!
//! 铃只有一条方向（有事/没事）。签名少一个参数就把这件事说完了，不必写注释解释
//! "为什么只有 Pull"。

use env::Wait;
use env::{EnvResult, HoleDir};

use crate::env::mail::{self, NolePie};

/// 门铃：一枚 Nole + "怎么用它"。
pub struct Bell {
    pie: NolePie,
}

impl Bell {
    /// 收下一枚已经在对端的 Nole 门闩（`Pier` 递过来的 token）。
    pub fn new(pie: NolePie) -> Bell {
        Bell { pie }
    }

    /// 等铃响：`millis`（上限族，`Wait`）。
    ///
    /// 返回 `true` = 本次调用**当场就绪**（未挂起）；`false` = 未就绪（挂起过、或超时
    /// ——两者不分）。**不清**那一位，见模块头。
    pub fn wait(&self, millis: Wait) -> EnvResult<bool> {
        mail::wait(self.pie.token(), HoleDir::Pull, millis)
    }

    /// 应铃：清掉"有待取之事"。未响返 `Busy`（没有可取之事）。
    pub fn hush(&self) -> EnvResult<()> {
        mail::hush(self.pie.token())
    }

    /// 自响：置"有待取之事"并唤醒听者。已响返 `Busy`。
    ///
    /// **今天没有用户调用者**：`irq` 那道门铃由内核在 trap 上下文响
    /// （`kernel/src/platform/devices.rs`，不走门闩），域侧只 `wait` / `hush`
    /// （见 `driver/router/main.rs`）。留着它是因为"自己叫自己"是正当写法——
    /// 响者由持铃者决定，不是内核的特权。
    pub fn ring(&self) -> EnvResult<()> {
        mail::ring(self.pie.token())
    }
}
