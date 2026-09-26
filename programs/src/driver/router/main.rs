#![no_std]
#![no_main]

//! router — **线路由者（中断面域）**：外部中断的收与结（**U 态**，**一枚线程**；见 `uart` 头注里那一格读数）。
//!
//! **它为什么叫 router**：它管的是**线**（哪条线、谁领走、领完怎么结），不是某一台设备。
//! 控制器自己的寄存器布局与"线怎么从树里解出来"在同目录的 `plic.rs`——**设备语义各带各的**，
//! 那是 [`crate::driver`] 的家族纪律；需求单在 [`needs`]。
//!
//! ```text
//! 收配给（父域按同一张需求单推来记录，**按位次归位**：控制器 / 自描述 / 门铃）
//!   → 读树：本域该用哪个 context、树里指到本控制器的线有哪几条（**名字只为日志**；顺带报"没进来的账"）
//!   → 板上一趟（装上板路、交上问话孔——**只为让板看得见本域的死**，不挂牌子）
//!   → 树上一趟（分出 `/device`、把本域的服务入口落成 `/device/router`、再查回来验一遍）
//!   → 等三个源（一只组）：**铃**（外部中断）、**门上有人**（登记）、**客人的排空**
//!       门上   登记：解树（区 → 线号）→ 占住那一格 → **接上线** → 把排空那条路挂进组
//!       铃     领到一条：**往主人手里投一帧** → 投到了才静音 + complete → 应铃
//!       排空   客人说"我排空了"：那一格回闲 → **把线放回去**
//!   → 每醒一次先**逐客**（`sweep`）：主人没了的那些线——拆线 + 空出格子（探活）
//! ```
//!
//! **起域时一条线都不接**：接线是登记的直接后果——没登记的线根本不进本 context，本域不再
//! 替所有人刹车（今天树里那 10 条里 9 条没主，全接上就是替它们吞中断）。
//!
//! # 为什么是一枚线程（从前是两枚）
//!
//! 本域有三件事要等：**铃**（外部中断）、**门上有人**（登记）、**客人的排空**——一只组
//! （`Pile`）就能同时等这几样，故合并本来就没有机制上的障碍（从前分两枚只是分工上的选择）。
//!
//! **真正把它定下来的是一条类型事实**：`PieToken` 是"**我这张表**里的第几个"⇒ 同一个域里
//! 两枚线程各有一张表、号过不了线。线那一面**两端都得在本线程手里**——客户往门里推，本域
//! 往客户手里推——故入口、账、各家客户的泊位只能同住一张表。此前那一枚待客线程（造入口、
//! 转发副本、按 `owner` 找回信孔）随之整个清掉。
//!
//! # 它是怎么被叫醒的
//!
//! 内核只有一件关于外部中断的知识：**"有外部中断"**。它把这一件事记进一枚**门铃**，
//! 铃响即唤醒等在这枚铃上的域——就是本域。谁在响、是哪条线、该干什么，内核一概不知，
//! 故它只能摇铃，**claim 只能由本域做**：claim 是"领走"，谁领谁欠 `complete`，
//! 内核一领就进了数据面。
//!
//! # 配给怎么到手（"坐标跟着门闩走"）
//!
//! 见 [`crate::driver::assemble`]：本域按会话协议装一条叫 `records` 的泊位，父域按需求单把
//! 「坐标 + 号」的记录推进来，本域**按位次归位**（单子第 i 条就是回单第 i 条）。**字节长什么样
//! 不在这里**（那是 [`protocol::system::grant`]，与父域同一份）；本域只说"我要哪几格"。
//!
//! # 门牌挂树上，板只管生死
//!
//! 两台目录原本都挂着本域的名字，今天分家了（用户裁定）：**按名找服务走树**（本域是
//! `/device/router`，名字用**服务名**，见 [`protocol::driver::DIR`]），**板**留着看生死——
//! 编排域监督的事件源就是板那条死亡道。故本域**不再向板挂牌**，只装板路 + 交问话孔。
//!
//! # 投递与排空：两件都由事件推动
//!
//! 本域按 [`protocol::driver::line`] 那四个原语办事：**登记**（解树 + 占格 + 接线）、
//! **投递**（往主人手里推一帧）、**排空**（那一格回闲 + 放回）、**收线**（拆线 + 空出格子）。
//! 客户是**持有那台设备的人**（今天 `uart`）——它占线、它收投递、**它排空设备**（读口在它手里，
//! 见 `programs/src/driver/uart/main.rs`）；**线号只在解树那一处产生**：客户从不报它，投递与
//! 排空那两帧里也没有它（泊位就是坐标，见 [`lcall`]）。
//!
//! **排空是一个事件**（`exhaust`）：客人往它那条泊位的另一半写一句"我排空了"，那条路在登记
//! 时就挂进了本域这只组（[`desk_face`]）⇒ 本域被叫醒、`drain_exhaust` 取干净、把线放回去。
//! 旧 `IRQ_WAIT_MS` 那一拍曾兼着"客户端说不出排空"的替身；今天整条等待是**纯事件**的
//! （无期限，见 `main` 里那一注）——排空由上面那条路说，铃由内核响。
//!
//! **逐客也是那一次醒来的一手**（[`sweep`]）：主人一没，它铸的那一枚孔就封印，而那一格正挂在
//! 本域这只组上 ⇒ **醒来本身就是通知**。故"收线"那一格不靠板、不靠一拍，靠这只组。
//! **读数**：`harness/src/lodger`（房客）每次冷启动都占住 1 号线、一句话不说就走 ⇒
//! `router: line 1 = virtio_mmio@10001000` 与 `router: vacate line=1`（两道门都当固定读数）。
//!
//! **两个方向的堵法不对称**（实测定下来的）：投递**阻塞**（客户总会回到收投递那一格），
//! "我排空了"**不阻塞**（它是幂等的状态通知，见 `line::client::Line::exhaust`）——两边都阻塞
//! 就会各自堵在"往对方的单槽里推"上，谁也回不去取自己那一格。
//!
//! # 读数
//!
//! - 起域那一行：`device_count` / `ctx` / 树里那些线（**带名字**）/ **没进来的五笔**；
//! - `router: line <n> = <设备名>`——**登记那一趟**（解树解出来的权威，只由登记产生）；
//! - 每条线**第一次**被领到时一行 `router: line=<n>`——**只可能由中断链产生**
//!   （串口驱动开闸 ⇒ 设备拉线 ⇒ 控制器 ⇒ 内核摇铃 ⇒ 本域 claim）；
//! - `router: exhaust line=<n>`——**排空那一趟**：只可能由客户说"我排空了"产生，而这一句
//!   是它**真的读走了设备里的字节**之后才说的（读口在它手里）。线放回因此是有据的；
//! - `router: vacate line=<n>`——**逐客那一手**：只可能由"客人没了"产生（探活答不出），
//!   故它出现一次就是一条线真的被收掉了；
//! - `router: lane dropped line=<n>`——**登记被拒那一趟**（"这条线有人了"）：这一趟刚交上来的
//!   泊位被放回去了（房客那趟 `TAKEN` 每次冷启动走一遍）；
//! - 账的格数按 `device_count` 要（备不下就拒起，见 [`fail::Fail::Account`]）——账够不够用是**装配期的判据**，
//!   不是运行期的分支。
//!
//! 本域**不读走设备里的字节**：`serial@10000000` 的持有者是 [`crate::driver::uart`]。不读 ⇒
//! 每条线在"投出去了、还没排空"这段时间里源头一直挂着电平，故每领一条就把它**静音**
//! （`priority = 0`），等客人那一声"我排空了"再把线放回去。按线静音本来就是驱动域自己的
//! 细杠杆。
//!
//! **这一格已经裁过**（"静音还是压着不结"）：另一条路（不静音、把 `complete` 押到客户排空）
//! 也防得住白叫醒，但**结是那一格的再武装**，押着的代价是"客户中途死"那条路上的一条永久死线
//! ——两条都量过，读数与裁法见 `protocol::driver::line::mod` 那一节。
//!
//! **照实记**：投递投不出去（客户的口封了）时**不静音**——那一格在**同一次或下一次醒来**被
//! [`sweep`] 收掉（探活答不出 ⇒ `vacate` + 拆线），`router: deliver failed line=` 是它的读数。

