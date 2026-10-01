//! control::supervise — **监督相**：哪一位没了、怎么记账、什么时候收场。
//!
//! 本文件管**起完之后一直看**：板把"某位的门封印了"变成它那条死亡道上的一格，本线程从
//! 组上醒来、按道上的名字认人、等它真收尾、写 `Dead`、放下它的域、报一行读数。
//!
//! **收场只有两句话**（照实记：这一刀把"扳机"从位次里拿出来）：
//!
//! ```text
//!   if control.due()  { control.stop_rest(); }   // 该收了：活着的都是常驻，没人还在等
//!   if control.done() { return true; }           // 收讫了：账上一个不剩
//! ```
//!
//! 原先"该收了"读的是**位次**——单上 `order` 最大的那一台（控制台）退场。那是"位置即语义"，
//! 而且它**不等读数**：读数台（客人 / 探针）还没退场就被扑杀，验收上那片 `no /svc*` 正是这个
//! 形状。今天那一格读账：**会走的没走完，闸就不成立**——"扳机等读数"于是成了推论。
//!
//! **装配那一半不在这里**（[`super`] 与 [`super::service`]）：建域 / 放行 / 等就绪 / 收一枚都是
//! **装配期**的事。两半之间只有两处来往——表里那几格状态（`State` / `Slot`），与那一枚原语
//! `service::until`（本文件只读它，不重写）；**下刀那一手**（`Control::stop_rest`）在收场
//! 那两句里叫，不在本文件的循环里各写一遍。
//!
//! **道与组是这一相自己的状态**（[`Watch`]）：装配期铸、起完之后一直看——一枚孔一条道，
//! 读者只有本文件。

use alloc::string::String;
use alloc::vec::Vec;

use crate::system::control::core::{self, Reaped};
use crate::system::control::desk::{Slot, State, Table};
use env::{HoleDir, PieToken, Wait};
use protocol::communication::sender::Sender;
use protocol::debug;
use protocol::system::control as ccall;
use runtime::core::pile::Pile;
use runtime::env::chrono::clock;
use runtime::env::mail::{self, HolePie};
use runtime::env::unit as utask;

// 表那一侧的那一手（本文件只读、不重写）。
use super::Control;

/// **监督相在编排域这一侧的状态**：等"有事"的组 ＋ **control 那一面**。
///
/// **照实记（道表那一格退场）**：这一格从前还收着一张**道表**（一位一条，记号 `gone-<名字>`）——
/// 死那条来路由板推进去。上一轮量过（见 [`Watch::new`] 那条照实记）：**表侧那一扫独自就够**，
/// 道表那一格随"铸道"一起退场。
pub struct Watch {
    pile: Pile,
    /// **待客那四枚入口**（一原语一面，位次即 `Grant` 那四位；`None` = 那一面没接上：
    /// 这一景没有持树者 / 那几趟没成）。
    ///
    /// **照实记（它为什么与道表同组）**：道那一枚枚是"某一位没了"，面那一枚是"有人来问
    /// control"——两件事都是**本线程要醒一次**的理由，故挂进**同一只组**：多源等待，
    /// 不是两个圈（同 `board/server.rs::host_loop` 的写法）。
    ///
    /// **它们由 [`Watch::attach_face`] 逐面接上**：编排域主线程把
    /// `/svc/sys/control/{state,mint,start,stop}` 四格挂上树之后，当场把本线程铸的那四枚入口交到这里
    /// （`Assembly::mount_control`）——铸入口与待客是**同一枚线程**，故那几枚副本不会被内核的
    /// 派生链摘掉（见 `Assembly::supervise` 的照实记）。
    ///
    /// **位次即面**（`Grant::at()` 1..=COUNT）：醒来的是哪一枚孔，就用哪一位判面——名册 / 盟册
    /// 那两处同一条口径。
    faces: [Option<PieToken>; ccall::Grant::ALL.len()],
}

