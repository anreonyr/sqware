#![no_std]
#![no_main]

//! serial@10000000 的持有者，兼控制台服务（U 态，见 `driver/uart/mod.rs`）。
//!
//! # 常驻那一圈（两条路都不挂起）
//! ```text
//! ① 取写口：人敲的字落在 tx 那一具架里 —— 读干为止（`recv(POLL)`）
//! ② 等组：**线的铃 ＋ tx 那一具架**（页上那一位）—— 这一圈唯一的阻塞格
//! ③ 排空设备：FIFO → 一批 → 落进 rx 那一具架（**写端永不挂起**）
//! ```
//! **为什么不再有 `pending` / `handed` / 两半握手**：写端只 `send`（满了按 `Mode::Oldest`
//! 顶掉最旧未读格、记在 `lost` 上），读端只在自己的架上等——**谁也不等对方**，故"两边各压
//! 一手、等对面先收"那一族环（见 `da97c34`）从构造上不可达。设备 FIFO 也不再被憋住：
//! 从前 `pending` 一满就停止 drain（字节无声丢在 FIFO 里、无人计数），现在每趟都 drain。

extern crate alloc;
extern crate programs;

/// 起手全在 `adapt::desk`；判定在 `core`/`dev`；本文件只剩流程。
mod adapt;

mod dev;

use crate::dev::uart as device;
use env::{HoleDir, Wait};
use programs::driver::shared::fail::Fail;
use programs::driver::uart::core::frame::{Bytes, DRAIN_MAX};
use programs::unit::uart::E_UART;
use ::resource::pile::Pile;

const MS: usize = 1000;

/// `Ok(())` 是"跑完了"（常驻域走不到那一格）。**一族口径**在 programs::driver::shared::fail
#[programs::entry]
fn main() -> Result<(), Fail> {
    let mut desk = adapt::desk::start(Wait::AtMost(MS))?;

    // **两个源**：写口那一具架上有人交来的一条字、线上有"设备收来了字节"——组等任意一格
    // （与 `rtc` 那一台同一条判据）。tx 那一格就是**那一枚页**（页上那一位即铃）。
    let pile = Pile::unseal(false).map_err(|_| Fail::at(E_UART, "desk"))?;
    let lane = desk.line.hole().map_err(|_| Fail::at(E_UART, "line"))?;
    if pile
        .attach(desk.tx.ship(), HoleDir::Pull)
        .is_err()
        || pile
            .attach(lane, HoleDir::Pull)
            .is_err()
    {
        return Err(Fail::at(E_UART, "desk"));
    }
    let view = desk.dev.view();
    let mut raw = [0u8; DRAIN_MAX];

    loop {
        // ① 写口：人敲的字（读干为止；读空的那一趟把页上那一位应掉）。
        while let Ok(one) = desk.tx_r.recv(Wait::POLL) {
            device::put(view, one.bytes());
        }
        // ② 等组（**唯一会挂起的一格**）。
        if pile.await_(Wait::Forever).is_err() {
            return Err(Fail::at(E_UART, "line gone"));
        }
        // ③ 设备 FIFO 排空，逐批落进 rx 那一具架（**永不挂起**）。
        // FIFO 里排着，多等这一瞬不丢。
        while desk.line.receive(Wait::POLL).is_ok() {
            let n = device::drain(view, &mut raw);
            if let Some(batch) = Bytes::of(&raw[..n]) {
                // 满了（`Mode::Oldest`）由架顶掉最旧未读格并把数记在 `lost` 上——不由本域等。
                let _ = desk.rx_w.send(&batch);
            }
            // 排空的**通知**照旧发：0 字节也算"这一条我处理完了"——那一格回闲 ＋ 把线放回去。
            desk.line.exhaust().unwrap();
        }
    }
}
