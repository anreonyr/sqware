#![no_std]
#![no_main]

//! uart — **串口驱动域**：`serial@10000000` 的持有者，兼**控制台服务**（**U 态**，见下）。
//!
//! ```text
//! 收配给（父域按本域那张单子推来记录：**位置即格**）
//!   → 开图：那台串口那一页借映进本域
//!   → 把"收到字节就拉线"打开（`IER.RX`）——**线的闸门归设备持有者**
//!   → 上板：板因此看得见本域的死（**不挂牌子**：名字挂在树上）
//!   → 上树：门牌 `/device/uart` —— 牌子上挂的就是"读行"的那枚孔（见下）
//!   → 从树上找到线路由者（`/device/router`）、**登记本域那条线**（报那一段区，不说线号）
//!   → 常驻：那条线一响 ⇒ **排空设备**（读走 `RBR`）⇒ 把这一批字节推给读行的人
//!            ⇒ 说一句"这一条我排空了"（路由者据此把线放回去）
//! ```
//!
//! # 读口归设备持有者
//!
//! 从前读口在**内核的调试面**（`echo` 走 `DebugCall` → SBI DBCN → 固件的 `uart8250_getc` 读
//! `RBR`）：那一段由固件代读，设备里的字节**先被它取走**——实测：20 字节喂进去，本域每次被
//! 投递叫醒去看，FIFO 已经空了（本域排空读到 0，调试面读到 20）。今天读口搬到本域：`IER.RX`
//! 由本域开、`RBR` 也由本域读——**一台设备只有一个读者**，这是"持设备者才有资格动它"的直接后果。
//!
//! # 服务面：交出去的是什么
//!
//! **一次排空读到的字节，原样交出去**。服务入口那枚孔**就是**读行的那一枚：客人（`echo`）
//! 经树上 `FIND /device/uart` 拿到的副本，就是"从本域读"的那一份。**一条消息 = 一次排空，
//! 而边界无意义**——它是一条字节流；"一行"是**终端**的约定（按行显示在客人那侧，见
//! `programs/src/user/echo.rs`），不是设备的事。不另立帧形：孔是单槽、一次推就是一个消息单元，
//! 再写一层长度前缀只是把同一件事说两遍。
//!
//! **照实记**：排空读到 0 字节时**不推**（内核 `Push` 那一格不收 0 字节的报文）——这不是丢字节，
//! 一批 0 字节本来就没有内容可交。**照实记之二**：树授出去的副本是 `R|W|VEST`
//! （[`protocol::system::operator::call::ship`] 的统一口径），故客人**也推得动**这枚孔；"客人在读、
//! 不在写"是约定、不是判据——与线路由者那边"抢线没有属主验证"同一类代价。
//!
//! # 为什么它不退场
//!
//! **资源寿命 = 能力寿命**：那枚门闩在本域表里 ⇒ 本域一退，设备就没人持有了（而 `IER`
//! 那一位是**硬件状态**，会留在原地）；门牌那枚孔也是本域铸的 ⇒ 本域一退，读行的人当场
//! 看出来（那枚孔径死）。故它的收场只有两条路：被编排域收掉（`doom`）或随级联走。
//!
//! # 特权级：量出来的 U 态
//!
//! 今天是 **U 态**。旧树这一格写 S 态、理由是"要读写寄存器"——那不是理由：banner 里串口与
//! PLIC 的 PMP 都是 **S/U (R,W)**，U 态读得动。这一格现在**有读数**了：清单 里两台
//! 驱动都写成 `User` 之后，两道门（examine 3/3、soak 10/10）照旧全过——写 `IER`、读 `RBR`、
//! `claim`/`complete`、持门闩、铸孔、挂组都不需要 S 态。**唯一收在 S 态的一格是铸铃**，而
//! 路由者那枚铃是**内核给的**（它只 `hush`，不铸）。

extern crate alloc;
extern crate programs;

// 共享件住驱动这一族里：`assemble` 是三台驱动都要写一遍的那段客侧装配。
use programs::driver::assemble;
use programs::driver::uart::needs;

// 板：本域是**客侧**（只装上板路，不挂牌子）；树：也是客侧（门牌挂 `/device/uart`、按名找线路由者）。
use protocol::system::operator::Where;
use protocol::system::operator::call as ocall;
use protocol::system::operator::client as operator;
use protocol::system::board::client as board;

use cases::Suite;
use env::{Name, PieToken, TaskId};
use protocol::driver::line;
use protocol::session::Quay;
use runtime::core::dock::Dock;
use runtime::env::debug;
use runtime::env::mail::{self, HolePie, PolePie};
use runtime::env::unit as utask;

/// 设备侧（本域私有：谁的设备谁自己带）。
mod uart;

/// 本域的死法（编号 + 那句话）——见那个文件与 `programs::Exit`。
mod fail;

/// 要找的那位服务（线路由者）在树上的名字。
const SERVICE: &str = "router";

/// 本域挂在树上的名字：`/device/uart`（[`protocol::driver::DIR`] 之下的那一段，**服务名**）。
const ME: &str = "uart";