extern crate alloc;
// 本包 lib 提供 `_start` + panic_handler；必须真的链接它，`use` 只带符号不算。
extern crate programs;

// 客侧装配与需求单都住在驱动这一族里：`assemble` 是三台驱动与房客共用的那段机器（会话 + 配给）。
use env::Wait;
use env::Mark;
use programs::driver::assemble;
use programs::driver::router::needs;

// 板：本域是**客侧**（装板路、交问话孔——**只为让板看得见本域的死**；名字不挂这里）。
use contract::message::Message;
use protocol::system::board::client as board;
// 树：本域也是**客侧**（门牌挂 `/device/router`，见文件头）。
use protocol::system::operator::Where;
use protocol::system::operator as ocall;
use protocol::system::operator::client as operator;

/// 设备侧（本域私有，同 `lib.rs` 的纪律：谁的设备谁自己带）。
mod plic;

/// 本域的死法（编号 + 那句话）——见那个文件与 `programs::Exit`。
mod fail;

use cases::Suite;
use env::{HoleDir, Name, PieToken, TaskId};
use protocol::driver::line::{core::Lines, frame as lcall};
use protocol::session::call as scall;
use protocol::session::{Pier, Quay};
use runtime::core::bell::Bell;
use runtime::core::dock::Dock;
use runtime::core::pile::Pile;
use runtime::env::debug;
use runtime::env::mail;
use runtime::env::mail::{HolePie, NolePie, PolePie};
use runtime::env::unit as utask;
use runtime::PAGE_SIZE;

