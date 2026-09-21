#![no_std]
#![no_main]

//! plic — **中断面域**：外部中断的收与结（S 态 supervisor 域，**两枚线程**）。
//!
//! ```text
//! 装会话（交一枚孔给父域，父域按名字认领）
//!   → 收配给（父域按同一张需求单推来记录，按 Slot 归位）
//!   → 板上一趟（挂上本域的服务入口、查回来验一遍：答话三格 + 入口有没有到手）
//!   → 接上控制器自报的每一条线
//!   → 两枚线程各守一个源：
//!       主线程     等铃 → claim 到空 → 按线静音 + complete → 应铃 → 到点把线放回去
//!       待客线程   守服务入口（有人敲就记一行、把本域的名字答回去）
//! ```
//!
//! # 为什么是两枚线程
//!
//! 本域有两件事要等：**铃**（外部中断）与**服务入口**（有人按名字找上门）。
//! **今天这两件事能等在一处**——组（`Tole`）的成员就收"孔的一个方向 / 一枚铃"两类，
//! 板线程正是用一枚组同时等"提示孔 + 每位客人的问话孔"。故本域分两枚线程是**分工上的
//! 选择**（一枚线程守一个源，一支循环只做一件事），不是"合并不了"。
//!
//! **照实记**：旧注写的是"门铃与孔各有各的唤醒键，合并不了……**一枚线程守一个源**是今天
//! 唯一不退化的办法"——那是 `Tole` 落地之前的实情，`Tole` 一落地就不成立了。
//!
//! # 服务入口为什么由待客线程铸
//!
//! `PieToken` 是"**我这张表**里的第几个"——**同一个域里两枚线程各有一张表，号过不了线**。
//! 客人推上来的那一句得由**守在那扇门上**的那枚线程读走，故入口由它铸、它读；本域主线程
//! 要的是**副本**（拿去挂到板上），那一枚经 `Ship` 过来（同域转授，与跨域同一条路）。
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
//! ```text
//! 1  本域按会话协议装一条叫 records 的泊位 → 那枚孔落到父域表里
//! 2  父域按需求单把要的几样交出来，再把「名字 + 句柄」的记录推进那条通道
//! 3  本域解出记录，按需求单的 Slot 归位
//! ```
//!
//! **字节长什么样不在这里**（那是 [`pairing`]，与父域同一份）；本域只说"我要哪几格"。
//!
//! # 还没有的那一格：投递给客户端
//!
//! 本域做"收与结"：接上所有线、claim、complete、应铃。**没有**"名字 → 线号"的表，也没有
//! 把中断投递给客户端那一段——今天来敲门的只有 `guest`（按名字问一句、本域答一句），
//! 还没有谁是"要这条线的中断"的人。
//!
//! 本域**不读走设备里的字节**：`serial@10000000` 的接收字节归 console。不读 ⇒ 源头一直
//! 挂着电平，故每领一条线就把它**静音**（`priority = 0`），等铃静下来一拍再把线放回去。
//! 按线静音本来就是驱动域自己的细杠杆。

extern crate alloc;
// 本包 lib 提供 `_start` + panic_handler；必须真的链接它，`use` 只带符号不算。
extern crate programs;

// 需求单由**本域自己开**（收方那张账）——它就是 lib 里同一份源码。
use programs::supervisor::plic::needs;
use protocol::system::grant;

// 板：本域是**客侧**（挂牌子、查回来）。
use protocol::system::board::client as board;

/// 设备侧（本域私有，同 `lib.rs` 的纪律：谁的设备谁自己带）。
mod plic;
mod uart;

use env::wire::PAIR_LEN;
use env::{PieToken, TaskId};
use protocol::session::Quay;
use protocol::system::board::call as bcall;
use runtime::core::bell::Bell;
use runtime::core::dock::Dock;
use runtime::core::port::{self, Access, Policy};
use runtime::core::unit::{Join, closure};
use runtime::env::debug;
use runtime::env::mail;
use runtime::env::mail::{NolePie, PolePie};
use runtime::env::room::exit_with;
use runtime::env::unit as utask;

use crate::plic::{LINE_PRIORITY, Plic};