impl Watch {
    /// **立组**：本线程独享它（`shared = false`）——等"有人来问 control 那一面"。
    ///
    /// **照实记（"铸道"那一半退场：死改由表侧那一扫认）**：这一手从前还要给每一位
    /// `Relation::presence` 的台**铸一条道**（记号 `gone-<名字>`；板看见某位的门封印了就往里推
    /// 一格，本线程从组上醒来）。**上一轮量过**（同一份 release `root` 景、同一喂法）：
    ///
    /// | 那一跑 | 表侧那一扫记上的 | 道那一档（`presence: true` 那 11 位） |
    /// |---|---|---|
    /// | 道与表都在 | 22 位（差 `canonical`——被道先记了） | 11 位 |
    /// | **把道那一档关掉** | **23 位——在单上一台不落** | —— |
    ///
    /// 两跑都是 16 条 `exit tid=…` 逐条相同、无 `system: idle`、无异常。⇒ **道买的是"更早发现"，
    /// 不是"别人看不见的死"**：同一条信息今天由 [`sweep`] 从**内核那一格**（"这一枚收尾了"）
    /// 直接读到，故这里不再铸道、也不再读 [`Relation::presence`]。
    ///
    /// **代价照实说**（照实记，留给下一步）："门封印了、却一直没收尾"那一档（`Reaped::Unsettled`）
    /// 只有道产得出来——本手只在**内核说收尾了**之后才落 `Dead`；真遇上那种台要等静默兜底
    /// （10 s）出声。**上面两跑都没出现这一档**，故这条是**推的**，不是量到的。
    pub fn new() -> Result<Watch, ()> {
        // 组是**独占**的（`shared = false`）。
        let pile = Pile::unseal(false).map_err(|_| ())?;
        Ok(Watch {
            pile,
            faces: [None; ccall::Grant::ALL.len()],
        })
    }

    /// **认出某一面的待客入口**：把那一枚挂进**同一只组**（多源等待的写法）。
    ///
    /// 调用者只有一处：`Assembly::mount_control`——**铸入口那一枚线程**（编排域主线程）在
    /// `/svc/sys/control/{…}` 落定之后，把它自己铸的那几枚**逐面**交到这里。那一枚此后归这只组管：
    /// 它的到达就是"有人来问 control 这一面了"那一格。
    /// **装不上也认**（`faces` 仍记着）：面那一侧每拍还会非阻塞地取一次（单手的推没有丢的
    /// 道理，本手只是把"醒来"这条快路接上）。
    pub fn attach_face(&mut self, grant: ccall::Grant, face: PieToken) {
        let _ = self.pile.attach(&HolePie::from_token(face), HoleDir::Pull);
        self.faces[(grant.at() - 1) as usize] = Some(face);
    }