use crate::plic::{LINE_PRIORITY, Plic, Sources};

/// 本域挂在树上的名字（`/device/router`，[`protocol::driver::DIR`] 之下的那一段）。
const SERVICE: &str = "router";

/// 装泊位 / 等配给 / 办一趟登记的期限（毫秒）。
const QUAY_MS: usize = 1000;

/// 本域那一台：**返回类型就是它的死法**——编号与那句话都在 [`fail::Fail`] 里
/// （装配那三步 `1`–`3` 由 [`assemble`] 那一族共用，本域自己那几格从 4 起）。
#[programs::entry]
fn main() -> Result<(), fail::Fail> {
    // 客侧装配：会话 + 收配给（**编号原样带出去**——`assemble` 报的是"死在装配的哪一步"，
    // 折成同一个号就等于把那几个编号变成没人读得到的死码）。
    let mut slots = [None; needs::WANTS.len()];
    let got = assemble::receive(&mut slots)?;
    // 三枚都要在：少一枚就不必继续（父域按同一张单子发货，缺格即装配错）。
    let [Some(plic_pie), Some(dtb_pie), Some(bell_pie)] = slots else {
        return Err(fail::Fail::Assemble(assemble::E_GRANT));
    };
    say(&alloc::format!("router: got {got}"));

    // 开图 + 读树：控制器、本域的 context、要接的线（与"没进来的账"）。
    let plic_dock = Dock::open(PolePie::from_token(plic_pie.token())).map_err(|_| fail::Fail::Open)?;
    let dtb_dock = Dock::open(PolePie::from_token(dtb_pie.token())).map_err(|_| fail::Fail::Open)?;
    let (plic, sources) = Plic::new(plic_dock.view(), dtb_dock.view()).ok_or(fail::Fail::Tree)?;
    say("router: docks open");
    // 线集合与五笔"没进来的账"——这台机器上有哪些中断源，唯一一次陈述。
    say(&alloc::format!(
        "router: device_count={} ctx={} lines={:?} unparented={} beyond={} mapped={} unparsed={} unregion={}",
        plic.device_count(),
        plic.context(),
        sources
            .lines
            .iter()
            .map(|s| s.line)
            .collect::<alloc::vec::Vec<u32>>(),
        sources.unparented,
        sources.beyond,
        sources.mapped,
        sources.unparsed,
        sources.unregion
    ));
    let bell = Bell::new(NolePie::from_token(bell_pie.token()));

    // 账：格数按控制器自报的线数要，装不下 ⇒ 拒起（"领到的线一定记得下"是构造性事实）。
    // **起域时一条都不接**：接线是登记的直接后果（见文件头）。
    let mut lines = Lines::new(plic.device_count()).ok_or(fail::Fail::Account)?;

    // 服务入口：本线程铸、本线程读——**它就是树上那块门牌**。
    //
    // 线那一面（账 + 各家客户的泊位）与入口同住这一张表：`PieToken` 只在铸它的那张表里
    // 念得出来，而客户往门里推、路由者往客户手里推——两端都得在同一张表里，故这里不再有
    // 第二枚线程。
    let entry = mail::unseal_hole(board::ENTRY_MARK).map_err(|_| fail::Fail::Desk)?;

    // 板那趟（装上板路、交上问话孔——只为让板看得见本域的死）+ 上树那趟（门牌）。
    let sire = utask::sire()?;
    serve_board(sire, entry);

    // 等三个源：**铃**（外部中断）、**门上有人**（登记）、**客人的排空**（每登记一条线
    // 就把那位客户的泊位挂进来，见 `desk_face`）。一只组同时等这三样——三件都是事件，
    // 故等待**没有期限**（见 `main` 里那一注）：会丢的那一次铃已在根上修掉。
    let pile = Pile::unseal(false).map_err(|_| fail::Fail::Bell)?;
    let entry_hole = HolePie::from_token(entry);
    if pile
        .attach(&NolePie::from_token(bell_pie.token()), HoleDir::Pull)
        .is_err()
        || pile.attach(&entry_hole, HoleDir::Pull).is_err()
    {
        return Err(fail::Fail::Bell);
    }

    // 一问的形状是 `lcall::Occupy::LEN`；缓冲给**一页**（载体的界，见 `Push` 的前置条件）。
    let mut buf: alloc::vec::Vec<u8> = alloc::vec::Vec::new();
    if buf.try_reserve_exact(PAGE_SIZE).is_err() {
        return Err(fail::Fail::Desk);
    }
    buf.resize(PAGE_SIZE, 0);
    loop {
        // **等到有事件**：三样（铃 / 门上有人 / 客人的排空）都可等地，醒来就说明有一格有事。
        //
        // **纯事件（`Wait::Forever`），没有兜底的一拍**：旧写法带 20 ms 期限，为的是盖住"偶尔
        // 一次组等待没被叫醒"（实测：PLIC 的 `pending` 置着、本域不再被叫醒，字节留在设备里）。
        // 那一格的根在铃那一侧——空闲核不进外部 trap，`raise_irq` 在它身上没有调用点，铃
        // 根本没响（见 `kernel/src/work/room/scheduler/core/fetch.rs` 的空闲循环）。根修在
        // 那里，`SEIP` 能挂的那两条长驻态各有振铃点之后这一拍就是多余的：铃一定响，
        // 醒来 `claim`+`hush` 即到。
        match pile.await_(Wait::Forever) {
            Ok(Some(_)) => {}
            // 挂起过（不是期限）：照样往下走一遍——`claim` 领到空就什么也不做。
            Ok(None) => {}
            // 组坏了 ⇒ 本域也没事可做（铃那一格今天不可达：它的资源实体由内核**永久持有**，
            // `platform/devices.rs::IRQ`——它是一格防御，不是读数）。
            Err(_) => return Err(fail::Fail::Bell),
        }
        // 逐客：**每次醒来扫一遍有主的那些条**——主人没了就拆线 + 空出格子。放在最前：
        // 那一格收掉之后再取排空、再登记，账里就只剩还活着的客人。
        sweep(&mut lines, &plic, &pile);
        // 排空：客人说一句"这一条我排空了" ⇒ 那一格回闲 + **把线放回去**（事件，不是节拍）。
        // **先取排空，再登记**：登记会把新的一条线接上，紧接着到来的那一枚中断才不漏。
        drain_exhaust(&mut lines, &plic, &mut buf);
        // 门上：非阻塞地把槽里的都取走（登记）。缓冲是**一页**（载体的界，见 `Push` 的前置
        // 条件）——于是任何一条消息一趟都取得出来，"取不出也丢不掉"那个状态不存在。
        while let Ok((n, from)) = entry_hole.pull_timeout_from(&mut buf, Wait::POLL) {
            desk_face(&mut lines, &plic, &sources, from, &buf[..n], &pile);
        }
        // 铃：**领到空**——不按 `bell.wait(0)` 的返回值判。那一位是**中断闸门的账**
        // （响着 ⇒ 本 hart 的 `SEIE` 关着），而组那一次等待会与它互相消费 ⇒ 按返回值
        // 进门就会漏掉"响过、但组先把它取走了"的那一次，**闸门从此关着，一枚中断都不再来**
        // （实测：字节留在设备里、PLIC 的 `pending` 一直置着，而本域睡到天荒地老）。
        // 故这里直接领：领到空就什么也不做，领到就投递 + 结，最后**无条件** `hush`。
        loop {
            let line = plic.claim();
            if line == 0 {
                break;
            }
            // **静音只在"那一帧真的送到了"之后**：投不出去（客户的口封了）就不静音
            // ——静音是"这一条有人接了"，而没人接的那一条不该由本域替它按下。
            // **照实记**：这一格由下一次/同一次的 `sweep` 收掉（探活答不出 ⇒ `vacate`）；
            // 报一行让它看得见。
            if lines.deliver(line, &[lcall::NOTE]).is_ok() {
                plic.disable(line);
            } else {
                say(&alloc::format!("router: deliver failed line={line}"));
            }
            // 这条线的**第一次**：打一行只可能由中断链产生的读数（见文件头）。
            //
            // **行首先补一个换行**：这一行是**兜底**——根因（一条读数行要 2~5 次 ecall、
            // 窗口就在两次之间）已经在 `kernel/src/console.rs::_write` 收掉了（一行攒起来、
            // **一次 ecall 发**；照实记与实测都在那一处：13876 行 0 例）。留这个 `\n` 是防
            // **残留那一档**：两颗核同时进 M 模式写同一个 UART —— 它只保证本行从行首开始，
            // 不保证中间不被打断。
            // （**照实记**：当初实测到的是 `pingplic: irq line=10`，本域改名后即
            // `pingrouter: line=10`——那时"正文 + 换行"确实是**两次**写，不补就与别人粘住。）
            // **报过没有**那一格归账（[`Lines::told`]）——从前是这里另开的一本定长账，容量
            // 与账不联动、越界是裸下标（> 127 条线的控制器上当场 panic）。**一线一次，退场不清**。
            if lines.told(line) {
                say(&alloc::format!("\nrouter: line={line}"));
            }
            plic.complete(line);
        }
        // 应铃：清掉那一位并让内核**立即**重开本 hart 的闸门。**无条件**做——
        // 没响时它答 `Busy`（幂等），而少做一次就是闸门永久关着。
        let _ = bell.hush();
    }
}