/// 与父域之间那条通道的名字（两端按它对位，不靠位置约定）。
const RECORDS: &str = "records";

/// 本域挂在板上的名字，以及**对照**用的那个"板上没有的名字"。
///
/// 对照那一问是必要的：只报"查到了"而不知道"查不到会怎样"，那一格读数证明不了什么。
const SERVICE: &str = "plic";
const NOBODY: &str = "no-such-name";

/// 与待客线程之间那条路的名字（本域登记它交回来的服务入口时用；它那侧不看名字）。
const DESK_LINK: &str = "plic-entry";

/// 装泊位/等配给的期限（毫秒）。
const QUAY_MS: usize = 1000;

/// 有静音的线时等铃的上界（毫秒）。**只有这时才用节拍**——闲的时候本域真睡（见主循环）。
const IRQ_WAIT_MS: usize = 20;

/// 静音位图能记到第几条线（`u64` 一位一条）。
///
/// 越界的线不记账：当场放回即可（virt 上 `ndev` 是 32，这条够不着）。
const MUTE_BITS: u32 = 64;

/// 收记录用的缓冲容量：需求单几条就备几条（父域按**同一张单子**推）。
///
/// 发货方不必抄这个数（`pairing::pack` 按单子逐条走）；收货方要它，因为它得**备缓冲**。
const CAP: usize = PAIR_LEN * needs::WANTS.len();

/// 失败编号指"死在装配的哪一步"（本域是常驻的，正常退场码不作数）。
const E_SIRE: usize = 1;
const E_UP: usize = 2;
const E_GRANT: usize = 3;
const E_OPEN: usize = 4;
const E_TREE: usize = 5;
const E_BELL: usize = 6;
const E_DESK: usize = 7;

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    // **编号原样带出去**：`boot` 报的是"死在装配的哪一步"，折成同一个号就等于把那几个
    // 编号变成没人读得到的死码（`E_SIRE` / `E_UP` 曾经就是这样）。
    let slots = match boot() {
        Ok(slots) => slots,
        Err(code) => exit_with(code),
    };

    // 开图 + 读树：控制器、本域的 context、要接的线。
    let Ok(plic_dock) = Dock::open(PolePie::from_token(slots[needs::Slot::Plic as usize])) else {
        exit_with(E_OPEN);
    };
    let Ok(dtb_dock) = Dock::open(PolePie::from_token(slots[needs::Slot::Dtb as usize])) else {
        exit_with(E_OPEN);
    };
    let Ok(src_dock) = Dock::open(PolePie::from_token(slots[needs::Slot::Source as usize])) else {
        exit_with(E_OPEN);
    };
    let Some((plic, lines)) = Plic::new(plic_dock.view(), dtb_dock.view()) else {
        exit_with(E_TREE);
    };
    say("plic: docks open");
    let bell = Bell::new(NolePie::from_token(slots[needs::Slot::Bell as usize]));

    for &line in &lines {
        plic.enable(line, LINE_PRIORITY);
    }
    // 测试源：把"收到字节就拉线"打开。**必须在 enable 之后**——控制器先就位，线再开闸。
    // 读走字节的仍是内核的调试面（本域只动中断使能这一位），故敲键不会丢给谁。
    uart::arm_rx(src_dock.view());

    // 循环：等铃 → 领到空 → 逐条静音 + 结 → 应铃 → 到点把静音的放回去。
    //
    // **闲的时候真睡**：`muted == 0` 时 wait 传 `usize::MAX`（永久挂起），本域一次都不醒。
    // 只有手里还压着静音的线时才退回节拍——那是今天唯一能"放回"的手段（还没有谁会来说
    // 一声"我抽干了"）。把放回换成事件，这个节拍随之消失。
    let mut muted: u64 = 0;
    loop {
        // 铃是一位：闲时（`muted == 0`）即使中断此刻就到，`ring` 也会把本域唤醒——不会漏。
        let wait = if muted == 0 { usize::MAX } else { IRQ_WAIT_MS };
        match bell.wait(wait) {
            // 铃响：把这一轮该领的都领走。
            Ok(true) => {
                loop {
                    let line = plic.claim();
                    if line == 0 {
                        break;
                    }
                    plic.disable(line);
                    if line < MUTE_BITS {
                        muted |= 1 << line;
                    } else {
                        // 记不下就当场放回：宁可让它再报一次，也不能把它忘在静音里。
                        plic.enable(line, LINE_PRIORITY);
                    }
                    plic.complete(line);
                }
                // 应铃：清掉那一位并让内核**立即**重开本 hart 的闸门。
                let _ = bell.hush();
            }
            // **没当场就绪**：把**真被静音过**的那几条放回去、放完清零。这一支**不只有
            // "到点"**——`wait` 的 `false` 是"挂起过"（被铃叫醒与期限到**不分**，内核没有
            // 第二次执行机会），故下一轮可能出现"铃其实响着"。那不会漏：内核那一格**先探**
            // 响位，真响着就当场答 `true`（见 `mail/nole.rs::wait`）——故这里不必分辨。
            Ok(false) => {
                let mut bits = muted;
                muted = 0;
                while bits != 0 {
                    let line = bits.trailing_zeros();
                    bits &= bits - 1;
                    plic.enable(line, LINE_PRIORITY);
                }
            }
            // 铃死了（父域收摊）⇒ 本域也没事可做。
            Err(_) => exit_with(E_BELL),
        }
    }
}

