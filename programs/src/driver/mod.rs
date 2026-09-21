//! driver — **驱动这一族**：设备面的持有者，与它们共用的装配件。
//!
//! 判据是**角色**，不是特权级（与 [`crate::stress`] 同款）：本目录下的成员各自在
//! `kernel/build.rs::INITRD_BINS` 声明自己是哪一档——今天两台都是 `Supervisor`，而
//! "驱动该 S 还是 U"这一格**还没有读数**（banner 里 UART 与 PLIC 的 PMP 都是 S/U (R,W)，
//! 故 U 态读得动设备；旧树的 `docs/driver.md §2` 裁过"驱动是 U 态域"）。
//!
//! ```text
//!   router   线路由者（中断面域）：持有中断控制器，接 / claim / complete 每一条线
//!   uart     串口驱动：持有 serial@10000000，把"收到字节就拉线"打开
//! ```
//!
//! **线的权威 / 属主 / 登记 / 投递 / 抽干 / 收线**那一层的**功能模型**已立在
//! `crates/protocol/src/driver::line`（结构与签名未走门；`router` 今天只做"收与结"那一半）。
//!
//! # 门牌（已裁，未落）
//!
//! 驱动的门牌挂 `protocol::operator` 的 **`/device`**：名字用**服务名**（`router` / `uart`，
//! 与装配单、日志同一个名），那块 Pane 由**装配侧**建一次（树上 `land` 的父段不存在时答
//! `Unknown`、`part` 到一块非空 Pane 上答 `NonEmpty` ⇒ 只有一次创建机会），**按名找服务走树**；
//! 板留着管生死（编排域监督的唯一事件源是板那条死亡道）。
//!
//! **代价照实记**：树上撞名是**换绑**（旧的一枚放下，`operator` 不做 owner 判断）⇒ 拿得到树路
//! 的域能顶掉别人的门牌；板那边撞名是 `Taken`——门槛从"撞名即拒"降成"撞名即顶"。
//!
//! **今天两台都还没落**：`Program::operator` 都是 `false`——`router` 挂的是板，`uart` 连入口
//! 都没有（它没有服务面，挂不了树）。这一刀与 `line` 的载体（第 2 关）一起落。
//!
//! # 两条家族纪律
//!
//! - **设备语义各带各的**：谁的设备谁在自己目录里放设备模块（[`router`] 的 `plic.rs`、
//!   [`uart`] 的 `uart.rs`）——本仓不用一份"驱动框架"去包它们。
//! - **需求单归收方**：[`router::needs`] / [`uart::needs`] 各开自己那张单，装配者只是
//!   `use` 它们（见 [`crate::supervisor::service::Program`]）。
//!
//! 本级的 [`assemble`] 是两台驱动**都要写一遍**的那一段客侧装配（会话 + 收配给 + 归位）。

pub mod assemble;
pub mod router;
pub mod uart;