/// 排空那一件事：客人说一句"这一条我排空了" ⇒ 那一格回闲 + 把那条线放回去。
///
/// **按泊位认线**：一条线一枚泊位，谁推的那一枚就是哪一条——**帧里没有线号**（1 字节记号，
/// 见 [`lcall`]），故这里无需对账，取到就是那一条。
///
/// 非阻塞取干净再回去等：`pull(.., 0)` 期限内没有就是没有，**不是错误**。
///
/// `buf` = **门外那一页**（调用方那只，见 `main` 的常驻循环）：道上的记号虽然只有 1 字节，
/// 缓冲仍按**载体**的界备——一枚更长的推落进道里时，1 字节的读法取不出也丢不掉，这一条线
/// 就永远回不了闲（`exhaust` 那一句再也不会跑，线也不会重开）。
fn drain_exhaust(lines: &mut Lines, plic: &Plic, buf: &mut [u8]) {
    // **取"忙"的那些**（不是"有主"的那些）：只有"投出去过、还没回闲"的那一格才欠一句
    // 排空；这一句也是那个 `忙` 的唯一读者——账上那一格因此不是写给别人看的。
    let busy: alloc::vec::Vec<u32> = lines.busy().collect();
    for line in busy {
        let Some(lane) = lines.lane(line) else {
            continue;
        };
        while lane.pull(buf, Wait::POLL).is_ok() {
            let _ = lines.exhaust(line);
            plic.enable(line, LINE_PRIORITY);
            say(&alloc::format!("router: exhaust line={line}"));
        }
    }
}

