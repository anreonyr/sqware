#![no_std]
#![no_main]

//! rtc — **第二台设备驱动**：`rtc@101000` 的持有者（设备树里那条 11 号线），兼**报时服务**。
//!
//! 它存在的理由原来是一条**判据**（线那四格、配给、门牌、设备面这一整套，只有 `uart` 一台
//! 真设备走过——**抽象等第二个实例**）。那一刀落完之后，它是**第二个实例走进服务面**的那一台：
//! `uart` 那一面只有一个方向（把排空读到的字节交出去），本面**两个方向都有**（客人问、设备叫）。
//!
//! ```text
//!   1  领配给：`rtc@101000` 那页寄存器（`ONLY`）
//!   2  开图 + **自证**：读两次它的纳秒计数器（两次不同 ⇒ 它真的在走）
//!   3  上板（板看得见本域的死）+ 上树一趟：**落门牌 `/device/rtc`**（报时服务的入口）
//!   4  占线：报**那一段区**（**线 = 区的函数**），收一格答码 —— 11 号线归本域
//!   5  常驻：**一只组等两个源**
//!        门上有请求（客人借来一枚回信孔）  问时间 → 就地答；定闹钟 → 占住那一格 + 武装设备
//!        线上有投递（设备自己拉的线）      清掉那一格 ⇒ 那一格到点 ⇒ 从那枚孔推"那一声"
//! ```
//!
//! # 服务面：两个方向放进同一面
//!
//! 那是**本驱动自己的具体协议**（协议层不放服务面：旧 `uart` 协议的死因就是把它放了进去），
//! 故它住本目录：帧形与记号在 [`call`]、那一格与失败域在 [`core`]、
//! 客侧两手在 [`client`]——客人与驱动 `use` 的是同一份源码。
//!
//! 树上的名字只多一处：本域的门牌（`/device/rtc`）。**板只管生死**（不挂牌子），
//! 与 `router` / `uart` 同一条分家。
//!
//! # 为什么是"借一枚回信孔"而不是"从门牌孔原地回话"
//!
//! 门牌那一枚孔是**本域自己开的**：客人没了它不死（寿命边那一句反过来说就是"开者是甲方、
//! 用者是乙方，乙方死了资源不死"，见 `kernel` 的 `gate::cull`）⇒ 闹钟响的时候本域会往一枚
//! 没人取的槽里推、**永远堵在那儿**，而那台设备从此没人排空。客人自己铸一枚孔借过来，
//! "客人还在不在"就由**那一枚孔自己**答（开者退场 ⇒ 它开的资源一起封印），本域既不必探活、
//! 也不必记"客人是谁"——那一格里只有"到点时刻 + 往哪回"。
//!
//! # 照实记：三处想当然被读数打回来
//!
//! 这一台在这个仓里以前没人量过，故本域起手先报一行自证读数（那对纳秒格子读两次），
//! 再靠一行行读数把设备语义坐实。第一版写了三处**想当然**，全被打回来：
//!
//! 1. **读时间**：先读高再读低（还自以为要"连读两次高、不变才算一对"）。真语义是**低半格那次
//!    读把高半格锁存起来** ⇒ 正确读法是**先低后高**，那一对天生自洽。
//! 2. **报警状态**：以为 `ALARM_STATUS` 是"到点了"，而它**在到点那一瞬是 0**（它是
//!    `alarm_running`：响过就清）。故本域改用 `IRQ_ENABLED`（闸门）与两头的时间读数说话。
//! 3. **写闹钟**：先写低半格再写高半格；真语义是**低半格那次写会当场比较一次**——首次写时高
//!    半格还是 0 ⇒ 当场判成"到点了"（实测：第一次武装在目标之前约 99 ms 就报了一次）。
//!    改成**先高后低**。
//!
//! 另量到一条：**这一格是电平源**——把 `CLEAR_INTERRUPT` 那一手临时去掉，同一段运行里投递
//! 从 5 次变成 **3093** 次（线一直挂着）。故"清掉那一格"不是客气。
//!
//! # 照实记：自走那一圈撤了
//!
//! 从前本域**每次排空之后立刻再武装一次**（10 Hz 自走）：那时没有客人，那一圈是"这台设备
//! 自己会拉线"的唯一读数。服务面落下之后，**那条读数改由客人的真请求产生**，自走就成了没人
//! 要的机制（"没有读数的机制不落"的反面）——故 `PERIOD_NS` 与启动时那一次武装整条撤掉，
//! 闹钟从此只由客人定。**读数照旧**（`router: line=11` / `rtc: rang n=` 还是由真闹钟产生），
//! 变的是它的**因果**：那一声是客人约来的。
//!
//! # 特权级
//!
//! **U 态**（`kernel/build.rs::INITRD_BINS`）：读那页寄存器、`claim` / `complete`、持门闩、
//! 铸孔挂组都不需要 S 态——驱动那一档是量出来的（见 `programs/src/driver/uart/main.rs` 头注）。

