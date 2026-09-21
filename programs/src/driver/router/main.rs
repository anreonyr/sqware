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
//!   → 读树：本域该用哪个 context、树里指到本控制器的线有哪些（顺带报"没进来的账"）
//!   → 板上一趟（挂上本域的服务入口、查回来验一遍：答话三格 + 入口有没有到手）
//!   → 接上树里指到本控制器的每一条线
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
//! 见 [`crate::driver::assemble`]：本域按会话协议装一条叫 `records` 的泊位，父域按需求单把
//! 「名字 + 句柄」的记录推进来，本域按 [`needs::Slot`] 归位。**字节长什么样不在这里**
//! （那是 [`protocol::system::grant`]，与父域同一份）；本域只说"我要哪几格"。
//!
//! # 还没有的那一格：投递给客户端
//!
//! 本域做"收与结"：接上树里指到本控制器的线、claim、complete、应铃。**没有**"名字 → 线号"
//! 的表，也没有把中断投递给客户端那一段——今天来敲门的只有 `guest`（按名字问一句、本域
//! 答一句），还没有谁是"要这条线的中断"的人。
//!
//! **这一格将来的住处已经定了**（用户裁定）：线的权威 / 属主 / 登记 / 投递 / 收线**住在
//! driver protocol 里**——不另立一份 `irq` 协议。本域今天只是它"收与结"的那一半。
//!
//! # 三条读数（都是本刀补的账）
//!
//! - 起域那一行：`ndev` / `ctx` / 接的线集合 / **没进来的四笔**（见 [`crate::plic::Sources`]）；
//! - 每条线**第一次**被领到时一行 `router: line=<n>`——**只可能由中断链产生**
//!   （串口驱动开闸 ⇒ 设备拉线 ⇒ 控制器 ⇒ 内核摇铃 ⇒ 本域 claim）；回显本身走得上轮询，
//!   故没有这一行，中断链断了也没人看得出来；
//! - 静音账的容量按 `ndev` 校验（装不下就拒起，见 [`E_ACCOUNT`]）——账够不够用是
//!   **装配期的判据**，不是运行期的分支。
//!
//! 本域**不读走设备里的字节**：`serial@10000000` 的持有者是 [`crate::driver::uart`]，它也只
//! 开了 `IER.RX`（读口仍在内核的调试面）。不读 ⇒ 源头一直挂着电平，故每领一条线就把它
//! **静音**（`priority = 0`），等铃静下来一拍再把线放回去。按线静音本来就是驱动域自己的
//! 细杠杆；**放回该是一个事件**（"我抽干了"），今天没有那个事件，故退回节拍（见
//! [`IRQ_WAIT_MS`]）。

extern crate alloc;
// 本包 lib 提供 `_start` + panic_handler；必须真的链接它，`use` 只带符号不算。
extern crate programs;

// 客侧装配与需求单都住在驱动这一族里：`assemble` 是两台驱动共用的那段机器（会话 + 配给）。
use programs::driver::assemble;
use programs::driver::router::needs;

// 板：本域是**客侧**（挂牌子、查回来）。
use protocol::system::board::client as board;

/// 设备侧（本域私有，同 `lib.rs` 的纪律：谁的设备谁自己带）。
mod plic;

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

/// 本域挂在板上的名字，以及**对照**用的那个"板上没有的名字"。
///
/// 对照那一问是必要的：只报"查到了"而不知道"查不到会怎样"，那一格读数证明不了什么。
const SERVICE: &str = "router";
const NOBODY: &str = "no-such-name";

/// 与待客线程之间那条路的名字（本域登记它交回来的服务入口时用；它那侧不看名字）。
const DESK_LINK: &str = "router-entry";

/// 装泊位/等配给的期限（毫秒）。
const QUAY_MS: usize = 1000;

/// 有静音的线时等铃的上界（毫秒）。**只有这时才用节拍**——闲的时候本域真睡（见主循环）。
///
/// **这个数没有依据**（照实记）：它的作用只是"手里还压着静音的线时别永久挂起"，而放回
/// 本该是一个事件（"我抽干了"）——把放回换成事件，这个节拍随之消失。
const IRQ_WAIT_MS: usize = 20;