/// 装配的前半：装会话 → 收配给 → 按 `Slot` 归位 → 板上一趟。
///
/// 返那几枚门闩（按需求单的格子），失败给退场码。
fn boot() -> Result<[PieToken; needs::WANTS.len()], usize> {
    // 1. 会话：本域那一侧的孔交给"生我者"——就是建本域的那枚线程（父域的装配者）。
    //    记号 = 这条泊位的名字（`seat` 铸孔时刻上去的）：父域放行之后按 `(本域, records)`
    //    两格把这一枚认下来（`Quay::claim` 的正文），本域**不必报名字**——记号随副本过线。
    let sire = utask::sire().map_err(|_| E_SIRE)?;
    let channel = env::Name::new(RECORDS).map_err(|_| E_UP)?;
    let mut quay = Quay::open(sire);
    quay.seat(channel).map_err(|_| E_UP)?;

    // 2. 收配给：父域按同一张需求单推来记录，**按 `Slot` 归位**（不数第几条）。
    let mut buf = [0u8; CAP];
    let up = quay.find(channel).ok_or(E_UP)?;
    let n = up.pull(&mut buf, QUAY_MS).map_err(|_| E_GRANT)?;
    say(&alloc::format!("plic: got {n}"));
    let mut got: [Option<PieToken>; needs::WANTS.len()] = [None; needs::WANTS.len()];
    grant::unpack(
        &buf[..n],
        |name| needs::slot_of(name),
        |slot, token| {
            if let Some(cell) = got.get_mut(slot) {
                *cell = Some(token);
            }
        },
    );

    // 3. 四枚都要在：少一枚就不必继续（父域按同一张单子发货，缺格即装配错）。
    let mut out = [PieToken::NONE; needs::WANTS.len()];
    for (i, cell) in got.iter().enumerate() {
        out[i] = cell.ok_or(E_GRANT)?;
    }

    // 4. 板那条路：本端装一条、认下生我者那一枚（它再转授给板线程）。
    //    返两样：本端这座码头 + **板线程的号**（板路上先到的那一格）：孔只在铸它的表里
    //    念得出来，故交问话孔、交入口都得先叫得出板是谁。
    let link = board::open(sire, QUAY_MS).ok();

    // 5. 待客线程 + 服务入口：入口由待客线程铸（谁守那扇门谁读它），副本本域登记。
    //    **它不拦装配**：起不来就报一句读数（板是"起来之后"的事，不是起来的条件）。
    let Ok(me) = utask::self_id() else {
        return Err(E_GRANT);
    };
    let entry = start_desk(me);

    // 6. 板上一趟：挂上这一枚入口、再查回来验一遍。
    let (no_link, no_desk) = (link.is_none(), entry.is_none());
    let trip = match (link, entry) {
        (Some((link, board)), Some(entry)) => board_trip(&link, board, entry),
        _ => None,
    };
    match &trip {
        Some(t) => say(&alloc::format!(
            "plic: board reg={} miss={} hit={} entry={} grant={}",
            t.reg,
            t.miss,
            t.hit,
            t.entry.get(),
            t.grant.map(|g| g.get()).unwrap_or(0)
        )),
        None => say(if no_link {
            "plic: board: no link"
        } else if no_desk {
            "plic: board: no desk"
        } else {
            "plic: board: trip"
        }),
    }
    Ok(out)
}