extern crate alloc;
extern crate programs;

// 共享件住驱动这一族里：`assemble` 是各驱动都要写一遍的那段客侧装配，需求单同一份源码编一次。
use programs::driver::assemble;
use programs::driver::rtc::needs;
// 服务面那三份：帧形与记号、那一格、客侧两手（客人 `use` 的是同一份）。
use programs::driver::rtc::{call, core::Slot};

// 板：本域是**客侧**（只装板路）；树：也是客侧（落门牌 + 按名找线路由者）。
use protocol::operator::Where;
use protocol::operator::call as ocall;
use protocol::operator::client as operator;
use protocol::system::board::client as board;

use alloc::format;

use env::{HoleDir, Name, PieToken, TaskId};
use protocol::driver::line;
use protocol::session::Quay;
use protocol::session::call as scall;
use runtime::core::dock::{Dock, View};
use runtime::core::tole::Tole;
use runtime::env::debug;
use runtime::env::mail::{self, HolePie, PolePie};
use runtime::env::room::exit_with;
use runtime::env::unit as utask;

/// 设备面（本域私有：谁的设备谁自己带）。
mod rtc;

/// 要找的那位服务（线路由者）在树上的名字。
const SERVICE: &str = "router";

/// 本域挂在树上的名字：`/device/rtc`（[`protocol::driver::DIR`] 之下的那一段，**服务名**）。
const ME: &str = "rtc";

/// 等板 / 等树 / 办一趟登记的总上限（毫秒）。**必须有界**：对面死在头几步时本域不能陪着挂死。
const MS: usize = 1000;

