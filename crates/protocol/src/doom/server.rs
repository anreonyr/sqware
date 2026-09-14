//! doom·server — 他杀服务（服务侧）：**名字 → 活实例 → 内核下令 → 等它真的没了**。
//!
//! 三步的分工就是本设计的全部：内核回答"能不能"（血缘判据），**目录**回答"这个名字
//! 现在是谁"（名字的账在它那儿，本模块不存第二份），而 `Ok` 这个回执的含义是"内核确认
//! 它**回收完了**"——不是"收到了"。
//!
//! # 本模块不做 I/O
//!
//! [`serve`] 只产出 [`Outcome`]：回执的推送、服务的收场由调用方（服务线程）做——
//! 与 `dispatch::server::Directory::serve` 同一条规矩（"本函数不做 I/O——回复由调用方
//! 推送"）。
//!
//! # 两个调用者共用 [`collect`]
//!
//! 他杀服务（用户发来的 `Kill`）与 **root 的关机收尾**（`docs/root.md` 4.3）。两件事
//! 本来就是同一件——把"这个名字此刻指向的那个域"收掉，并等到它真的没了；差别只在
//! 前者要把结局回给调用方，后者只想知道"收干净没有"。
//!
//! # 等它走用的是"探手里这枚副本还在不在"，不是 `Join`
//!
//! `Join` 的授权是**任务粒度**的（谁生的谁能等，`docs/driver.md` §8.1.20 实测监护线程
//! 等不到主线程生下的那个），而 console 可能正是监护线程重发的 ⇒ 关机收尾那条路不能
//! 依赖 `Join`。
//!
//! "该不该"只剩一层，且不是判据：**够得着就能请求**——入口门闩只亲授给 root 引荐过的
//! 域（`Refer` 的产物），沙箱里的域连不上目录，也就拿不到这扇门的副本。

use core::time::Duration;

use env::{PieToken, TaskId};

use runtime::env::mail;
use runtime::env::room;

use crate::dispatch::client::Directory;

use runtime::core::port::address_of;

use super::wire::{Ack, OP_KILL, OP_QUIT, Query};

/// 服务处理一条请求时，等目标**消失**的探测间隔（毫秒）与轮次。
///
/// 探的是什么：**服务手里那枚指向目标的副本还在不在**——目标一死，内核的退出钩子会沿
/// 派生链把它（连同目录手里那枚）一起摘掉，那正是"客户端死亡"用的同一条机制
/// （`gate::doom` 的 BFS）。副本没了 = 它走完了死亡路径；探完仍活着 = [`Ack::Slow`]。
pub const GONE_ROUND_MS: usize = 20;
pub const GONE_ROUNDS: usize = 25;

/// 一条请求处理完的结局——**要推什么、要不要收场**，由调用方落地。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Outcome {
    /// 回执推给 `ack`（调用方拿它推 [`Ack::byte`]），一次往返就此走完。
    Reply { ack: PieToken, status: Ack },
    /// 主人说的那一句"停服"：调用方收场（**不是**一次往返，没有回执）。
    Quit,
    /// 坏报文 / 认不得的动词 / 不是给本服务的：什么都不做。
    Ignore,
}

/// 处理一条报文：认动词、认人，其余交给 [`collect`]。
///
/// `from` = 内核盖章的推者；`owner` = 本服务的主人（本域的主线程）——`Quit` **只归它**。
pub fn serve(msg: &[u8], from: TaskId, dir: &Directory, owner: TaskId) -> Outcome {
    let Some(&op) = msg.first() else {
        return Outcome::Ignore;
    };
    match op {
        OP_QUIT if from == owner => return Outcome::Quit,
        OP_KILL => {}
        _ => return Outcome::Ignore,
    }
    let Some(kill) = Query::decode(msg) else {
        return Outcome::Ignore;
    };
    // 回信地址从**帧**里抠（它不在请求值里）：坏报文也欠对方一句答复，而这一句
    // 得知道往哪儿推——所以这一读发生在解码之后、与解码无关。
    let Some(ack) = address_of(msg) else {
        return Outcome::Ignore;
    };
    Outcome::Reply {
        ack,
        status: collect(dir, kill.target.as_str()),
    }
}

/// 收**一个名字**：解析 → 内核下令 → 有界等它真的没了。
pub fn collect(dir: &Directory, target: &str) -> Ack {
    // 名字 → 活实例 → 它的属主 task：目录给的入口门闩副本，`owner` 就是开它的那个域的
    // 主线程（`vestor` 会被转发改写，`owner` 不会）。解析完当场放下这枚副本——它不是
    // 我们的资源。
    let Ok(entry) = dir.connect_token(target) else {
        return Ack::Dead;
    };
    let owner = match mail::reserve(entry) {
        Ok((_, owner)) => owner,
        Err(_) => {
            let _ = mail::release(entry.get());
            return Ack::Dead;
        }
    };
    if owner.get() == 0 {
        let _ = mail::release(entry.get());
        return Ack::Dead;
    }
    match room::doom(owner) {
        Ok(()) => {}
        // `Dead`(-2) = 内核那侧对不上号（已回收 / 从未入册）；其余如实报"不许"。
        Err(e) if e.source.code() == -2 => {
            let _ = mail::release(entry.get());
            return Ack::Dead;
        }
        Err(_) => {
            let _ = mail::release(entry.get());
            return Ack::Denied;
        }
    }
    // 有界等"它真的没了"：探的是**手里这枚副本还在不在**——目标一死，内核的退出钩子
    // 沿派生链把它一起摘掉（"客户端死亡"用的同一条机制）。副本没了才回 `Ok`；
    // 探完还在就如实说"已下令、没等到"（`Slow`），**不假装成功**。
    for _ in 0..GONE_ROUNDS {
        if mail::reserve(entry).is_err() {
            return Ack::Ok;
        }
        let _ = room::sleep(Duration::from_millis(GONE_ROUND_MS as u64));
    }
    // 副本还在 ⇒ 目标还活着；这枚副本是解析时拿的，用完放下（它不属于我们）。
    let _ = mail::release(entry.get());
    Ack::Slow
}
