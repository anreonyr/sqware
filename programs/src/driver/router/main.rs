#![no_std]
#![no_main]

//! router — **线路由者（中断面域）**：外部中断的收与结（S 态 supervisor 域，**两枚线程**）。
//!
//! **它为什么叫 router**：它管的是**线**（哪条线、谁领走、领完怎么结），不是某一台设备。
//! 控制器自己的寄存器布局与"线怎么从树里解出来"在同目录的 `plic.rs`——**设备语义各带各的**，
//! 那是 [`crate::driver`] 的家族纪律；需求单在 [`needs`]。
//!
//! ```text
//! 收配给（父域按同一张需求单推来记录，按 Slot 归位：控制器 / 自描述 / 门铃）
//!   → 读树：本域该用哪个 context、树里指到本控制器的线有哪几条（**带名字**；顺带报"没进来的账"）
//!   → 板上一趟（装上板路、交上问话孔——**只为让板看得见本域的死**，不挂牌子）
//!   → 树上一趟（分出 `/device`、把本域的服务入口落成 `/device/router`、再查回来验一遍）
//!   → 等两个源（一只组）：**铃**（外部中断）与**门上有人**（登记 / 招呼）
//!       门上   登记：解树（名字 → 线号）→ 占住那一格 → **接上线** → 回一格状态码
//!       铃     领到一条：**往主人手里投一帧** → 静音 + complete → 应铃；到点把忙的放回去
//! ```
//!
//! **起域时一条线都不接**：接线是登记的直接后果——没登记的线根本不进本 context，本域不再
//! 替所有人刹车（今天树里那 10 条里 9 条没主，全接上就是替它们吞中断）。
//!
//! # 为什么是一枚线程（从前是两枚）
//!
//! 本域有两件事要等：**铃**（外部中断）与**门上有人**（登记 / 招呼）——一只组
//! （`Tole`）就能同时等"一枚铃 + 一个方向"，故合并本来就没有机制上的障碍（从前分两枚只是
//! 分工上的选择）。
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
//! # 配给怎么到手（"名字跟着门闩走"）
//!
//! 见 [`crate::driver::assemble`]：本域按会话协议装一条叫 `records` 的泊位，父域按需求单把
//! 「名字 + 句柄」的记录推进来，本域按 [`needs::Slot`] 归位。**字节长什么样不在这里**
//! （那是 [`protocol::system::grant`]，与父域同一份）；本域只说"我要哪几格"。
//!
//! # 门牌挂树上，板只管生死
//!
//! 两台目录原本都挂着本域的名字，今天分家了（用户裁定）：**按名找服务走树**（本域是
//! `/device/router`，名字用**服务名**，见 [`protocol::driver::DIR`]），**板**留着看生死——
//! 编排域监督的事件源就是板那条死亡道。故本域**不再向板挂牌**，只装板路 + 交问话孔。
//!
//! # 投递（已落）与它今天缺的那一半
//!
//! 本域按 [`protocol::driver::line`] 那四个原语办事：**登记**（解树 + 占格 + 接线）、
//! **投递**（往主人手里推一帧）、**排空**（那一格回闲 + 放回）、**收线**（拆线 + 空出格子）。
//! 客户是**持有那台设备的人**（今天 `uart`）——它登记、它收投递，本域从不读线号。
//!
//! **照实记**：`exhaust` 那一句今天**没有真内容**——排空的前提是读走设备里的字节，而读口
//! 仍在**内核的调试面**手里（`uart` 一读就把回显抢了）⇒ 客户端说不出"我排空了"，放回仍靠
//! 那一拍（[`IRQ_WAIT_MS`]）。**那一拍就是排空的替身**；要真排空，得先把读口搬到驱动
//! （控制台那一刀）。
//!
//! # 读数
//!
//! - 起域那一行：`ndev` / `ctx` / 树里那些线（**带名字**）/ **没进来的四笔**；
//! - `router: line <n> = <设备名>`——**登记那一趟**（解树解出来的权威，只由登记产生）；
//! - 每条线**第一次**被领到时一行 `router: line=<n>`——**只可能由中断链产生**
//!   （串口驱动开闸 ⇒ 设备拉线 ⇒ 控制器 ⇒ 内核摇铃 ⇒ 本域 claim）；回显本身走得上轮询，
//!   故没有这一行，中断链断了也没人看得出来；
//! - 账的格数按 `ndev` 要（备不下就拒起，见 [`E_ACCOUNT`]）——账够不够用是**装配期的判据**，
//!   不是运行期的分支。
//!
//! 本域**不读走设备里的字节**：`serial@10000000` 的持有者是 [`crate::driver::uart`]，它也只
//! 开了 `IER.RX`（读口仍在内核的调试面）。不读 ⇒ 源头一直挂着电平，故每领一条线就把它
//! **静音**（`priority = 0`），等铃静下来一拍再把线放回去。按线静音本来就是驱动域自己的
//! 细杠杆；**放回该是一个事件**（客户说"我排空了"），今天没有那个事件，故退回节拍（见
//! [`IRQ_WAIT_MS`]）。