/// 逐客：`alive` 答不出的那几条线——**拆线 + 空出格子**（`vacate` 那一手，连它的两个后果）。
///
/// 时机是**每一次醒**（组那一次等待回来就扫一遍）：主人一没，它铸的那一枚孔就封印，而那一格
/// 正挂在本域这只组上（`seal` 走 `wipe` 敲到组键）⇒ 那一次敲键就是把本域叫起来的那一件事。
/// 故"收线"不靠板、也不靠一拍。
///
/// **这一跳有读数了**：`harness/src/lodger`（房客）每次冷启动都占住 1 号线、然后一句话不说就走
/// ⇒ 本域被叫醒、`alive` 答不出 ⇒ `router: vacate line=1`（两道门的固定读数，见
/// `crates/gate/src/soak.rs`）。链条本身是 `cull::seal_owned` → `messenger::wipe` → 组键。
///
/// 两个后果缺一不可：不 `unwire` 则线还在本 context 里（电平挂着 ⇒ 白报），不 `detach` 则
/// 那一格永远留在组里（对端没了 ⇒ 每次都当场就绪）。
fn sweep(lines: &mut Lines, plic: &Plic, pile: &Pile) {
    let held: alloc::vec::Vec<u32> = lines.held().collect();
    for line in held {
        let Some(lane) = lines.lane(line) else {
            continue;
        };
        if alive(&lane) {
            continue;
        }
        plic.unwire(line);
        let _ = pile.detach(&HolePie::from_token(lane.hole()), HoleDir::Pull);
        let _ = lines.vacate(line);
        say(&alloc::format!("router: vacate line={line}"));
    }
}