/// 起待客线程，并把**它的服务入口**收回来（本域登记用的那一枚副本）。
///
/// 入口由待客线程铸（`PieToken` 过不了线，见文件头），副本经 `Ship` 交到本线程表里；
/// 本线程另开一座码头认它——判据两格：`owner == 待客线程`（铸的人就是 owner）**且记号是
/// `entry`**（它铸那一刻刻的就是这个用途名；本线程表里同时还有别的外来孔）。
///
/// `seat` 那一步也会把本线程铸的一枚交给它（本协议里"认领"要求本端先装一条）：它不用，
/// 也不碍事。
fn start_desk(me: env::TaskId) -> Option<PieToken> {
    let node: Join<()> = closure(move || desk(me));
    let id = node.id();
    drop(node);
    let link = env::Name::new(DESK_LINK).ok()?;
    let entry = env::Name::new(board::ENTRY_MARK).ok()?;
    let mut quay = Quay::open(id);
    quay.seat(link).ok()?;
    quay.claim(id, entry, QUAY_MS).ok()?;
    let pier = quay.find(link)?;
    pier.at_peer()
}

/// 待客：服务入口上有人说话，就记一行、把本域的名字答回去。
///
/// **一枚线程守一个源**（见文件头）：本线程只等这一枚孔，故 `usize::MAX` = 真挂起。
/// 一问一答各 32 字节（一个名字，与牌子同一个解码面）；**两个方向各一枚孔**——问从入口
/// 读（本线程铸的那一枚），答推到**客人借过来的那枚回信孔**（记号 `back`，按 `owner` 认，
/// 见 [`opened_for`]）。单槽的孔只够一个方向：同一枚上"我推了再读"读到的是自己那一句
/// （session 正文事实 2）。
fn desk(me: env::TaskId) -> ! {
    // 服务入口：**本线程铸、本线程读**（`PieToken` 过不了线，见文件头）。记号 `entry`：
    // 本域主线程认它（它那张表里同时还有别的外来孔），板也按它把入口与问话孔分开。
    let Ok(entry) = mail::unseal_hole(board::ENTRY_MARK) else {
        exit_with(E_DESK)
    };
    // 副本交给本域主线程：它拿去挂到板上——别人按名字找到的就是这一扇门。
    let said = port::ship(
        &mail::HolePie::from_token(entry),
        me,
        Access::FETCH | Access::STORE,
        Policy::VEST,
    );
    if said.is_err() {
        exit_with(E_DESK)
    }
    let hole = mail::HolePie::from_token(entry);
    let Ok(our) = env::Name::new(SERVICE) else {
        exit_with(E_DESK)
    };
    loop {
        let mut buf = [0u8; env::wire::NAME_LEN];
        // **一并取回发送者**：答话要推到"这位客人借给我的那一枚回信孔"上，而"是谁"
        // 由内核在推的那一刻盖章（报文里没有来源字段，也不必有）。
        let Ok((n, from)) = hole.pull_timeout_from(&mut buf, usize::MAX) else {
            exit_with(E_DESK)
        };
        // 长度不对 = 对面送来的不是一个名字：照说一句、照答一句（不猜内容）。
        let who = (n == env::wire::NAME_LEN)
            .then(|| env::Name::from_bytes(buf).ok())
            .flatten();
        let who = who.as_ref().map(env::Name::as_str).unwrap_or("?");
        say(&alloc::format!("plic: desk {who}"));
        // 回信孔：本端表里 `owner` 是这位客人的那一枚（副本共享 owner、转手不变）。
        let Some(back) = opened_for(from) else {
            continue;
        };
        if mail::HolePie::from_token(back).push(our.bytes()).is_err() {
            exit_with(E_DESK)
        }
    }
}