extern crate alloc;
// 本包 lib 提供 `_start` + panic_handler；必须真的链接它，`use` 只带符号不算。
extern crate programs;

// 客侧装配与需求单都住在驱动这一族里：`assemble` 是两台驱动共用的那段机器（会话 + 配给）。
use programs::driver::assemble;
use programs::driver::router::needs;

// 板：本域是**客侧**（装板路、交问话孔——**只为让板看得见本域的死**；名字不挂这里）。
use protocol::system::board::call as bcall;
use protocol::system::board::client as board;
// 树：本域也是**客侧**（门牌挂 `/device/router`，见文件头）。
use protocol::operator::call as ocall;
use protocol::operator::client as operator;

/// 设备侧（本域私有，同 `lib.rs` 的纪律：谁的设备谁自己带）。
mod plic;

use env::{HoleDir, Name, PieToken, TaskId};
use protocol::driver::line::{call as lcall, core::Lines};
use protocol::session::{Pier, Quay};
use runtime::core::bell::Bell;
use runtime::core::dock::Dock;
use runtime::core::tole::Tole;
use runtime::env::debug;
use runtime::env::mail;
use runtime::env::mail::{HolePie, NolePie, PolePie};
use runtime::env::room::exit_with;
use runtime::env::unit as utask;

use crate::plic::{LINE_PRIORITY, Plic, Sources};

/// 本域挂在树上的名字（`/device/router`，[`protocol::driver::DIR`] 之下的那一段）。
const SERVICE: &str = "router";

/// 招呼那一趟借过来的回信孔上刻的记号（`guest` 那一趟用的字面量；登记那一句用的是
/// [`lcall::BACK`]——**同一位给的多枚孔靠记号分开**，不按记号认就会认错）。
const GREET_BACK: &str = "back";

/// 装泊位 / 等配给 / 办一趟登记的期限（毫秒）。
const QUAY_MS: usize = 1000;

/// 手里还压着忙线时等一回的上界（毫秒）。**只有这时才用节拍**——闲的时候本域真睡。
///
/// **这个数没有依据**（照实记）：它只是"别永久挂起"。放回本该是一个事件（"我排空了"），
/// **这一拍就是那个事件的替身**——今天客户端不读设备（读口在内核的调试面），说不出有内容的
/// `exhaust`；要真排空，得先把读口搬到驱动（控制台那一刀）。
const IRQ_WAIT_MS: usize = 20;

/// 失败编号指"死在装配的哪一步"（本域是常驻的，正常退场码不作数）。
///
/// 装配那三步（`1`–`3`）的编号由 [`assemble`] 那一族共用；本域自己那几格从 4 起。
const E_OPEN: usize = 4;
const E_TREE: usize = 5;
const E_BELL: usize = 6;
const E_DESK: usize = 7;
/// 账备不下这台控制器的格子：**拒起**，不是运行期降级。
const E_ACCOUNT: usize = 8;