/// 客人还答得出来吗：**问它铸的那一枚**（`mail::reserve` 走存活闸：封印之后答不出）。
///
/// 问的是对端的写端（`at_peer`）而不是本端读的那一枚：本端那一枚的活命随本域，问它恒活。
fn alive(lane: &Pier) -> bool {
    match lane.at_peer() {
        Some(at_peer) => mail::reserve(at_peer).is_ok(),
        None => false,
    }
}

/// 板那趟 + 上树那趟：**不向板挂牌**（板只管生死），门牌挂树上。
///
/// **它不拦主循环**：两件都是"起来之后"的事——哪一件没成只报一句读数，收与结照旧。
fn serve_board(sire: TaskId, entry: PieToken) {
    // 板那条路：本端装一条、认下生我者那一枚（它再转授给板线程），再交一枚问话孔——
    // 不交的那一位在板账上永远"没挂齐"，板线程会一直退化成 1 ms 节拍。
    let link = board::open(sire, Wait::AtMost(QUAY_MS)).ok();
    let boarded = match &link {
        Some((_, board)) => board::ask_hole(*board).is_ok(),
        None => false,
    };
    if !boarded {
        say("router: board: no link");
    }
    // 上树：本域的门牌 = `/device/router`（名字用服务名，见 [`protocol::driver::DIR`]）。
    tree_trip(sire, entry);
}

/// 门上那一句话：**登记**（带动作码）——报**那一段区** ⇒ **解树**（"线 = 区的函数"，权威只在
/// 这一处）⇒ 占住那一格 + 接上线 ⇒ 回一格状态码。
///
/// 答话推到**客人借过来的那枚回信孔**上（按记号认：那位给的多枚孔靠记号分开）。
/// **照实记**：这一扇门从前还兼着"招呼"（旧形状：一个名字进、一个名字回）——那条路随旧 32
/// 字节形状一起退休了，今天只有登记一种形状（"找人"走树，见 `guest`）。
fn desk_face(
    lines: &mut Lines,
    plic: &Plic,
    sources: &Sources,
    from: TaskId,
    frame: &[u8],
    pile: &Pile,
) {
    if let Some(key) = <lcall::Occupy as Message>::fetch(frame) {
        let code = match sources.line_of(key) {
            // 树里没这条线 ⇒ 那个坐标不是中断源（线挂在设备上，别的形自然落这一支）。
            None => lcall::UNKNOWN,
            Some(src) => {
                let (line, name) = (src.line, src.name);
                match take_lane(from) {
                    // 客户没把泊位交出来（或交不出来）。
                    None => lcall::DENIED,
                    Some((mut quay, lane)) => match lines.occupy(line, lane) {
                        Ok(()) => {
                            // **接线是登记的直接后果。**
                            plic.enable(line, LINE_PRIORITY);
                            // **排空那条路也在这里挂上**：客人往它写一句"我排空了"，本域就被叫醒
                            // ——那一格是**事件**，不是节拍（挂的是本端读的那一枚，见 `drain_exhaust`）。
                            if let Some(lane) = lines.lane(line) {
                                let _ =
                                    pile.attach(&HolePie::from_token(lane.hole()), HoleDir::Pull);
                            }
                            // 名字只为日志：**当场从树里读**（装不下就打 `?`）。
                            say(&alloc::format!(
                                "router: line {line} = {}",
                                name.as_ref().map(|n| n.as_str()).unwrap_or("?")
                            ));
                            lcall::OK
                        }
                        // **拒了就放回去**：这一趟刚交上来的那条泊位不能留在账外（见 `drop_lane`）。
                        Err(fail) => {
                            drop_lane(&mut quay, lane, line);
                            lcall::fail_to_code(Some(fail))
                        }
                    },
                }
            }
        };
        if let Some(back) = scall::find(from, lcall::BACK_MARK) {
            let _ = HolePie::from_token(back).push(&[code]);
            // **答完就放下**：这一枚是这一趟借过来的（一问一答一个往返），它不在本域的账里
            // ——账里根本没有它，此后没人会替它收。不放的话，每有一次登记就在本域表里多留
            // 一枚，直到本域退场；读数就带在 `pies=` 那一格上（见 `drop_lane`）。
            let _ = mail::release(back);
        }
    }
}