    /// 监督循环：**发现死亡 + 记账 + 放下死域 + 待客 + 收场**。
    ///
    /// 事件有两个来源，挂在**同一只组**上（多源等待，不是一个轮询圈）：
    ///
    /// - **道表**：客人一死，它开的孔随退出钩子封印（或它自己说了退场）⇒ 板当场看出来 ⇒
    ///   往**那一位的死亡道**里推一格 ⇒ 本线程从组上醒来。**一服务一道**，故"是哪一位"由
    ///   **哪条道响**给出——不必猜、也不会两条挤一格丢名字；
    /// - **control 那一面**（[`serve_face`]）：别的域拿着树上那枚门牌来问四手（`mint` /
    ///   `start` / `stop` / `state`）——本线程醒来把这一问交给 [`Control`] 那四手，从这一趟
    ///   借来的回信孔答回去。**这一源今天有来路**：编排域主线程把 `/svc/sys/control` 挂上树之后
    ///   就把入口交给了本线程（[`Watch::attach_face`]）；哪一景没有持树者，它就空着。
    ///
    /// 醒来做两件事，次序即契约：**表侧惰性剔死**（内核说"这一枚收尾了" ⇒ 落 `Dead` ＋ `oust`
    /// ＋ 报一行——**它就是"谁没了"唯一的来路**）→ **面**（取干净这一批客人的问）；然后是收场
    /// 那两句（见本文件头注）。
    ///
    /// **照实记（"永远挂起（零轮询）"那一句退了场）**：那一趟从前还等一道"道"——有界节拍只在
    /// "有人没有道"或"正在收场"时开。逐份声明数过：在单上 **23 台里 11 台 `presence: true`、
    /// 12 台为假** ⇒ 那个条件**每一拍都为真** ⇒ "零轮询"从来没有成立过。今天道那一档整支退场
    /// （见 [`Watch::new`] 那条照实记里的两跑），那一扫就是唯一的来路 ⇒ **节拍是它的节拍**，
    /// 不必再装作"挂起"：`Wait::AtMost(TICK_MS)` 写在那一行上。
    ///
    /// 返 `true` = **全收讫**（这一趟的结局）；`false` = 有人没收讫，交本域退场时的级联。
    pub fn run(&mut self, control: &mut Control) -> bool {
        // 收帧那一页：**一页**——与门那一侧同一条规则（谁能往里推，缓冲就按**载体**的界备，
        // 不按"这条路上平常走几个字节"备）。备不下 ⇒ 报一句就交给退场时的级联，不在这里赌。
        // 道与面共用这一页（两者不同时读）。
        let mut buf: Vec<u8> = Vec::new();
        if buf.try_reserve_exact(runtime::PAGE_SIZE).is_err() {
            debug!("system: no room");
            return true;
        }
        buf.resize(runtime::PAGE_SIZE, 0);
        // **收场那一相的闩**（不是判据）：闸一旦成立就一直成立（会走的只会更少），拍下它只是
        // 为了知道"此刻在收场"——判决那一句要等，`tick` 也要跟着它开。
        let mut settling = false;
        // **静默兜底**：以"有台退场"为事件，不以时钟为日程——正常跑一次都走不到。
        //
        // **它为什么必须有**（照实记）：闸读的是账，而账要等的那些台**可能会永远不退场**
        // （一道门的对面一直不答、一圈"有界重试"每一轮都含着一问 `Wait::Forever`……）。
        // 没有它，机器就钉在那里——那是比"红"更坏的结局：**红而无成因**。
        let mut forced = false;
        // **上一次"账上少一位"是什么时候**（纳秒标量，`chrono` 那一族的钟）。
        let mut quiet_at = clock();
        let mut owed = control.table.living().count();
        loop {
            // 一、表侧惰性剔死（G3 从板那本账搬来的那一格）——**今天它是"谁没了"唯一的来路**
            //     （照实记见 [`Watch::new`]）。
            sweep(&mut control.table);
            // 二、等一格有事：**有界节拍**（`TICK_MS`）。它不再是"顺便看一眼"的兜底，而是那一扫
            //     的节拍本身 ⇒ **每一拍都要**（不再由"有没有道"决定）。**`Pile` 的既定用法**：
            //     挂起过的那一侧返回的是预置值——内核没有第二次执行机会，故醒来必须自己按组复核，
            //     不能靠返回值拿身份。
            let _ = self.pile.await_(Wait::AtMost(TICK_MS));
            // 四、四面：有人来问 control 吗（**逐面**非阻塞地取干净这一批——每枚孔单手，
            //     各自的批各自取）。位次翻回是哪一面：醒来的是哪一枚孔，就是哪一位。
            for i in 0..ccall::Grant::ALL.len() {
                let Some(face) = self.faces[i] else {
                    continue;
                };
                serve_face(control, ccall::Grant::ALL[i], face, &mut buf);
            }
            // 五、先看账有没有少一位（"有动静"是兜底唯一的复位信号）。
            let living = control.table.living().count();
            if living < owed {
                owed = living;
                quiet_at = clock();
            }
            // 六、收场那两句。**闸**读账（会走的都走了、听令的已经发过话）⇒ 下刀；**判决**读账
            //     （一个不剩）⇒ 收场。两者都在 `Control` 那一列上，本相只负责"什么时候问"。
            if !settling && control.due() {
                control.stop_rest();
                settling = true;
                quiet_at = clock();
            }
            if settling && control.done() {
                // **出了静默兜底就不算自然收讫**：那一趟的结局要留在读数上（`Fail::Doom`）。
                return !forced;
            }
            // 七、静默兜底：**还有"会自己走"的台，静了 `IDLE_MS` 就出声并收场**；已经在收场而
            //     判决还没成立，静了同样久就把余下交给退场级联（`false`）。
            //     只剩常驻 / 听令的台时**不兜底**——听令那一台的等待归外面那一层（喂它的那个人）。
            if clock() - quiet_at >= IDLE_NS {
                if !settling && core::walking(&control.table) {
                    // **这两句也走不设门的那一手**（照实记：与 `mount_grants` 那条同一类——
                    // 上一刀把那一批兑现时漏了这两句；兜底"出声"若在 release 里哑掉，
                    // 就只剩退场那一层的 `system: doom`，说的是"没走完"、没说"卡在哪一档"）。
                    debug::put(&alloc::format!(
                        "system: idle {}ms with walkers alive; forcing shutdown",
                        IDLE_MS
                    ));
                    forced = true;
                    control.stop_rest();
                    settling = true;
                    quiet_at = clock();
                } else if settling {
                    debug::put(&alloc::format!(
                        "system: idle {}ms while settling; {} still alive",
                        IDLE_MS,
                        owed
                    ));
                    return false;
                } else {
                    quiet_at = clock();
                }
            }
        }
    }
}