/// 等板 / 等树 / 办一趟登记的总上限（毫秒）。**必须有界**：对面死在头几步时本域不能陪着挂死。
const MS: usize = 1000;

/// 一次排空最多搬走多少字节。FIFO 只有 16 字节，取四倍宽；满了剩下的还在设备里，
/// **下一次中断（本域说"排空了" ⇒ 路由者放回线）再来**。
const DRAIN_MAX: usize = 64;


/// 本域那一台：**返回类型就是它的死法**——`Err(Fail::…)` 一路 `?` 出来，
/// `Ok(())` 是"跑完了"（常驻域走不到那一格）。
#[programs::entry]
fn main() -> Result<(), fail::Fail> {
    // 1. 客侧装配：父域按本域那张单子把 `ns16550a` 那一台授进来（坐标由它读树定下来）。
    let mut slots = [None; needs::WANTS.len()];
    let got = assemble::receive(&mut slots)?;
    let [Some(serial)] = slots else {
        // 单子上只有一条，缺了它就没得开工（父域按同一张单发货，缺格即装配错）。
        return Err(fail::Fail::Assemble(assemble::E_GRANT));
    };
    say(&alloc::format!("uart: got {got}"));

    // 2. 开图 + 开闸。**设备到手之后第一件要打开的就是"收到字节就拉线"**：这条线归本域，
    //    因为只有持有设备的人才有资格动它（`ONLY` 是资源事实，见 `needs`）。
    let dock = Dock::open(PolePie::from_token(serial.token())).map_err(|_| fail::Fail::Open)?;
    uart::arm_rx(dock.view());
    // 坐标**随记录发下来**（内核按 `reg` 段造的门闩；本域既不写死名字、也不写死地址）。
    let key = serial.key().ok_or(fail::Fail::Open)?;
    // 设备门的坐标只能是区（内核就是按 `reg` 段造的）；别的形就是配给错了。
    let base = key.base().ok_or(fail::Fail::Open)?;
    say(&alloc::format!("uart: ier=rx at={base:#x}"));

    // 3. 上板：**只为让板看得见本域的死**（本域开的那扇门随收尾封印 ⇒ 板当场看出来）。
    //    不挂牌子——名字在树上。**问话孔照交**：不交的那一位在板账上永远"没挂齐"，
    //    板线程会一直退化成 1 ms 节拍（`board::settle` 的 `unarmed`）。
    let sire = utask::sire()?;
    let (_link, board) = board::open(sire, MS).map_err(|_| fail::Fail::Board)?;
    if board::ask_hole(board).is_err() {
        return Err(fail::Fail::Board);
    }

    // 4. **上树一条会话，办两趟**：落自家的门牌（读行的那枚孔）、再找线路由者登记那条线。
    //
    //    **只能开一条**：`operator::open` 装的是"一条叫 `operator` 的泊位"（`Quay::seat`），
    //    同一个域开第二条会撞同名（`Seat::NoName`）——而两次 `open` 拿到的还是两条*不同*的
    //    会话，孔各自归各自的表，混用更糟。故这里一次开、几手都用它（实测栽过：第二趟
    //    `open` 失败 ⇒ 本域当场退出，客人那一侧读一枚封了的孔，一个字节都读不到）。
    let (link, host) = operator::open(sire, MS).map_err(|_| fail::Fail::Tree)?;
    let talk = operator::ask_hole(host).map_err(|_| fail::Fail::Tree)?;
    let entry = mail::unseal_hole(board::ENTRY_MARK).map_err(|_| fail::Fail::Tree)?;
    serve_tree(&link, talk, host, entry);

    // 5. **登记本域那条线**：按名从树上找到线路由者（`/device/router`），报的是**发下来的那一段
    //    区**——"线 = 区的函数"那条权威在路由者那边解，本域从不说线号，也不自己造坐标。
    let held = register(&link, talk, key).map_err(|_| fail::Fail::Line)?;
    say("uart: line occupied");

    // 6. 常驻：那条线一响 ⇒ 排空设备 ⇒ 把这一批字节交给读行的人 ⇒ 说一句"我排空了"。
    //
    //    顺序是有意的：**先读走设备里的字节，再说"排空了"**——路由者收到那句话才把线放回
    //    （`exhaust` 是一个事件，不是节拍；而它不阻塞，见 `line::client::Line::exhaust`）。
    //    反过来的话，线放回了而字节还挂在设备里，就是"电平一直高、却没人读"的空转。
    let console = HolePie::from_token(entry);
    let mut raw = [0u8; DRAIN_MAX];
    loop {
        // 这一格失败 = 那条线没了 ⇒ 这个域没有可继续的状态（照实报 `Dead`）。
        if held.receive(usize::MAX).is_err() {
            return Err(fail::Fail::Dead);
        }
        let n = uart::drain(dock.view(), &mut raw);
        // 交给读行的人。**这一手要阻塞**：字节是内容，丢了补不回来；读行的人（`echo`）
        // 总会回到"取一行"那一格，故等它是有界的。
        let handed = n == 0 || console.push(&raw[..n]).is_ok();
        let _ = held.exhaust();
        say(&alloc::format!("uart: rang n={n} out={handed}"));
    }
}