/// 本地失败编号（装配那三步用 [`assemble`] 的家族编号 1–3）。
const E_OPEN: usize = 4;
const E_BOARD: usize = 5;
const E_LINE: usize = 6;
const E_TREE: usize = 7;
const E_DESK: usize = 8;

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    // 1. 领配给：那一页寄存器（`ONLY`：同一时刻只该有一个持有者）。
    let mut slots = [None; needs::WANTS.len()];
    let got = match assemble::receive(&mut slots) {
        Ok(n) => n,
        Err(code) => exit_with(code),
    };
    let [Some(rtc_pie)] = slots else {
        exit_with(assemble::E_GRANT)
    };
    say(&format!("rtc: got {got}"));

    // 2. 开图 + 自证：那对纳秒格子读两次（两次不同 ⇒ 它是活的）。
    let Ok(dock) = Dock::open(PolePie::from_token(rtc_pie.token())) else {
        exit_with(E_OPEN)
    };
    let view = dock.view();
    let (t0, t1) = (rtc::now(view), rtc::now(view));
    say(&format!("rtc: time {t0} -> {t1}"));

    // 3. 上板（只为让板看得见本域的死）+ 上树：门牌 `/device/rtc` 落在树上（那一枚入口先取出来，
    //    树的 LAND 与组的两只耳朵都要它）。
    let Ok(sire) = utask::sire() else {
        exit_with(E_BOARD)
    };
    let Ok((_link, board_link)) = board::open(sire, MS) else {
        exit_with(E_BOARD)
    };
    if board::ask_hole(board_link).is_err() {
        exit_with(E_BOARD);
    }
    let Ok(entry) = mail::unseal_hole(board::ENTRY_MARK) else {
        exit_with(E_TREE)
    };
    let Ok((link, host)) = operator::open(sire, MS) else {
        exit_with(E_TREE)
    };
    let Ok(talk) = operator::ask_hole(host) else {
        exit_with(E_TREE)
    };
    serve_tree(&link, talk, host, entry);

    // 4. 占线：报**发下来的那一段区**（线号由路由者解树解出来，本域从不说它）。
    let Some(key) = rtc_pie.key() else {
        exit_with(E_LINE)
    };
    let Ok(held) = register(&link, talk, host, key) else {
        exit_with(E_LINE)
    };
    say("rtc: line occupied");

    // 5. 常驻：**一只组等两个源**——门上有请求、线上有投递。
    //
    //    两个源都是**事件**：请求是客人推来的，投递是设备自己拉线换来的。故等待没有期限。
    //    那只组的成员就是那两枚孔（"就绪"挂进组，"取消息"仍走各自那一手）。
    let Ok(tole) = Tole::unseal(false) else {
        exit_with(E_DESK)
    };
    let entry_hole = HolePie::from_token(entry);
    let Ok(lane) = held.hole() else {
        exit_with(E_LINE)
    };
    if tole.attach(&entry_hole, HoleDir::Pull).is_err()
        || tole
            .attach(&HolePie::from_token(lane), HoleDir::Pull)
            .is_err()
    {
        exit_with(E_DESK);
    }

    let mut slot = Slot::new();
    // 一帧请求的上界就是 `ARM` 那一句（`ASK` 更短，也走得进来）。
    let mut buf = [0u8; call::ARM_LEN];
    let mut rang = 0usize;
    loop {
        // 等到有事件。非阻塞地把两个源各取干净——**先门后线**：门上的问要就地答，而线那一趟
        // 到点才有的说（次序不承担语义，只省一次绕回）。
        match tole.await_(usize::MAX) {
            Ok(_) => {}
            Err(_) => exit_with(E_DESK),
        }
        while let Ok((len, from)) = entry_hole.pull_timeout_from(&mut buf, 0) {
            desk(&mut slot, view, from, &buf[..len]);
        }
        while held.receive(0).is_ok() {
            // 一次投递 = 设备那一格拉起来了。顺序与 `uart` 同一条道理：**先把设备那一格清干净**
            // （清 `irq_pending`：电平源，不清线就一直挂着），再看那一格到点没有，最后说"排空了"。
            let now = rtc::now(view);
            rtc::clear(view);
            if let Some(back) = slot.fire(now) {
                match HolePie::from_token(back).push(&call::pack_time(now)) {
                    Ok(()) => {
                        rang += 1;
                        say(&format!("rtc: rang n={rang} now={now}"));
                    }
                    // **推不出去 = 那位客人没了**（它开的那枚孔随它退场封印）。那一格已经空着
                    // （取走就是兑现），故这里只报一行，不重试、不补发。
                    Err(_) => say("rtc: notify failed"),
                }
                let _ = mail::release(back);
            }
            let _ = held.exhaust();
        }
    }
}

/// 门上那一句话：**解帧 → 办事 → 从这一趟自带的那枚孔答回去**。
///
/// 认那枚孔靠 [`scall::find`] 的两格正判据（谁给的 + 记号），多枚时取**最后**那一枚——
/// 客人先交孔、后推帧，故最后那一枚就是这一趟那一枚（见 [`call`] 的次序契约）。
///
/// **拒了的那一趟也要收尾**：那一枚孔不在任何账上（那一格根本没占上），此后没人会替它收
/// ⇒ 答完当场放下。这与线那一刀 `drop_lane` 是同一条纪律、同一个理由。
fn desk(slot: &mut Slot, view: View, from: TaskId, frame: &[u8]) {
    let Some(ask) = call::unpack_ask(frame) else {
        // 不是那个形状：不猜、不动账、也不回话——没有可信的"往哪回"。
        return;
    };
    let Some(back) = scall::find(from, call::BACK) else {
        // 这一趟没把回信孔交进来（或交得不成）：没有可回的路，账一动不动。
        return;
    };
    match ask {
        call::Ask::Now => {
            let now = rtc::now(view);
            let _ = HolePie::from_token(back).push(&call::pack_time(now));
            let _ = mail::release(back);
            say(&format!("rtc: asked now={now}"));
        }
        call::Ask::Arm(at) => {
            let now = rtc::now(view);
            match slot.arm(at, back, now) {
                Ok(()) => {
                    // **设备那一手紧随原语之后**（账记下了，硬件跟上）——与线那一层
                    // "接线是登记的直接后果"同一条分工。
                    rtc::arm(view, at);
                    say(&format!(
                        "rtc: armed at={at} ier={} alarm={}",
                        rtc::irq_enabled(view),
                        rtc::armed(view)
                    ));
                    // 答码**先于**那一声：那一格已经占上，而设备要过一会儿才拉线。
                    let _ = HolePie::from_token(back).push(&[call::OK]);
                }
                Err(fail) => {
                    let _ = HolePie::from_token(back).push(&[call::fail_to_code(Some(fail))]);
                    let _ = mail::release(back);
                }
            }
        }
    }
}

