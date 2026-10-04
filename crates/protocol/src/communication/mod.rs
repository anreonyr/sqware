//! 关系怎么建立、字节怎么过边界。
//! # 分两支：关系一支、缓冲两档
//! ```text
//! hand   一手：一枚孔上一条报，交完即走（单槽 ⇒ 满了只答 Busy）
//! rack   一具寄存架：一枚页上 N 格，满了按策略丢（有界 ⇒ 丢几个有数）
//! session 一条路：`establish`（两枚孔怎么到手）＋ `session`（路本身）
//! ```
//! **缓冲两档不共类型**：`Message` 那四样（`Buf` / `EMPTY` / `store` / `fetch`）是共用的，
//! 但失败域与时机不同（交接对寄存：`SendFail::Unbound` 对 `SendFail::Full`），故各是一个
//! 类型，也不抽公共"发送端"——抽出来就是"为复用造抽象"（见下面"本层不认识什么"）。
//! # 一条不变量：Mail 是单向单手
//! 一枚 `PieToken` 一个方向。故"收发"**不是**一个双向端点上的两件事，而是**两枚孔、
//! 两个对象**——与 std::sync::mpsc 的 `Sender<T>` / `Receiver<T>` 同构（那边也是两个类型、
//! 两个方向）。**类型挂在方向上**：`Sender<M>` 里 `M` = 我发的那种报，`Receiver<M>` 里
//! `M` = 我收的那种报，两者可以不同族（问话与答话本来就是两种）。
//! # 期限在**每次调用**上
//! 同 mpsc 的 `recv_timeout`：`Wait` 一个参数说尽三态——`POLL`（= `AtMost(0)`）**就是**
//! `try_send` / `try_recv`：单次尝试、一次也不挂起。**只有建立那一手例外**：它收一格
//! `claim_for`，因为"等对方那一枚孔"与"等对方的字节"是两件事（前者是关系，后者是数据）。
//! # 两个手柄都不持缓冲
//! 发的那只在**这一帧的栈上**借本族那只（Message::EMPTY）——那条报是自己编的，超不出本族
//! 最长；收的那只由**调用方**给——推得进来什么由载体定界（一页），门 / 服务那一侧要给
//! **载体那一页**。
//! # 本层不认识什么
//! 正文、判定、账（那些住 `system` / `driver` 的 `core.rs`）；也不认识服务与 RPC——一问一答
//! 那三句（编 → 发 → 收 → 解 → 折失败）住各族自己的 `client.rs`（折失败那一步每家不同，
//! 共享一手必须带回调，那是"为复用造抽象"）。本层只认两样：**孔**，与**这一路上流的那种报**。

use env::Wait;
use env::chrono;

pub mod hand;
pub mod rack;
pub mod session;

/// 期限 → **那个到不了的点**（单调钟，纳秒）
/// **永久落成 u64::MAX，不落成 Wait::Forever**：唤醒那一手要的是一格期限，而
pub(crate) fn deadline(wait: Wait) -> u64 {
    match wait {
        Wait::Forever => u64::MAX,
        Wait::AtMost(ms) => chrono::clock().saturating_add(ms as u64 * 1_000_000),
    }
}

/// 那个点还剩多久（`POLL` = 已经不剩）。**单调钟按纳秒读**，不依赖 timebase 频率
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