/// "报过第一次"账：哪几条线已经打过 `router: line=` 那一行（不按每枚中断打，否则日志变脏）。
///
/// **只记这一件事**——"手边还压着哪几条"归 [`Lines`] 里那个"忙"，两处不重叠。
#[derive(Clone, Copy, Default)]
struct Seen([u64; 2]);

impl Seen {
    /// 这条线**是第一次**置吗？（置上并回答）
    fn first(&mut self, line: u32) -> bool {
        let word = &mut self.0[line as usize / 64];
        let bit = 1 << (line % 64);
        let fresh = (*word & bit) == 0;
        *word |= bit;
        fresh
    }
}

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    // 客侧装配：会话 + 收配给（**编号原样带出去**——`assemble` 报的是"死在装配的哪一步"，
    // 折成同一个号就等于把那几个编号变成没人读得到的死码）。
    let mut slots = [None; needs::WANTS.len()];
    let got = match assemble::receive(&mut slots, needs::slot_of) {
        Ok(n) => n,
        Err(code) => exit_with(code),
    };
    // 三枚都要在：少一枚就不必继续（父域按同一张单子发货，缺格即装配错）。
    let [Some(plic_token), Some(dtb_token), Some(bell_token)] = slots else {
        exit_with(assemble::E_GRANT)
    };
    say(&alloc::format!("router: got {got}"));

    // 开图 + 读树：控制器、本域的 context、要接的线（与"没进来的账"）。
    let Ok(plic_dock) = Dock::open(PolePie::from_token(plic_token)) else {
        exit_with(E_OPEN);
    };
    let Ok(dtb_dock) = Dock::open(PolePie::from_token(dtb_token)) else {
        exit_with(E_OPEN);
    };
    let Some((plic, sources)) = Plic::new(plic_dock.view(), dtb_dock.view()) else {
        exit_with(E_TREE);
    };
    say("router: docks open");
    // 线集合与四笔"没进来的账"——这台机器上有哪些中断源，唯一一次陈述。
    say(&alloc::format!(
        "router: ndev={} ctx={} lines={:?} unparented={} beyond={} mapped={} unparsed={}",
        plic.ndev(),
        plic.context(),
        sources
            .lines
            .iter()
            .map(|s| s.line)
            .collect::<alloc::vec::Vec<u32>>(),
        sources.unparented,
        sources.beyond,
        sources.mapped,
        sources.unparsed
    ));
    let bell = Bell::new(NolePie::from_token(bell_token));

    // 账：格数按控制器自报的线数要，装不下 ⇒ 拒起（"领到的线一定记得下"是构造性事实）。
    // **起域时一条都不接**：接线是登记的直接后果（见文件头）。
    let Some(mut lines) = Lines::new(plic.ndev()) else {
        exit_with(E_ACCOUNT)
    };

    // 服务入口：本线程铸、本线程读——**它就是树上那块门牌**。
    //
    // 线那一面（账 + 各家客户的泊位）与入口同住这一张表：`PieToken` 只在铸它的那张表里
    // 念得出来，而客户往门里推、路由者往客户手里推——两端都得在同一张表里，故这里不再有
    // 第二枚线程。
    let Ok(entry) = mail::unseal_hole(board::ENTRY_MARK) else {
        exit_with(E_DESK)
    };
    let Ok(our) = Name::new(SERVICE) else {
        exit_with(E_DESK)
    };

    // 板那趟（装上板路、交上问话孔——只为让板看得见本域的死）+ 上树那趟（门牌）。
    let Ok(sire) = utask::sire() else {
        exit_with(assemble::E_SIRE)
    };
    serve_board(sire, entry);

    // 等两个源：**铃**（外部中断）与**门上有人**（登记 / 招呼）。一只组同时等"一枚铃 +
    // 一个方向"；`await_` 的 `None` = 这一轮挂起过（期限到）——**那一拍就是"排空"的替身**，
    // 见 [`IRQ_WAIT_MS`]。
    let Ok(tole) = Tole::unseal(false) else {
        exit_with(E_BELL)
    };
    let entry_hole = HolePie::from_token(entry);
    if tole
        .hang(&NolePie::from_token(bell_token), HoleDir::Pull)
        .is_err()
        || tole.hang(&entry_hole, HoleDir::Pull).is_err()
    {
        exit_with(E_BELL);
    }

    let mut seen = Seen::default();
    let mut buf = [0u8; lcall::ASK];
    loop {
        // **闲的时候真睡**：手里没有忙线时 `usize::MAX`（永久挂起）——不会漏，铃与门都会叫醒。
        let wait = if lines.busy().next().is_none() {
            usize::MAX
        } else {
            IRQ_WAIT_MS
        };
        match tole.await_(wait) {
            // **到点**（这一轮挂起过）：把忙的那些放回去——**那一拍就是排空的替身**。
            Ok(None) => {
                let busy: alloc::vec::Vec<u32> = lines.busy().collect();
                for line in busy {
                    let _ = lines.exhaust(line);
                    plic.enable(line, LINE_PRIORITY);
                }
            }
            Ok(Some(_)) => {}
            // 组坏了 ⇒ 本域也没事可做（铃那一格今天不可达：它的资源实体由内核**永久持有**，
            // `platform/devices.rs::IRQ`——它是一格防御，不是读数）。
            Err(_) => exit_with(E_BELL),
        }
        // 门上：非阻塞地把槽里的都取走（登记 / 招呼）。**取干净再去看铃**——登记可能就在
        // 这一轮把一条线接上，接着到来的那一枚中断因而不会漏。
        while let Ok((n, from)) = entry_hole.pull_timeout_from(&mut buf, 0) {
            desk_face(&mut lines, &plic, &sources, our, from, &buf[..n]);
        }
        // 铃：领到空。领到的那几条**先往主人手里投一帧**，再静音 + 结（那一帧没送到就不算忙）。
        if matches!(bell.wait(0), Ok(true)) {
            loop {
                let line = plic.claim();
                if line == 0 {
                    break;
                }
                let _ = lines.deliver(line, &lcall::pack_line(line));
                plic.disable(line);
                // 这条线的**第一次**：打一行只可能由中断链产生的读数（见文件头）。
                //
                // **行首先补一个换行**：调试面是**共用**的（echo / 板 / 编排域都在写），
                // 而内核那一格的一次 `put` 是"正文 + 换行"**两次**写（`putln!` 展开成
                // `format_args!("{}\n", ..)`）⇒ 不补换行就可能跟别人正写到一半的那一行
                // 粘住（实测：`pingplic: irq line=10`，本域改名后即 `pingrouter: line=10`）。
                if seen.first(line) {
                    say(&alloc::format!("\nrouter: line={line}"));
                }
                plic.complete(line);
            }
            // 应铃：清掉那一位并让内核**立即**重开本 hart 的闸门。
            let _ = bell.hush();
        }
    }
}