/// 上树那一趟：**分目录 → 落门牌 → 查回来验一遍**（门牌 = 本域那枚服务入口）。
///
/// ```text
///   PART ["device"]           → 0 = 拿到那块目录的号（本域建的 / 已经在了——`part` 幂等）
///   LAND ["device","rtc"]     → 0 = 门牌落上（那枚孔经会话交给持树者）
///   FIND ["device","rtc"]     → 0 = 查得到，且那一枚经会话授回本域表里
///   got                        → 本域在表里认出刚授回来的那一枚了吗
/// ```
///
/// `got` 只是"认出了那一枚"；它指不指得回原物，由**真客人**（`user/sleeper`）证——它照同一条路
/// 找上门、问一句、拿回一个时刻。故本域不自问自答。
fn serve_tree(link: &Quay, talk: PieToken, host: TaskId, entry: PieToken) {
    let (Ok(dir), Ok(me)) = (Name::new(protocol::driver::DIR), Name::new(ME)) else {
        say("rtc: tree: bad name");
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
            false,
            MS,
        ),
        Err(code) => Err(code),
    };
    let (land, pid) = match plate {
        Ok(id) => (ocall::OK, id.get()),
        Err(code) => (code, 0),
    };
    // 查回来验一遍：**按号**（名字只在上面那两格用过，此后一律按号）。
    let find = match plate {
        Ok(id) => operator::find(talk, link, id, MS).unwrap_or(ocall::BAD),
        Err(code) => code,
    };
    let got = operator::take(link, host).is_some();
    // 拿号问名：**号 ↔ 名**这一对对得起来，才算那枚号是真坐标。
    let pname = plate
        .ok()
        .and_then(|id| operator::name(talk, link, id, MS).ok());
    say(&format!(
        "rtc: tree part={part} dir={dir_id} land={land} find={find} got={got} entry={} plate={pid} pname={}",
        entry.get(),
        pname.as_ref().map(|n| n.as_str()).unwrap_or("-"),
    ));
}

/// 三格答码共用的"没走到 / 读不懂"那一格（与树自己的 [`ocall::BAD`] 同值）。
const BAD: u8 = ocall::BAD;

/// 从树上找到线路由者，把本域那条线登记下来。
///
/// 会话是**上面那一条**（同一个域只开一条，见 `driver/uart` 头注）；坐标是**配给回给本域的
/// 那一段区**（本域不写死它）；入口经会话从树上授进来，泊位由 `line` 那一层装。
fn register(
    link: &Quay,
    talk: PieToken,
    host: TaskId,
    key: env::Key,
) -> Result<line::client::Line, ()> {
    let dir = Name::new(protocol::driver::DIR).map_err(|_| ())?;
    let want = Name::new(SERVICE).map_err(|_| ())?;
    let road = [dir, want];
    // **间接寻址那一手**：名字先译成号（号才是树的直接坐标），此后按号。
    let id = operator::seek(talk, link, &road, MS).map_err(|_| ())?;
    let code = operator::find(talk, link, id, MS).unwrap_or(BAD);
    if code != ocall::OK {
        return Err(());
    }
    let entry = operator::take(link, host).ok_or(())?;
    line::client::Line::occupy(entry, key, MS).map_err(|_| ())
}

/// 打一行。调试面是"服务还没起来的嘴"：本域没有控制台，只有它。
fn say(msg: &str) {
    let _ = debug::put(msg);
}
