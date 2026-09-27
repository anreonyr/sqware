#![no_std]
#![no_main]

//! uart — **串口驱动域**：`serial@10000000` 的持有者，兼**控制台服务**（**U 态**，见
//! `driver/uart/mod.rs`）。
//!
//! **主流程只有三段**（本文件就是全部）：
//!
//! ```text
//! 设备   领配给 → 开图 → 把"收到字节就拉线"打开（IER.RX，线的闸门归设备持有者）
//! 入系统 解门牌 → 上板（板因此看得见本域的死）→ 上树（/device/uart）
//!         → 从树上找到线路由者、登记本域那条线（报那一段区，不说线号）
//! 核心   那条线一响 ⇒ 排空设备（读走 RBR）⇒ 把这一批字节推给读行的人
//!         ⇒ 说一句"这一条我排空了"（路由者据此把线放回去）
//! ```
//!
//! **适配那几段不在这里**：`Device` / `Context` 住 [`programs::driver`]（三台逐字同构的那些
//! 步骤）；设备面在 [`uart`](self)；"这一批能不能交"那条纪律在 `core::batch`。服务面的判据与
//! 照实记（读口归谁、一条消息是什么、为什么它不退场、特权级）在 `driver/uart/mod.rs`。

extern crate alloc;
extern crate programs;

/// 纯功能：交出去的那一批（非空不可表达）。
mod core;

/// 设备面（本域私有：谁的设备谁自己带）。
mod uart;

use crate::core::batch::Batch;
use crate::uart as device;
use env::Wait;
use programs::driver::context::{Context, Mine};
use programs::driver::device::Device;
use programs::driver::fail::Fail;
use programs::program::uart::{E_UART, UART_WANTS as WANTS};
use protocol::debug;

/// 本域挂在树上的名字：`/device/uart`（[`protocol::driver::DIR`] 之下的那一段，**服务名**）。
const ME: &str = "uart";

/// 等板 / 等树 / 办一趟登记的总上限（毫秒）。**必须有界**：对面死在头几步时本域不能陪着挂死。
const MS: usize = 1000;

/// 一次排空最多搬走多少字节。FIFO 只有 16 字节，取四倍宽；满了剩下的还在设备里，
/// **下一次中断（本域说"排空了" ⇒ 路由者放回线）再来**。
const DRAIN_MAX: usize = 64;

/// 本域那一台：**返回类型就是它的死法**——`Err(Fail::at(E_UART, "…"))` 一路 `?` 出来，
/// `Ok(())` 是"跑完了"（常驻域走不到那一格）。**一族口径**在 [`programs::driver::fail`]
/// （号取自装配表：本域用的是 [`programs::program::uart::E_UART`]，一个数都不写）。
#[programs::entry]
fn main() -> Result<(), Fail> {
    // ── 设备 ───────────────────────────────────────────────
    let [serial] = Device::claim::<{ WANTS.len() }>()?;
    debug!("uart: got {}", WANTS.len());
    let dev = Device::open(serial).map_err(|_| Fail::at(E_UART, "device open failed"))?;
    device::arm_rx(dev.view());
    // 坐标**随记录发下来**（内核按 `reg` 段造的门闩；本域既不写死名字、也不写死地址）。
    let base = dev.key().base().ok_or(Fail::at(E_UART, "device open failed"))?;
    debug!("uart: ier=rx at={base:#x}");

    // ── 入系统 ─────────────────────────────────────────────
    // 解门牌 → 上板 ＋ 开会话 → 上树 → 占线：那一趟的壳在 [`Context::enter`]（两台逐字同构，
    // 失败那几格说**步名**）。报的是**发下来的那一段区**——"线 = 区的函数"那条权威在路由者
    // 那边解。
    let (ctx, line) = Context::enter(dev.key(), ME, Mine::Yes, E_UART, Wait::AtMost(MS))?;

    // ── 核心 ───────────────────────────────────────────────
    let mut raw = [0u8; DRAIN_MAX];
    loop {
        if line.receive(Wait::Forever).is_err() {
            return Err(Fail::at(E_UART, "line gone"));
        }
        let n = device::drain(dev.view(), &mut raw);
        // 交给读行的人（门牌那枚孔＝读行的那一枚）。**这一手要阻塞**：字节是内容，丢了补不回来；
        // 读行的人（`echo`）总会回到"取一行"那一格，故等它是有界的。
        //
        // **`n == 0` 那一趟不推**：[`Batch::of`] 把那一格做进了类型（内核只收 `1..=一页`）。
        if let Some(batch) = Batch::of(&raw, n) {
            ctx.publish(batch.bytes()).unwrap();
        }
        // 排空的**通知**照旧发：0 字节也算"这一条我处理完了"——那一格回闲 + 把线放回去。
        line.exhaust().unwrap();
    }
}