/// **有界节拍**（毫秒）：要"顺便看一眼"时的等待上限。**不是轮询圈**——事件一到就醒。
///
/// **照实记（这一格的理由归了位）**：从前它的理由写成"只为表侧惰性剔死兜底"，还夹着"最后一位
/// 没有道"那半条（位次时代）。今天它只服务两件事：① `Relation::presence` 为假的那几台
/// **没有道**——它们的死只有表侧那一扫收得到；② **收场期间**判决要反复读账（`core::done` 是
/// 一个不动点，不是一次事件）。**两件事今天都还在**（23 台里 12 台 `presence: false`）⇒ 这一拍
/// 是真的在跑，不是回退路——"永远挂起"那一句的照实记与判据见 [`Watch::run`]。
///
/// **它为什么是 10 而不是 1**：表侧那一扫要对每一行在册的服务问一次 `Join{task, POLL}`，
/// 故节拍直接是这趟开销的倍数；10 ms 够让"某位静默地没了"在监督读数里及时落定，又不把本线程
/// 变成一台压着内核问的机器。
const TICK_MS: usize = 10;

/// **静默上限**（毫秒）：账上一位都没少的时长上限——超过它而闸还没成立（或收场还没收讫），
/// 就**出声并收场**。
///
/// **它是活性下限，不是日程**（照实记）：正常跑一次都走不到（有台在动就会复位）。它买的只有
/// 一件事——**机器不会永远停不下来**；付的代价照实说：真有一台卡住时，读数可能缺，而"缺"
/// 这件事由本相印出来（`system: doom`），不再是一片沉默。
const IDLE_MS: usize = 10_000;

/// 静默上限的纳秒形（[`clock`] 那一族的标量）。
const IDLE_NS: u64 = IDLE_MS as u64 * 1_000_000;

