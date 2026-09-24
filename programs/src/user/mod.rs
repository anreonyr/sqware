//! user — **U 态那一档**：不建域、不读设备、不碰 MMIO，也不转授权。
//!
//! 判据是特权级（唯一声明处：`kernel/build.rs 的清单（`PRODUCTS` / `PROBES` / `RIGS`，按场景选表）`）：本目录下都是 `User`——
//! `echo.rs` / `guest.rs` / `passer.rs` / `sleeper.rs` / `subject.rs` / `member.rs` 与
//! `lodger/`，各是一份入口。
//!
//! **照实记**：这一句原先列的是"`echo.rs` / `guest.rs` / `passer.rs` / `sleeper.rs` /
//! `lodger/` 五位客人，各是一份入口（不被 lib 收进来）"——两句都不对了：身份那一刀带来的
//! `subject` 没被补进这张单子，而 `lodger` 的[需求单](lodger::needs)要进 lib（装配者照它
//! 开单）。这一刀（结盟服务 + 它的探针）按同一张 清单 把单子补全。
//!
//! [`lodger`] 是唯一**领了一枚门闩**的：它领的是那台 virtio 设备的寄存器页
//! （`virtio_mmio@10001000`，1 号线——**一条没人要的线**），但从不映视图、不读写它：领它只为
//! "主人"这个说法是真的（见 `lodger/main.rs` 头注），故"不读设备、不碰 MMIO"照旧成立。
//! 它与前三位形状上只差一处：它的
//! [需求单](lodger::needs)要进 lib（装配者照它开单），入口仍是一份 bin。
//!
//! 持树者（`operator`）**不在这里**：它 `ship` 带 `VEST` 的副本、是这台机器的转授权中枢，
//! 故与监督侧同档，住 [`crate::supervisor::operator`]。监督侧那一档整体在
//! [`crate::supervisor`]。

pub mod lodger;
