#![no_std]
#![no_main]

//! uart — **串口驱动域**：`serial@10000000` 的持有者（S 态 supervisor 域）。
//!
//! ```text
//! 收配给（父域按本域那张单子推来记录，按 Slot 归位）
//!   → 开图：那台串口那一页借映进本域
//!   → 把"收到字节就拉线"打开（`IER.RX`）——**线的闸门归设备持有者**
//!   → 上板：板因此看得见本域的死（**不挂牌子**：没有服务面，没有名字可挂）
//!   → 从树上找到线路由者（`/device/router`）、**登记本域那条线**（报设备名，不说线号）
//!   → 常驻：守那条线——投递到了就记一行
//! ```
//!
//! # 为什么一字节都不读
//!
//! 读口今天仍归**内核的调试面**（`echo` 走 `DebugCall`，那一段是内核直通固件）。本域一读
//! `RBR` 就把那条路抢了——回显当场失效，而验收门只看回显。故这一刀只落**所有权**：
//! 设备门闩在本域手里、闸门由本域开；"排空设备 + 把字节交给客户端"是控制台那一刀。
//!
//! **照实记**：因为不读，本域也说不出**有内容的** `exhaust`（"这一条我排空了"）——排空的前提
//! 是读走设备里的字节。故路由者那一侧仍靠一拍放回；等读口搬过来，那一句才是真的。
//!
//! # 为什么它不退场
//!
//! **资源寿命 = 能力寿命**：那枚门闩在本域表里 ⇒ 本域一退，设备就没人持有了（而 `IER`
//! 那一位是**硬件状态**，会留在原地）。故它的收场只有两条路：被编排域收掉（`Ruin`）或
//! 随级联走。
//!
//! # 特权级照实记
//!
//! 今天声明为 S 态，是**照搬**（旧说"要读写寄存器"——那不是理由：banner 里串口与 PLIC 的
//! PMP 都是 **S/U (R,W)**，U 态读得动）。"驱动该 S 还是 U"这一格**还没有读数**，旧树的
//! `docs/driver.md §2` 裁过"驱动是 U 态域"；那一裁等专门的刀。

extern crate alloc;
extern crate programs;

// 共享件住驱动这一族里：`assemble` 是两台驱动都要写一遍的那段客侧装配。
use programs::driver::assemble;
use programs::driver::uart::needs;

// 板：本域是**客侧**（只装上板路，不挂牌子）；树：也是客侧（按名找线路由者）。
use protocol::operator::call as ocall;
use protocol::system::board::client as board;

use env::{Name, PieToken, TaskId};
use protocol::driver::line;
use runtime::core::dock::Dock;
use runtime::env::debug;
use runtime::env::mail::PolePie;
use runtime::env::room::exit_with;
use runtime::env::unit as utask;

/// 设备侧（本域私有：谁的设备谁自己带）。
mod uart;

/// 等板 / 等树 / 办一趟登记的总上限（毫秒）。**必须有界**：对面死在头几步时本域不能陪着挂死。
const MS: usize = 1000;

/// 要找的那位服务（线路由者）在树上的名字。
const SERVICE: &str = "router";

/// 本地失败编号（装配那三步用 [`assemble`] 的家族编号 1–3）。
const E_OPEN: usize = 4;
const E_BOARD: usize = 5;
const E_LINE: usize = 6;

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    // 1. 客侧装配：父域按本域那张单子把 `serial@10000000` 授进来。
    let mut slots = [None; needs::WANTS.len()];
    let got = match assemble::receive(&mut slots, needs::slot_of) {
        Ok(n) => n,
        Err(code) => exit_with(code),
    };
    let [Some(serial)] = slots else {
        // 单子上只有一条，缺了它就没得开工（父域按同一张单发货，缺格即装配错）。
        exit_with(assemble::E_GRANT)
    };
    say(&alloc::format!("uart: got {got}"));

    // 2. 开图 + 开闸。**设备到手之后第一件要打开的就是"收到字节就拉线"**：这条线归本域，
    //    因为只有持有设备的人才有资格动它（`ONLY` 是资源事实，见 `needs`）。
    let Ok(dock) = Dock::open(PolePie::from_token(serial)) else {
        exit_with(E_OPEN)
    };
    uart::arm_rx(dock.view());
    say("uart: serial@10000000 ier=rx");

    // 3. 上板：**只为让板看得见本域的死**（本域开的那扇门随收尾封印 ⇒ 板当场看出来）。
    //    不挂牌子——没有服务面就没有名字。**问话孔照交**：不交的那一位在板账上永远
    //    "没挂齐"，板线程会一直退化成 1 ms 节拍（`board::settle` 的 `unarmed`）。
    let Ok(sire) = utask::sire() else {
        exit_with(E_BOARD)
    };
    let Ok((_link, board)) = board::open(sire, MS) else {
        exit_with(E_BOARD)
    };
    if board::ask_hole(board).is_err() {
        exit_with(E_BOARD);
    }

    // 4. **登记本域那条线**：按名从树上找到线路由者（`/device/router`），报的是本域那张需求单
    //    上的**设备名**——"线 = 名字的函数"那条权威在路由者那边解，本域从不说线号。
    let Ok(reserved) = register(sire) else {
        exit_with(E_LINE)
    };
    say("uart: line reserved");

    // 5. 常驻：守本域那条线——投递到了就记一行。**今天不排空**（读口在内核的调试面手里，
    //    本域一读就把回显抢了），故本域说不出有内容的"我排空了"：放回仍归路由者那一拍。
    let mut buf = [0u8; 8];
    let mut seen = 0u32;
    loop {
        let Ok(no) = reserved.recv(&mut buf, usize::MAX) else {
            exit_with(E_BOARD)
        };
        if no != seen {
            seen = no;
            say(&alloc::format!("uart: line {no} rang"));
        }
    }
}

/// 从树上找到线路由者，把本域那条线登记下来。
///
/// 设备名取自**本域那张需求单**（名字只有一处）；入口经会话从树上授进来，泊位由 `line` 那一层装。
fn register(sire: TaskId) -> Result<line::client::Line, ()> {
    let (tree, host) = protocol::operator::client::open(sire, MS).map_err(|_| ())?;
    let talk = protocol::operator::client::ask_hole(host).map_err(|_| ())?;
    let dir = Name::new(protocol::driver::DIR).map_err(|_| ())?;
    let want = Name::new(SERVICE).map_err(|_| ())?;
    let path = [dir, want];
    let code =
        protocol::operator::client::ask(talk, &tree, host, ocall::FIND, &path, PieToken::NONE, MS)
            .map_err(|_| ())?;
    if code != ocall::OK {
        return Err(());
    }
    let entry = protocol::operator::client::take(&tree, host).ok_or(())?;
    let device = needs::WANTS[0].name().ok_or(())?;
    line::client::Line::reserve(entry, device, MS).map_err(|_| ())
}

/// 打一行。调试面是"服务还没起来的嘴"：本域没有会话、没有控制台，只有它。
fn say(msg: &str) {
    let _ = debug::put(msg);
}
