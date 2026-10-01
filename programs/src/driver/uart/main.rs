#![no_std]
#![no_main]

//! uart — **串口驱动域**：`serial@10000000` 的持有者，兼**控制台服务**（**U 态**，见
//! `driver/uart/mod.rs`）。
//! **主流程只有三段**（本文件就是全部）：
//! ```text
//! 设备   领配给 → 开图 → 把"收到字节就拉线"打开（IER.RX，线的闸门归设备持有者）
//! 入系统 铸两枚孔（读口 / 写口）→ 上板（板因此看得见本域的死）→ 上树落两枚门牌
//!         → 从树上找到线路由者、登记本域那条线（报那一段区，不说线号）
//! 核心   两个源：写口上有客人交来的一条字 ⇒ 原样写进设备
//!               那条线一响 ⇒ 排空设备（读走 RBR）⇒ 把这一批字节推给读行的人
//!               ⇒ 说一句"这一条我排空了"（路由者据此把线放回去）
//! ```
//! **适配那几段不在这里**：`Device` / `Context` 住 [`programs::driver`]（各台共用的那些步骤）；
//! 设备面在 [`uart`](self)；服务台（两枚门牌那一趟 ＋ 把一条字写出去）在 [`desk`](self)；

extern crate alloc;
extern crate programs;

/// 纯功能：交出去的那一批（非空不可表达）。
mod core;

/// 服务台：两枚门牌那一趟 ＋ 从写口取一条字写出去。
mod desk;

/// 设备面（本域私有：谁的设备谁自己带）。
mod uart;

use crate::core::batch::Batch;
use crate::uart as device;
use env::{HoleDir, Wait};
use programs::driver::fail::Fail;
use programs::unit::uart::E_UART;
use runtime::PAGE_SIZE;
use runtime::core::pile::Pile;
use runtime::env::mail::HolePie;

/// 本域挂在树上的名字：`/svc/drv/uart`（[`protocol::driver::ROAD`] 之下的那一段，**服务名**）。
/// 它是一块 **Pane**：两枚门牌 `rx` / `tx` 在它下面。
const ME: &str = "uart";

/// 等板 / 等树 / 办一趟登记的总上限（毫秒）。**必须有界**：对面死在头几步时本域不能陪着挂死。
const MS: usize = 1000;

/// 一次排空最多搬走多少字节。FIFO 只有 16 字节，取四倍宽；满了剩下的还在设备里，
/// **下一次中断（本域说"排空了" ⇒ 路由者放回线）再来**。
const DRAIN_MAX: usize = 64;

/// 本域那一台：**返回类型就是它的死法**——`Err(Fail::at(E_UART, "…"))` 一路 `?` 出来，
/// `Ok(())` 是"跑完了"（常驻域走不到那一格）。**一族口径**在 [`programs::driver::fail`]
/// （号取自装配表：本域用的是 [`programs::unit::uart::E_UART`]，一个数都不写）。
#[programs::entry]
fn main() -> Result<(), Fail> {
    // 铸两枚孔 → 上板 ＋ 开会话 → 上树落两枚门牌 → **认领设备** → 开闸 → 占线：那一趟全在
    // [`desk::start`]（本台是唯一双向的一台，故它的路长一段、牌两枚）。本域既不写死设备名、
    // 也不写死地址："哪一台是串口"由设备账回答（类 `ns16550a`）。
    let desk = desk::start(Wait::AtMost(MS))?;

    // **两个源**：写口上有客人交来的一条字、线上有"设备收来了字节"——组等任意一格
    // 。
    let pile = Pile::unseal(false).map_err(|_| Fail::at(E_UART, "desk"))?;
    let lane = desk.line.hole().map_err(|_| Fail::at(E_UART, "line"))?;
    if pile.attach(&desk.tx, HoleDir::Pull).is_err()
        || pile
            .attach(&HolePie::from_token(lane), HoleDir::Pull)
            .is_err()
    {
        return Err(Fail::at(E_UART, "desk"));
    }
    let view = desk.dev.view();
    // 写口那一页：**余量**——本族一条消息远小于它（与 `rtc` 备缓冲同一手）。
    let mut word: alloc::vec::Vec<u8> = alloc::vec::Vec::new();
    if word.try_reserve_exact(PAGE_SIZE).is_err() {
        return Err(Fail::at(E_UART, "desk"));
    }
    word.resize(PAGE_SIZE, 0);
    let mut raw = [0u8; DRAIN_MAX];

    loop {
        if pile.await_(Wait::Forever).is_err() {
            return Err(Fail::at(E_UART, "line gone"));
        }
        // **先写口后设备**：写口那一头是客人正阻塞等着的（`push` 满了就睡），而设备里的字节在
        // FIFO 里排着，多等这一瞬不丢。
        // **一次写 = 一条完整的字**：这一条消息就是要写出去的全部字节，本域不拆不并。
        while let Ok((len, _)) = desk.tx.pull(&mut word, Wait::POLL) {
            device::put(view, &word[..len]);
        }
        // 设备那一趟（次序不动）：`receive` 吃的是路由者那一枚"线响了"的通知。
        while desk.line.receive(Wait::POLL).is_ok() {
            let n = device::drain(view, &mut raw);
            // 交给读行的人（读口那枚孔）。**这一手要阻塞**：字节是内容，丢了补不回来；读行的
            // 人（`canonical`）总会回到"取一行"那一格，故等它是有界的。
            // **`n == 0` 那一趟不推**：[`Batch::of`] 把那一格做进了类型（内核只收 `1..=一页`）。
            if let Some(batch) = Batch::of(&raw, n) {
                desk.ctx.publish(batch.bytes()).unwrap();
            }
            // 排空的**通知**照旧发：0 字节也算"这一条我处理完了"——那一格回闲 + 把线放回去。
            desk.line.exhaust().unwrap();
        }
    }
}
