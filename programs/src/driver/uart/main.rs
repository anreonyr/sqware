#![no_std]
#![no_main]

//! uart — **串口驱动域**：`serial@10000000` 的持有者，兼**控制台服务**（**U 态**，见
//! `driver/uart/mod.rs`）。
//!
//! ```text
//! 收配给（父域按本域那张单子推来记录：**位置即格**）
//!   → 开图：那台串口那一页借映进本域
//!   → 把"收到字节就拉线"打开（`IER.RX`）——**线的闸门归设备持有者**
//!   → 上板：板因此看得见本域的死（**不挂牌子**：名字挂在树上）
//!   → 上树：门牌 `/device/uart` —— 牌子上挂的就是"读行"的那枚孔
//!   → 从树上找到线路由者（`/device/router`）、**登记本域那条线**（报那一段区，不说线号）
//!   → 常驻：那条线一响 ⇒ **排空设备**（读走 `RBR`）⇒ 把这一批字节推给读行的人
//!            ⇒ 说一句"这一条我排空了"（路由者据此把线放回去）
//! ```
//!
//! **本文件只剩流程**：起手在 `adapt/boot.rs`，上树与登记在 `adapt/tree.rs`，常驻在
//! `adapt/resident.rs`，死法在 `adapt/fail.rs`；"这一批能不能交"那条纪律在 `core/batch.rs`。
//! 服务面的判据与照实记（读口归谁、一条消息是什么、为什么它不退场、特权级）在
//! `driver/uart/mod.rs`。

extern crate alloc;
extern crate programs;

/// 住持面（适配）：起手 / 上树 / 常驻 / 死法——由 bin 自己 `mod`。
mod adapt;

/// 纯功能：交出去的那一批（非空不可表达）。
mod core;

/// 设备面（本域私有：谁的设备谁自己带）。
mod uart;

/// 本域那一台：**返回类型就是它的死法**——`Err(Fail::at(Step::…))` 一路 `?` 出来，
/// `Ok(())` 是"跑完了"（常驻域走不到那一格）。一格一格在 [`adapt::fail`] 里，
/// **一族口径**在 [`programs::driver::fail`]（号取自装配单）。
#[programs::entry]
fn main() -> Result<(), adapt::fail::Fail> {
    // 1–3 ＋ 树那条会话：领配给 → 开图开闸 → 上板。
    let up = adapt::boot::up()?;
    // 4–5：上树那一趟 + 登记本域那一条线。
    let held = adapt::tree::plate(&up)?;
    // 6：常驻。
    adapt::resident::run(&up, held)
}