/// 认下这位客户交出来的**线泊位**（记号 [`lcall::LANE`]），并把本端那一枚交给它。
///
/// 返那一格要记的泊位（**连码头一起**：收不下时要放回去，见 `drop_lane`）：`post` 往**它**推
/// 投递（客户读的那一枚），`pull` 收**它的**排空。
/// 客户在推登记之前先 `seat`（本端那一枚落在本域表里），故这一步通常当场成——认不到就是
/// 它没交（或交不出来）。
fn take_lane(from: TaskId) -> Option<(Quay, Pier)> {
    let mark = Name::new(lcall::LANE).ok()?;
    let mut quay = Quay::open(from, protocol::session::call::hands());
    quay.seat(mark).ok()?;
    quay.claim(from, Mark::of(lcall::LANE), Wait::AtMost(QUAY_MS)).ok()?;
    // **码头一起交出去**：`Lines` 收不下这条泊位时，得由拿着码头的人把它放回去
    // （只有码头知道那一枚是本端铸的，见 `drop_lane`）。
    let pier = quay.find(mark).copied()?;
    Some((quay, pier))
}

/// 拒绝那一趟的收尾：**把这一趟刚交上来的泊位放下**——本端铸的那一枚（`unseat`：顺手告诉
/// 对端"这条别用了"）+ 刚从它手里认下的那一枚。
///
/// **为什么非做不可**：`Lines::occupy` 拒了，那两枚就**不在任何账上**（账里根本没有这一格），
/// 故没有别人会替它收；`Quay` 也没有 `Drop`（放下一个 `Pier` 值只丢一个号，孔还在本域表里），
/// 于是每失败一次，本域表里就多两枚，直到本域退场。对一个会重试的客户，那就是无界增长。
///
/// **照实记（客户那一侧已经自己收干净了）**：从前客户把本端 `seat` 出去的那一枚与借出去的
/// 回信孔都留在自己表里（它没有 `claim`，接不到 `UNSEAT`），本域也收不了别人的表——那一枚随它
/// 退场清掉。今天 [`Line::occupy`](protocol::driver::line::client::Line::occupy) 自己收（失败
/// 那几趟 `Quay::shut` + 放下回信孔），读数在房客那一行 **`lodger: pies=`** 上（探针量过：
/// 临时关掉那几手，同一处从 `9` 涨到 `14`）。
///
/// 读数带一格 **`pies=`**（本域表里现在有几枚）：'放了没有'这件事因此**可量**——少放一枚，
/// 这一格当场大 1（判据钉在 `crates/gate/src/soak.rs` 里，涨了就是红）。**答完话那一枚回信孔副本**
/// 也走同一条纪律（见 `desk_face` 尾上那一手）。
fn drop_lane(quay: &mut Quay, lane: Pier, line: u32) {
    if let Some(at_peer) = lane.at_peer() {
        let _ = mail::release(at_peer);
    }
    if let Ok(mark) = Name::new(lcall::LANE) {
        quay.unseat(mark);
    }
    say(&alloc::format!(
        "router: lane dropped line={line} pies={}",
        mail::table_size()
    ));
}