/// 上树那一趟：**分目录 → 落门牌 → 查回来验一遍**（门牌 = 那枚读行的孔）。
///
/// ```text
///   PART ["device"]           → 0 = 拿到那块目录的号（本域建的 / 已经在了——`part` 幂等）
///   LAND ["device","uart"]    → 0 = 门牌落上（那枚孔经会话交给持树者）
///   FIND ["device","uart"]    → 0 = 查得到，且那一枚经会话授回本域表里
///   got                        → 本域在表里认出刚授回来的那一枚了吗
/// ```
///
/// `got` 只是"认出了那一枚"；它指不指得回原物，由**真客人**（`echo`）证——它照同一条路
/// 找上门、从这枚孔读行。故本域不自问自答。
fn serve_tree(link: &Quay, talk: PieToken, host: TaskId, entry: PieToken) {
    let (Ok(dir), Ok(me)) = (Name::new(protocol::driver::DIR), Name::new(ME)) else {
        say("uart: tree: bad name");
        return;
    };
    // **分目录 → 落门牌 → 查回来验一遍**：分与落各自**答出那一格的号**（"号出门"那一手）。
    // **分目录**：`part` 是**幂等**的——那块目录已经在就答它那个号（里面有没有东西不管）。
    let dir_at = operator::part(talk, link, Where::Root, dir, MS);
    let (part, dir_id) = match dir_at {
        Ok(id) => (ocall::OK, id.get()),
        Err(code) => (code, 0),
    };
    // **落门牌**：答的是门牌自己那一格的号。
    let plate = match dir_at {
        Ok(at) => operator::land(
            talk,
            link,
            host,
            Where::At(at),
            me,
            entry,
            ocall::Rule::Public,
            true,
            MS,
        ),
        Err(code) => Err(code),
    };
    let (land, pid) = match plate {
        Ok(id) => (ocall::OK, id.get()),
        Err(code) => (code, 0),
    };
    // 查回来验一遍：**按号**（名字只在上面那两格用过，此后一律按号）。
    let (find, got) = match plate {
        Ok(id) => match operator::find(talk, link, id, MS) {
            Ok((code, entry)) => (code, entry.is_some()),
            Err(_) => (ocall::BAD, false),
        },
        Err(code) => (code, false),
    };
    // **`got` 换了来路**（乙′）：见 `ocall::pack_seed` 的照实记。
    // 拿号问名：**号 ↔ 名**这一对对得起来，才算那枚号是真坐标。
    let pname = plate
        .ok()
        .and_then(|id| operator::name(talk, link, id, MS).ok());
    say(&alloc::format!(
        "uart: tree part={part} dir={dir_id} land={land} find={find} got={got} entry={} plate={pid} pname={}",
        entry.get(),
        pname.as_ref().map(|n| n.as_str()).unwrap_or("-"),
    ));
    // **这一趟的判据**（值那几格从门那边搬进来：门只剩"这一行还在不在"）。
    let mut suite = Suite::new("uart-tree");
    suite.case("the_device_directory_answered", move || {
        assert_eq!(part, ocall::OK)
    });
    suite.case("the_plate_landed", move || assert_eq!(land, ocall::OK));
    suite.case("the_plate_was_found_by_id", move || {
        assert_eq!(find, ocall::OK)
    });
    suite.case("the_shipped_plate_came_back", move || assert!(got));
    suite.case("the_id_and_the_name_agree", move || {
        assert_eq!(pname.as_ref().map(|n| n.as_str()), Some(ME))
    });
    suite.run();
}

/// 从树上找到线路由者，把本域那条线登记下来。
///
/// 会话是**上面那一条**（同一个域只开一条，见 `main` 第 4 步）；坐标是**发下来的那一段区**
/// （随配给记录到本域，见 `main` 第 2 步——本域不写死它）；入口经会话从树上授进来，
/// 泊位由 `line` 那一层装。
fn register(
    link: &Quay,
    talk: PieToken,
    key: plan::Key,
) -> Result<line::client::Line, fail::Fail> {
    let dir = Name::new(protocol::driver::DIR).map_err(|_| fail::Fail::Line)?;
    let want = Name::new(SERVICE).map_err(|_| fail::Fail::Line)?;
    let road = [dir, want];
    // **间接寻址那一手**：名字先译成号（号才是树的直接坐标），此后按号。
    let id = operator::seek(talk, link, &road, MS).map_err(|_| fail::Fail::Line)?;
    let entry = match operator::find(talk, link, id, MS) {
        Ok((ocall::OK, Some(entry))) => entry,
        // **查不到**与**授不出去**都落进这一格（`find` 的状态那一格说得出是哪一种）。
        _ => return Err(fail::Fail::Line),
    };
    line::client::Line::occupy(entry, key, MS).map_err(|_| fail::Fail::Line)
}

/// 打一行。调试面是"服务还没起来的嘴"：本域没有会话、没有控制台，只有它。
fn say(msg: &str) {
    let _ = debug::put(msg);
}