/// 本端表里**这位的那一枚孔**（客人借过来的回信孔）：`owner == who`，取最后登记的那一枚。
///
/// 判据落在 `owner` 上（副本共享同一事实、转手不变）：本端自己铸的每一枚 owner 都是本端，
/// 客人交进来的那一枚 owner 是客人——**编号比不出来，这一格比得出来**。
///
/// 记号那一格在这里**故意不读**：客人借过来的那一枚刻的是 `back`（它那条路的名字），
/// 而这一处要的是"这位的、能收话的那一枚"——问不到记号（不是孔）的候选自然不成立。
fn opened_for(who: env::TaskId) -> Option<PieToken> {
    let mut index = 0usize;
    let mut found = None;
    loop {
        let (token, _perm, _vestor) = mail::collect(index).ok()?;
        // 越界哨兵：这一遍扫完了。
        if token.get() == 0 {
            return found;
        }
        index += 1;
        if mail::reserve(token).ok().map(|(_v, owner, _mark)| owner) == Some(who) {
            found = Some(token);
        }
    }
}

/// 板上一趟的读数（`None` = 路都没装上）。
///
/// ```text
///   entry  本域的服务入口（自己铸的那一枚）          —— 谁想跟本域说话就往它推
///   reg    REGISTER "plic" 的答话码                  —— 0 = 板收下了
///   miss   LOOKUP   "no-such-name" 的答话码          —— 该是 1（UNKNOWN）
///   hit    LOOKUP   "plic" 的答话码                  —— 0 = 查到并把入口授了回来
///   grant  板经会话授进来的那一枚（`board::take` 的最后一枚）
/// ```
///
/// 四个码各是一格，缺一格这句话就证明不了什么：没有 `miss`，"查到了"是空话；没有 `hit`，
/// REGISTER 成没成也没对照。**`grant` 只是"授进来的是哪一枚"这个号**——"它指回原物"
/// 这件事在自检拆掉之后由**真客人**（`guest`）那一趟证（见 [`board_trip`]）。
struct BoardTrip {
    entry: PieToken,
    reg: u8,
    miss: u8,
    hit: u8,
    grant: Option<PieToken>,
}

/// 板上那一趟：**挂上自己那一枚入口，再查回来验一遍**。返一格读数（`None` = 路都没装上）。
///
/// ```text
///   REGISTER "plic"          → 板上挂了本域的服务入口（入口经会话交给板）
///   LOOKUP   "no-such-name"  → 板上没有的名字必须答 UNKNOWN（否则"查到了"不值钱）
///   LOOKUP   "plic"          → 查回来一枚入口（板经会话授进本域表里）
/// ```
///
/// 第四格是**板授进来的那一枚**（客侧按 `vestor` 认，见 `board::take`）——`None` = 板答了
/// "查到了"却没把入口交进来。**它只是"那一枚的号"**：指不指得回原物，自检拆掉之后由**真
/// 客人**那一趟证（下一段）。
///
/// **推读自检已经拆掉**：本域的服务入口由待客线程读（两个方向各一枚孔），本线程推一句
/// 进去只会被它读走——"授进来的指回原物"这件事改由**真客人**（`guest`）走一遍，那比自问
/// 自答结实。
fn board_trip(link: &Quay, board: TaskId, entry: PieToken) -> Option<BoardTrip> {
    // 问话孔：本端铸、给板读（本端自窄到只写）；答话仍走这条板路。
    let talk = board::ask_hole(board).ok()?;
    let name = env::Name::new(SERVICE).ok()?;
    let absent = env::Name::new(NOBODY).ok()?;
    let none = PieToken::NONE;
    let reg = board::ask(talk, link, board, bcall::REGISTER, name, entry, QUAY_MS).ok()?;
    let miss = board::ask(talk, link, board, bcall::LOOKUP, absent, none, QUAY_MS).ok()?;
    let hit = board::ask(talk, link, board, bcall::LOOKUP, name, none, QUAY_MS).ok()?;
    let grant = board::take(link, board);
    Some(BoardTrip {
        entry,
        reg,
        miss,
        hit,
        grant,
    })
}

/// 打一行。调试面是"服务还没起来的嘴"：本域没有会话、没有控制台，只有它。
fn say(msg: &str) {
    let _ = debug::put(msg);
}