/// **表侧惰性剔死**（照实记：G3 从板那本账搬来的那一格）：内核说这一枚收尾了 ⇒ 当场落 `Dead`。
///
/// **照实记（本手能不能独自把每一位都记上——量过，它是"撤板"那一刀的判据）**：同一份 release
/// `root` 景、同一喂法，量了两次：
///
/// | 那一跑 | 这一扫记上的 | 道那一档（`presence: true` 那 11 位） |
/// |---|---|---|
/// | 道与表都在（今天） | 22 位（差 `canonical`） | 11 位（`canonical` 被道先记了） |
/// | **把道那一档关掉**（探子，只量） | **23 位——在单上一台不落** | ——（探子仍报 11 次，不记账） |
///
/// 两跑的 `exit tid=…` 都是 **16 条、逐条相同**，都没有兜底（`system: idle`）与异常，机器都正常
/// 收场。⇒ **表侧这一扫独自就够**：道那一档给的是**更早**的发现（"门封印了"），而不是"别人看不见
/// 的死"；`canonical` 那一台就是因为道先记上、这一扫才没轮到它。
///
/// **代价照实说**（下一刀要认的）：`Reaped::Unsettled` 那一档（"门封印了、却一直没收尾"）只有道
/// 那一档产得出来——本手只在**内核说收尾了**之后才落 `Dead`；真遇上"封印了不收尾"的台，它要等
/// 静默兜底（10 s）出声。**上面那一跑没有出现这一档**，故这条代价是**推的**，不是量到的。
///
/// 与道表那一档的分工：道报的是**板看见的**死（客人开的那扇门封印），本手看的是**内核的事实**
/// （`Join{task, POLL}`）——**不要存在信号的那几台**（`Relation::presence = false`）只有这一档
/// 收得到。判决只认**非阻塞那一问**（与 [`service::until`] 同一条口径：挂起过的那一问读回的
/// 是预置值，不含信息）。
///
/// **`Starting` / `Ready` / `Stopping` 都扫**（照实记：这一格的相集换过两次）：
///
/// - 先前只扫 `Ready`——`service::mint` 把已产未放行的身子也置成 `Starting`，故"扫 `Starting`
///   会误杀 Mint 之后、Start 之前那一枚"曾是顾虑；**内核那本账把这件事分开了**：未放行是内核的
///   `TaskState::Held`，而 `Join` 的判据精确表示**收尾已完成**（`TaskState::Reaped`，
///   `kernel/src/work/unit/task.rs` 的正文），于是 `Held` 的身子答"没收尾"（不扫）。
/// - 这一刀改成 [`Table::living`]：**`Stopping` 也算活着**——被线上 `Stop` 推入 `Stopping`、
///   而它那条道又不响的行（`presence = false` 那几台）只有这一扫收得到；少了它，判决
///   （`core::done`）永远不会成立。
///
/// 记账与收尾都走 [`mark_dead`]（同一具身体）：落 `Dead`、放下它那个域、报一行读数。
fn sweep(table: &mut Table) {
    // 先把名字抄下来（表是定长的、行数有上界；拿名字再动表——与 `Desk` 那本账同一个形状）。
    let mut gone = [const { String::new() }; Table::CAP];
    let mut n = 0usize;
    for row in table.living() {
        let Slot::Live { task, .. } = row.slot else {
            continue;
        };
        if utask::join(task, Wait::POLL).unwrap_or(true) {
            gone[n] = row.name.clone();
            n += 1;
        }
    }
    for name in &gone[..n] {
        mark_dead(table, name.as_str(), Reaped::Now);
    }
}