/// 静音账的容量：`u64` 一位一条线，共 [`MUTE_WORDS`] 个字（[`MUTE_LINES`] 条）。
///
/// 容量**由控制器自报的 `ndev` 校验**（见 `boot` 之后那一步与 [`E_ACCOUNT`]）：装不下就
/// 拒起。故"领到的线一定记得下"是构造性事实，不是运行期的分支——旧写法在越界时"当场
/// 放回"，那等于 `claim → complete →` 立刻再报，把一次中断变成风暴。
const MUTE_WORDS: usize = 2;
const MUTE_LINES: u32 = MUTE_WORDS as u32 * 64;

/// 失败编号指"死在装配的哪一步"（本域是常驻的，正常退场码不作数）。
///
/// 装配那三步（`1`–`3`）的编号由 [`assemble`] 那一族共用；本域自己那几格从 4 起。
const E_OPEN: usize = 4;
const E_TREE: usize = 5;
const E_BELL: usize = 6;
const E_DESK: usize = 7;
/// 静音账装不下这台控制器（`ndev > MUTE_LINES`）：**拒起**，不是运行期降级。
const E_ACCOUNT: usize = 8;

/// 线集合：一位一条线（容量见 [`MUTE_WORDS`]）。两处用它，形状同一份：
///
/// - **静音账**：手边还压着哪几条（放回的唯一来源）；
/// - **"报过第一次"账**：哪几条已经打过 `router: line=` 那一行（不按每枚中断打，
///   否则门与 soak 的日志会变脏）。
#[derive(Clone, Copy, Default)]
struct Lines([u64; MUTE_WORDS]);

impl Lines {
    /// 置上这条线。**越界不可达**：起域时已按 `ndev` 校验过容量（见 [`E_ACCOUNT`]）。
    fn set(&mut self, line: u32) {
        self.0[line as usize / 64] |= 1 << (line % 64);
    }

    /// 这条线**是第一次**置吗？（置上并回答）
    fn first(&mut self, line: u32) -> bool {
        let word = &mut self.0[line as usize / 64];
        let bit = 1 << (line % 64);
        let fresh = (*word & bit) == 0;
        *word |= bit;
        fresh
    }

    fn is_empty(&self) -> bool {
        self.0.iter().all(|w| *w == 0)
    }