/// 板那趟 + 上树那趟：**不向板挂牌**（板只管生死），门牌挂树上。
///
/// **它不拦主循环**：两件都是"起来之后"的事——哪一件没成只报一句读数，收与结照旧。
fn serve_board(sire: TaskId, entry: PieToken) {
    // 板那条路：本端装一条、认下生我者那一枚（它再转授给板线程），再交一枚问话孔——
    // 不交的那一位在板账上永远"没挂齐"，板线程会一直退化成 1 ms 节拍。
    let link = board::open(sire, QUAY_MS).ok();
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

/// 门上的两种话。
///
/// - **登记**（带动作码）：报一个设备名 ⇒ **解树**（"线 = 名字的函数"，权威只在这一处）⇒
///   占住那一格 + 接上线 ⇒ 回一格状态码；
/// - **招呼**（旧形状：一个名字）：回自己的名字——`guest` 那一趟还在用它（"还没有协议"时代的
///   遗留；**同一扇门上靠帧长分开**：32 = 招呼，33 = 登记）。
///
/// 答话都推到**客人借过来的那枚回信孔**上（按记号认：两条路各刻各的记号，那位给的多枚孔
/// 才分得开）。
fn desk_face(
    lines: &mut Lines,
    plic: &Plic,
    sources: &Sources,
    our: Name,
    from: TaskId,
    frame: &[u8],
) {
    if let Some(device) = lcall::unpack_reserve(frame) {
        let code = match sources.line_of(device) {
            // 树里没这条线 ⇒ 那个名字不是中断源。
            None => lcall::UNKNOWN,
            Some(line) => match take_lane(from) {
                // 客户没把泊位交出来（或交不出来）。
                None => lcall::DENIED,
                Some(lane) => match lines.reserve(line, lane) {
                    Ok(()) => {
                        // **接线是登记的直接后果。**
                        plic.enable(line, LINE_PRIORITY);
                        say(&alloc::format!("router: line {line} = {}", device.as_str()));
                        lcall::OK
                    }
                    Err(fail) => lcall::code_of(fail),
                },
            },
        };
        if let Some(back) = find_mark(from, lcall::BACK) {
            let _ = HolePie::from_token(back).push(&[code]);
        }
        return;
    }
    // 招呼：一个名字进来，本域的名字回去。
    let Some(who) = bcall::name_of(frame) else {
        return;
    };
    say(&alloc::format!("router: desk {}", who.as_str()));
    if let Some(back) = find_mark(from, GREET_BACK) {
        let _ = HolePie::from_token(back).push(our.bytes());
    }
}

/// 认下这位客户交出来的**线泊位**（记号 [`lcall::LANE`]），并把本端那一枚交给它。
///
/// 返那一格要记的泊位：`post` 往**它**推投递（客户读的那一枚），`pull` 收**它的**排空。
/// 客户在推登记之前先 `seat`（本端那一枚落在本域表里），故这一步通常当场成——认不到就是
/// 它没交（或交不出来）。
fn take_lane(from: TaskId) -> Option<Pier> {
    let mark = Name::new(lcall::LANE).ok()?;
    let mut quay = Quay::open(from);
    quay.seat(mark).ok()?;
    quay.claim(from, mark, QUAY_MS).ok()?;
    quay.find(mark).copied()
}

/// 本端表里**这位给的、刻着那个记号的那一枚**（答话那条路）。
///
/// 判据两格：`owner == who`（副本共享同一事实、转手不变）+ **记号**——线泊位那两枚也是这位
/// 的，不按记号认就会认错那一枚。
fn find_mark(who: TaskId, mark: &str) -> Option<PieToken> {
    let mut index = 0usize;
    let mut found = None;
    loop {
        let (token, _perm, _vestor) = mail::collect(index).ok()?;
        // 越界哨兵：这一遍扫完了。
        if token.get() == 0 {
            return found;
        }
        index += 1;
        match mail::reserve(token) {
            Ok((_v, owner, m)) if owner == who && m.as_str() == mark => found = Some(token),
            _ => {}
        }
    }
}

/// 树上一趟：**分目录 → 落门牌 → 查回来验一遍**。读数一行四格 + 入口的号。
///
/// ```text
///   PART ["device"]              → 0 = 本域建的；2 = 已经在了（前一台驱动建的）——两个都要
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
    let Ok((link, host)) = operator::open(sire, QUAY_MS) else {
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
    let path = [dir, me];
    let none = PieToken::NONE;
    let part = operator::ask(talk, &link, host, ocall::PART, &[dir], none, QUAY_MS).unwrap_or(BAD);
    let land = operator::ask(talk, &link, host, ocall::LAND, &path, entry, QUAY_MS).unwrap_or(BAD);
    let find = operator::ask(talk, &link, host, ocall::FIND, &path, none, QUAY_MS).unwrap_or(BAD);
    let got = operator::take(&link, host).is_some();
    say(&alloc::format!(
        "router: tree part={part} land={land} find={find} got={got} entry={}",
        entry.get()
    ));
}

/// 三格答码共用的"没走到 / 读不懂"那一格（与树自己的 [`ocall::BAD`] 同值）。
const BAD: u8 = ocall::BAD;

/// 打一行。调试面是"服务还没起来的嘴"：本域没有会话、没有控制台，只有它。
fn say(msg: &str) {
    let _ = debug::put(msg);
}