/// 招待一位客人（面那一侧）：从**待客那一枚入口**读一帧、复核、交给四手、从这一趟借的回信孔
/// 答回去。
///
/// 认那枚回信孔靠**帧里那一格** ＋ **一次 [`mail::reserve`] 验**（同 `principal/server.rs::turn`
/// 那一门）：那一格是"客人借来的那枚回信孔**在本表里**是几号"——"是谁给的、刻的什么"仍要当场
/// 读出来核对，否则客人能让本域往**别人的孔**里写。
fn serve_face(control: &mut Control, grant: ccall::Grant, face: PieToken, buf: &mut [u8]) {
    let entry = HolePie::from_token(face);
    // 入口是**单手**：一次醒来的这一批要取干净（可能不止一位客人）。
    while let Ok((len, from)) = entry.pull(buf, Wait::POLL) {
        let Some((ask, back)) = ccall::frame::Wire::take(&buf[..len]) else {
            // 长度不对 ⇒ 连"往哪回"都没有：不猜、不动表、也不回话。
            continue;
        };
        if !matches!(
            mail::reserve(back),
            Ok((_vestor, owner, mark)) if owner == from && mark == ccall::BACK
        ) {
            // 这一趟没把回信孔交进来、或那一格指的是别人的孔：没有可回的路，账一动不动。
            continue;
        }
        // **面这一道在交给四手之前**：这一帧是从哪一面进来的，与它问的那一条属不属于那一面
        // ——只有这一句说得清。对不上答 [`DENIED`](ccall::frame::DENIED)（**终态**：换一面 /
        // 别重试），与 kernel 那几格失败分开。**它是"问面公开"的安全阀**：手里只有问面那一枚
        // 入口的人，发不出 `Mint` / `Start` / `Stop`（同 principal / coalition 那两族那一格）。
        //
        // 表外的动作码（`ask = None`）**不在这里判**：连面都判不出来，交给下面那一趟答 `BAD`。
        if let Some(wire) = &ask {
            let asked = ccall::Grant::of_wire(wire);
            if asked != grant.at() {
                debug!(
                    "control: face={} denied as={}",
                    grant.name(),
                    ccall::Grant::ALL[(asked - 1) as usize].name()
                );
                // **递出去就回去待客**：答话那一格由这一枚 `Sender` 自己担着（落出作用域时
                // 等那只手被取走）。**没有"共用一格存根"了**：一位客人一枚写端，一格招待所有
                // 客人那种错因此编不出来。**收口在 `release` 之前**（见下面那一格的照实记）：
                {
                    let mut tx = Sender::<ccall::frame::Said>::from_token(back);
                    let _ = tx.send(ccall::frame::said_status(ccall::frame::DENIED));
                }
                let _ = mail::release(back);
                continue;
            }
        }
        let said = answer(control, ask);
        // 答一句走这一趟那枚孔；装不上按构造到不了（`.ok()` 与板那一台同款）。
        // **写端跟着这一趟走**（`tx` 落出作用域时等这只手被取走）——不再有一格共用的存根。
        // **照实记（收口为什么必须在 `release` 之前）**：那一等要用**本域表里这一枚**（`wait`
        // 要走权限那一关）；先放下它再等 ⇒ `Denied` 当场返回，而孔上那只手还指着**这一帧的
        // 栈**——下一趟复用同一片栈，取的人复制到的就是别人的字节。量到的症状：客侧
        // `recv-unread`，而内核 `hand_over` 那一行一切正常（长度、发送者都对）。
        {
            let mut tx = Sender::<ccall::frame::Said>::from_token(back);
            let _ = tx.send(said);
        }
        let _ = mail::release(back);
    }
}

/// 把一问交给四手，编出一格答（**读不懂也答**，答 `BAD`）。
///
/// **四手就是 [`Control`] 那四手**（`mint` / `release` / `stop` / `state`）：本层不重写生命周期，
/// 只做"**复核 + 应答**"——复核的判据在那边一条一条列着；本层只把失败域翻成线上那一格。
///
/// **两格语义一个字不省**：`stop` 只到 `Stopping`（[`Control::stop`] 就是 [`service::stop`]），
/// 落 `Dead` 的是**监督那一趟**（[`account`] 的 `until` 两相）——本层不为它抢一步。
fn answer(control: &mut Control, ask: Option<ccall::frame::Wire>) -> ccall::frame::Said {
    let code = |fail: crate::system::control::core::Fail| {
        ccall::frame::fail_to_code(Some(wire_fail(fail)))
    };
    let Some(ask) = ask else {
        // 表外的动作码：这一问有回信的路，只是这一码我不认（与"读不懂"同一格）。
        return ccall::frame::said_status(ccall::frame::BAD);
    };
    match ask {
        ccall::frame::Wire::Mint(name) => match control.mint(name) {
            Ok(()) => ccall::frame::said_status(ccall::frame::OK),
            Err(fail) => ccall::frame::said_status(code(fail)),
        },
        ccall::frame::Wire::Start(name) => match control.release(name) {
            // **答的是那一枚身子**（第三格）：`TaskId` 跨域有意义，故它是这一族唯一交得出域外
            // 的东西。通道那本账留在 [`Control`] 里——`Endpoint` 的孔交不出去（见 `frame` 那一节）。
            Ok(service) => ccall::frame::said_task(service.0),
            Err(fail) => ccall::frame::said_status(code(fail)),
        },
        ccall::frame::Wire::Stop(name) => match control.stop(name) {
            Ok(()) => ccall::frame::said_status(ccall::frame::OK),
            Err(fail) => ccall::frame::said_status(code(fail)),
        },
        ccall::frame::Wire::State(name) => match control.state(name) {
            Ok(state) => ccall::frame::said_state(wire_state(state)),
            Err(fail) => ccall::frame::said_status(code(fail)),
        },
    }
}