    /// 逐条取出并清零——调用方按线号做"放回"。
    fn drain(&mut self, mut each: impl FnMut(u32)) {
        for (w, word) in self.0.iter_mut().enumerate() {
            while *word != 0 {
                each(w as u32 * 64 + word.trailing_zeros());
                *word &= *word - 1;
            }
        }
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
    // 静音账装不下这台控制器 ⇒ 拒起：于是"领到的线一定记得下"是构造性事实，不留运行期分支。
    if plic.ndev() > MUTE_LINES {
        exit_with(E_ACCOUNT);
    }
    say("router: docks open");
    // 线集合与四笔"没进来的账"——这台机器上有哪些中断源，唯一一次陈述。
    say(&alloc::format!(
        "router: ndev={} ctx={} lines={:?} muted-cap={} unparented={} beyond={} mapped={} unparsed={}",
        plic.ndev(),
        plic.context(),
        sources.lines,
        MUTE_LINES,
        sources.unparented,
        sources.beyond,
        sources.mapped,
        sources.unparsed
    ));
    let bell = Bell::new(NolePie::from_token(bell_token));

    for &line in &sources.lines {
        plic.enable(line, LINE_PRIORITY);
    }
    // **闸门归设备持有者**：把"收到字节就拉线"打开的那一位在 [`crate::driver::uart`]
    // （它持那枚门闩；见 `needs` 单）。本域只负责把线接上——两件事各归各的账。

    // 板那条路 + 待客线程 + 板上一趟（主线程守铃，服务入口由待客线程守，见文件头）。
    let Ok(sire) = utask::sire() else {
        exit_with(assemble::E_SIRE)
    };
    let Ok(me) = utask::self_id() else {
        exit_with(assemble::E_GRANT)
    };
    serve_board(sire, me);

    // 循环：等铃 → 领到空 → 逐条静音 + 结 → 应铃 → 到点把静音的放回去。
    //
    // **闲的时候真睡**：静音账空时 wait 传 `usize::MAX`（永久挂起），本域一次都不醒。
    // 只有手里还压着静音的线时才退回节拍——那是今天唯一能"放回"的手段（还没有谁会来说
    // 一声"我抽干了"）。把放回换成事件，这个节拍随之消失（见 [`IRQ_WAIT_MS`]）。
    let mut muted = Lines::default();
    let mut seen = Lines::default();
    loop {
        // 铃是一位：闲时即使中断此刻就到，`ring` 也会把本域唤醒——不会漏。
        let wait = if muted.is_empty() {
            usize::MAX
        } else {
            IRQ_WAIT_MS
        };
        match bell.wait(wait) {
            // 铃响：把这一轮该领的都领走。
            Ok(true) => {
                loop {
                    let line = plic.claim();
                    if line == 0 {
                        break;
                    }
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
                    muted.set(line);
                    plic.complete(line);
                }
                // 应铃：清掉那一位并让内核**立即**重开本 hart 的闸门。
                let _ = bell.hush();
            }
            // **没当场就绪**：把**真被静音过**的那几条放回去、放完清零。这一支**不只有
            // "到点"**——`wait` 的 `false` 是"挂起过"（被铃叫醒与期限到**不分**，内核没有
            // 第二次执行机会），故下一轮可能出现"铃其实响着"。那不会漏：内核那一格**先探**
            // 响位，真响着就当场答 `true`（见 `mail/nole.rs::wait`）——故这里不必分辨。
            Ok(false) => muted.drain(|line| plic.enable(line, LINE_PRIORITY)),
            // 铃死了 ⇒ 本域也没事可做。**照实记**：这一支今天不可达——铃的资源实体由内核
            // **永久持有**（`platform/devices.rs::IRQ`），没有任何封印路径。它是一格防御，
            // 不是读数（所以"退场 tally"落不到这里：本域收场是被级联杀的，不走这条支）。
            Err(_) => exit_with(E_BELL),
        }
    }
}

/// 板那条路 + 待客线程：挂上本域的服务入口，再查回来验一遍（读数三格）。
///
/// **它不拦主循环**：板是"起来之后"的事——挂不上只报一句读数（[`board_trip`] 返 `None`），
/// 收与结照旧。
fn serve_board(sire: TaskId, me: TaskId) {
    // 板那条路：本端装一条、认下生我者那一枚（它再转授给板线程）。返两样：本端这座码头 +
    // **板线程的号**（板路上先到的那一格）——孔只在铸它的表里念得出来，故交问话孔、交入口
    // 都得先叫得出板是谁。
    let link = board::open(sire, QUAY_MS).ok();
    // 待客线程 + 服务入口：入口由待客线程铸（谁守那扇门谁读它），副本本域登记。
    let entry = start_desk(me);
    // 板上一趟：挂上这一枚入口、再查回来验一遍。
    let (no_link, no_desk) = (link.is_none(), entry.is_none());
    let trip = match (link, entry) {
        (Some((link, board)), Some(entry)) => board_trip(&link, board, entry),
        _ => None,
    };
    match &trip {
        Some(t) => say(&alloc::format!(
            "router: board reg={} miss={} hit={} entry={} grant={}",
            t.reg,
            t.miss,
            t.hit,
            t.entry.get(),
            t.grant.map(|g| g.get()).unwrap_or(0)
        )),
        None => say(if no_link {
            "router: board: no link"
        } else if no_desk {
            "router: board: no desk"
        } else {
            "router: board: trip"
        }),
    }
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
        say(&alloc::format!("router: desk {who}"));
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
///   reg    REGISTER "router" 的答话码                  —— 0 = 板收下了
///   miss   LOOKUP   "no-such-name" 的答话码          —— 该是 1（UNKNOWN）
///   hit    LOOKUP   "router" 的答话码                  —— 0 = 查到并把入口授了回来
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
///   REGISTER "router"        → 板上挂了本域的服务入口（入口经会话交给板）
///   LOOKUP   "no-such-name"  → 板上没有的名字必须答 UNKNOWN（否则"查到了"不值钱）
///   LOOKUP   "router"        → 查回来一枚入口（板经会话授进本域表里）
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