/// 树上一趟：**分目录 → 落门牌 → 查回来验一遍**。读数一行四格 + 入口的号。
///
/// ```text
///   PART ["device"]              → 0 = 拿到那块目录的号（本域建的 / 已经在了——`part` 幂等）
///   LAND ["device","router"]     → 0 = 门牌落上（入口经会话交给持树者）
///   FIND ["device","router"]     → 0 = 查得到，且那一枚经会话授回本域表里
///   got                           → 本域在表里认出刚授回来的那一枚了吗
/// ```
///
/// `got` **只是"认出了那一枚"**：它指不指得回原物，由**真客人**（`guest`）那一趟证——它
/// 照同一条路找上门、说一句话、拿回答话。故本域不再自问自答（推读自检早已拆掉）。
///
/// 三格答码用的是树自己的失败域（[`ocall::NONEMPTY`] 是"那块目录已经有人建了"，**不是错误**）。
fn tree_trip(sire: TaskId, entry: PieToken) {
    let Ok((link, host)) = operator::open(sire, Wait::AtMost(QUAY_MS)) else {
        say("router: tree: no lane");
        return;
    };
    let Ok(talk) = operator::ask_hole(host) else {
        say("router: tree: no ask");
        return;
    };
    let (Ok(dir), Ok(me)) = (
        env::Name::new(protocol::driver::DIR),
        env::Name::new(SERVICE),
    ) else {
        say("router: tree: bad name");
        return;
    };
    // **分目录 → 落门牌 → 查回来验一遍**：分与落各自**答出那一格的号**（"号出门"那一手）。
    // **分目录**：`part` 是**幂等**的——那块目录已经在就答它那个号（里面有没有东西不管）。
    let dir_at = operator::part(talk, &link, Where::Root, dir, Wait::AtMost(QUAY_MS));
    let (part, dir_id) = match dir_at {
        Ok(id) => (ocall::OK, id.get()),
        Err(code) => (code, 0),
    };
    // **落门牌**：答的是门牌自己那一格的号。
    let plate = match dir_at {
        Ok(at) => operator::land(
            talk,
            &link,
            host,
            Where::At(at),
            me,
            entry,
            ocall::Rule::Public,
            false,
            Wait::AtMost(QUAY_MS),
        ),
        Err(code) => Err(code),
    };
    let (land, pid) = match plate {
        Ok(id) => (ocall::OK, id.get()),
        Err(code) => (code, 0),
    };
    // 查回来验一遍：**按号**（名字只在上面那两格用过，此后一律按号）。
    let (find, got) = match plate {
        Ok(id) => match operator::find(talk, &link, id, Wait::AtMost(QUAY_MS)) {
            Ok((code, entry)) => (code, entry.is_some()),
            Err(_) => (ocall::BAD, false),
        },
        Err(code) => (code, false),
    };
    // **`got` 换了来路**（乙′）：见 `ocall::Union::Seed` 的照实记。
    // 拿号问名：**号 ↔ 名**这一对对得起来，才算那枚号是真坐标。
    let pname = plate
        .ok()
        .and_then(|id| operator::name(talk, &link, id, Wait::AtMost(QUAY_MS)).ok());
    say(&alloc::format!(
        "router: tree part={part} dir={dir_id} land={land} find={find} got={got} entry={} plate={pid} pname={}",
        entry.get(),
        pname.as_ref().map(|n| n.as_str()).unwrap_or("-"),
    ));
    // **这一趟的判据**（值那几格从门那边搬进来：门只剩"这一行还在不在"）。
    let mut suite = Suite::new("router-tree");
    suite.case("the_device_directory_answered", move || {
        assert_eq!(part, ocall::OK)
    });
    suite.case("the_plate_landed", move || assert_eq!(land, ocall::OK));
    suite.case("the_plate_was_found_by_id", move || {
        assert_eq!(find, ocall::OK)
    });
    suite.case("the_shipped_plate_came_back", move || assert!(got));
    suite.case("the_id_and_the_name_agree", move || {
        assert_eq!(pname.as_ref().map(|n| n.as_str()), Some(SERVICE))
    });
    suite.run();
}

/// 打一行。调试面是"服务还没起来的嘴"：本域没有会话、没有控制台，只有它。
fn say(msg: &str) {
    let _ = debug::put(msg);
}


