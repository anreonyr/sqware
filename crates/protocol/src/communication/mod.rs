//! communication — **通信**：关系怎么建立、一枚孔上怎么收发。
//!
//! ```text
//!   establish.rs  Pair / endpoint / give / find   关系怎么建立
//!   sender.rs     Sender<M>                       我推的那一枚（类型 = 我发的那种报）
//!   receiver.rs   Receiver<M>                     我收的那一枚（类型 = 我收的那种报）
//! ```
//!
//! # 一条不变量：Mail 是单向单槽
//!
//! 一枚 `PieToken` 一个方向。故"收发"**不是**一个双向端点上的两件事，而是**两枚孔、
//! 两个对象**——与 `std::sync::mpsc` 的 `Sender<T>` / `Receiver<T>` 同构（那边也是两个类型、
//! 两个方向）。**类型挂在方向上**：`Sender<M>` 里 `M` = 我发的那种报，`Receiver<M>` 里
//! `M` = 我收的那种报，两者可以不同族（问话与答话本来就是两种）。
//!
//! # 期限在**每次调用**上
//!
//! 同 mpsc 的 `recv_timeout`：`Wait` 一个参数说尽三态——`POLL`（= `AtMost(0)`）**就是**
//! `try_send` / `try_recv`：单次尝试、一次也不挂起。**只有建立那一手例外**：它收一格
//! `claim_for`，因为"等对方那一枚孔"与"等对方的字节"是两件事（前者是关系，后者是数据）。
//!
//! # 两个手柄都不持缓冲
//!
//! 发的那只在**这一帧的栈上**借本族那只（`Message::EMPTY`）——那条报是自己编的，超不出本族
//! 最长；收的那只由**调用方**给——推得进来什么由载体定界（一页），门 / 服务那一侧要给
//! **载体那一页**（理由与实测见 [`receiver::Receiver::recv`]）。
//!
//! # 本层不认识什么
//!
//! 正文、判定、账（那些住 `system` / `driver` 的 `core.rs`）；也不认识服务与 RPC——一问一答
//! 那三句（编 → 发 → 收 → 解 → 折失败）住各族自己的 `client.rs`（折失败那一步每家不同，
//! 共享一手必须带回调，那是"为复用造抽象"）。本层只认两样：**孔**，与**这一路上流的那种报**。

use env::Wait;
use runtime::env::chrono;

pub mod establish;
pub mod hands;
pub mod receiver;
pub mod sender;

/// 期限 → **那个到不了的点**（单调钟，纳秒）。
///
/// **永久落成 `u64::MAX`，不落成 `Wait::Forever`**：唤醒那一手要的是一格期限，而
/// "到不了的点"与"永久"在这条路上的处置不同（见原 `Quay::claim` 的照实记——内核的武装点
/// 被 `BLIND_MS` 收着，故"到不了的点" = 每 ~100 ms 复探一次，那是这条等待今天的护栏）。
pub(crate) fn deadline(wait: Wait) -> u64 {
    match wait {
        Wait::Forever => u64::MAX,
        Wait::AtMost(ms) => chrono::clock().saturating_add(ms as u64 * 1_000_000),
    }
}

/// 那个点还剩多久（`POLL` = 已经不剩）。**单调钟按纳秒读**，不依赖 timebase 频率。
pub(crate) fn remain(deadline: u64) -> Wait {
    if deadline == u64::MAX {
        return Wait::Forever;
    }
    let now = chrono::clock();
    if now >= deadline {
        Wait::POLL
    } else {
        Wait::AtMost(((deadline - now) / 1_000_000) as usize)
    }
}