/// 模型那一格失败 → 线上那一格失败：两套都是**四格语义格**，逐格同形（协议那一份的 `Bad`
/// 是本端产生的，不在这一路——它由 [`answer`] 那两处"读不懂"直接落）。
fn wire_fail(fail: crate::system::control::core::Fail) -> ccall::Fail {
    use crate::system::control::core::Fail as Model;
    match fail {
        Model::Unknown => ccall::Fail::Unknown,
        Model::BadImage => ccall::Fail::BadImage,
        Model::Full => ccall::Fail::Full,
        Model::NotReady => ccall::Fail::NotReady,
    }
}

/// 表里那一格状态 → 线上那一格：两套 `State` 五格逐格同形（见协议那一份的头注）。
fn wire_state(state: State) -> ccall::State {
    match state {
        State::NeverStarted => ccall::State::NeverStarted,
        State::Starting => ccall::State::Starting,
        State::Ready => ccall::State::Ready,
        State::Stopping => ccall::State::Stopping,
        State::Dead => ccall::State::Dead,
    }
}

// **照实记（`stop_running` 与 `STOP_MS` 那一对退了）**：那两样是"扳机响 ⇒ 有界地逐位收、
// 逐位等"那一式收场。这一刀把收场拆成两件事——**下刀**（`Control::stop_rest`，下令即回）
// 与**等账空**（`core::done`，每一拍读一次）——于是"有界"这一格不再属于某一位，而属于
// 收场那一相自己。它留下的两样东西搬去了别处：**"别收本域那一枚"**那条护栏与它的照实记
// 住 `Control::stop_rest`；**"等不到就报一行"**住那一相的判决（`Watch::run`）。

/// 记一位：`Dead` ＋ 放下它那个域 ＋ 报一行。
///
/// **"先等它收尾"那一句随道退场了**（照实记）：从前这里要等——道报的是"**门封印了**"，而
/// `Oust` 要的前置是"域里没有还没收尾的线程"，两者之间可以隔很久。今天叫本手的那一处
/// （[`sweep`]：**内核说这一枚已经收尾了**）已经把那个前置查过了，故这一等没有了。
///
/// **幂等**：已经记过（`Dead`）就什么都不做（一位只落一次，故那一行读数一位只有一行）。
///
/// **照实记（`reaped` 那一格今天只有一档）**：从前它三档——`Now` / `Waited` / `Unsettled`
/// （有道那一档时，"门封印了但还没收尾"要等一等再报）。道退场之后，本手只在**内核说收尾了**
/// 之后被叫到 ⇒ 送进来的恒是 [`Reaped::Now`]；另两档的**生产者**（`account`）随道一起退了场，
/// 那一档（"封印了不收尾"）今天归静默兜底（10 s）管——代价写在 [`Watch::new`] 的照实记里。
/// 枚举那三档留着：`until` 那一族（`service.rs`）还在用它们。
fn mark_dead(table: &mut Table, name: &str, reaped: Reaped) {
    let Some(row) = table.find(name) else {
        return;
    };
    if matches!(row.state, State::Dead) {
        return;
    }
    let Slot::Live { team, .. } = row.slot else {
        return;
    };
    table.set_state(name, State::Dead);
    let before = utask::heir_count();
    // **本域那一枚没有别人的域可放下**（`team = None`）：放下它就是扑杀本域自己。
    let ousted = match team {
        Some(team) => utask::oust(team).is_ok(),
        None => false,
    };
    let after = utask::heir_count();
    let wait = match reaped {
        Reaped::Now => "now",
        Reaped::Waited => "waited",
        Reaped::Unsettled => "unsettled",
    };
    // **这一行是"谁没了"的唯一读数**（哪一位、放下它的域成没成、域里少了几个后继、等的是哪一档）
    // ——故它不设构建门（照实记：与 `mount_grants` 那条同一类，那里的实测写着"这一批在 release
    // 里从来就没落过一格"；它哑掉时，"谁没了"只剩退场那一层的 `system: doom`，**没有名字**）。
    debug::put(&alloc::format!(
        "system: gone {} state=Dead ousted={ousted} heir={before}→{after} wait={wait}{}",
        name,
        if team.is_none() { " inner" } else { "" }
    ));
}
