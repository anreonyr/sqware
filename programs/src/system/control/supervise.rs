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

use crate::program::Program;
use crate::system::control::core::{self, Reaped};
use crate::system::control::desk::{Slot, State, Table};
use env::{HoleDir, Mark, PieToken, Wait};
use protocol::communication::sender::Sender;
use protocol::debug;
use protocol::system::board::LANE_PREFIX;
use protocol::system::control as ccall;
use runtime::core::pile::Pile;
use runtime::env::chrono::clock;
use runtime::env::mail::{self, HolePie};
use runtime::env::unit as utask;

// 表那一侧的那一手（本文件只读、不重写）。
use super::Control;
use super::service::until;

/// **一条死亡道**：哪一位 + 那一条路（装配期铸的孔，记号 `gone-<名字>`）。
///
/// **照实记（为什么按名字，不按下标）**：原先道与装配表**按下标**对齐（`lanes[i]` ↔ 旧装配表第 i 行，
/// 本文件又按同一个下标把"哪条道响"翻回名字）——两张表必须各自自洽。装配表变成两段相接之后，
/// 跨两段维持"位次自洽"正是那条隐患复发的地方 ⇒ 改成**按名字**（板那一侧本来就是按记号
/// `gone-<名字>` 认领的）。
pub struct Lane {
    /// 这一位是谁（装配表上的名字）。
    pub name: &'static str,
    /// 那一条道。`None` 有**两条来路**：**这一位不要存在信号**（`Relation::presence = false`
    /// ——道是板写的，没有写端就不铸）或**本域铸不出孔**（交给退场级联）。
    pub road: Option<PieToken>,
}

/// **监督相在编排域这一侧的状态**：死亡道表 ＋ 等任一道响的组 ＋ **control 那一面**。
pub struct Watch {
    lanes: Vec<Lane>,
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
    /// **铸道 + 立组**：要存在信号的那几位一位一条（记号 `LANE_PREFIX` ＋ 名字）。
    ///
    /// 失败（那只组立不起来 / 备不下道表）由调用方折成 `system: no group`。
    /// **不要存在信号的那几位不铸道**：没有写端的道永远不会响。
    pub fn of(programs: &[&'static Program]) -> Result<Watch, ()> {
        // 组是**独占**的（`shared = false`）：本线程用它等任一道响（零轮询）。
        let pile = Pile::unseal(false).map_err(|_| ())?;
        let mut lanes: Vec<Lane> = Vec::new();
        lanes.try_reserve(programs.len()).map_err(|_| ())?;
        for program in programs {
            // 记号 = `LANE_PREFIX` ＋ 名字：**前缀只有一处定义**（板那一侧按同一个常量
            // 拼出来找它）。
            //
            // **照实记（这一行已交接回 task-4）**："要不要存在信号"那一格原名 `Program::board`
            // （随板退成**一枚死信号传感器**一起改名，见 `program::Relation::presence`）；改名那一
            // 刀由 T3（G4）落，语义一个字没动。
            let road = if program.relation.presence {
                mail::unseal_hole(Mark::of(&alloc::format!("{LANE_PREFIX}{}", program.name()))).ok()
            } else {
                None
            };
            if let Some(road) = road {
                let _ = pile.attach(&HolePie::from_token(road), HoleDir::Pull);
            }
            lanes.push(Lane {
                name: program.name(),
                road,
            });
        }
        // **照实记（"最后一位那条道真的在吗"那一格随位次一起退场）**：它原是停机的前提
        // （停机靠"最后一条走了"），故要先担保最后一位一定有道、还得为"没有"留一条回退路
        // （`watch_last` ＋ `last_reaped` ＋ 那一圈 10 ms 节拍的理由之一）。闸改读账之后这两样
        // 都没有读者：**道只需要"响的时候叫醒我"这一件事**。
        Ok(Watch {
            lanes,
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

    /// 这一位的死亡道（按名字取，不是按下标：见 [`Lane`]）。
    pub fn lane_of(&self, name: &str) -> Option<PieToken> {
        self.lanes
            .iter()
            .find(|l| l.name == name)
            .and_then(|l| l.road)
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
    /// 醒来先做三件事，次序即契约：**表侧惰性剔死**（内核说收尾了就落 `Dead`——没有道的那几台
    /// 只有这一档收得到）→ **道**（`until` 等它真收尾，再 `Dead` ＋ `oust` ＋ 报一行）→
    /// **面**（取干净这一批客人的问）；然后是收场那两句（见本文件头注）。
    ///
    /// **有界节拍只服务两件事**（照实记：理由归位）：① `Relation::presence` 为假的那几台
    /// **没有道**——它们的死只有表侧那一扫收得到；② **收场期间**判决要反复读账（它不是一次
    /// 事件，是一个不动点）。两者都不需要 ⇒ **永远挂起**（事件一到就醒，零轮询）。
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
        // **组坏了 ⇒ 退化成有界节拍**（照实记：这一支原先的写法是"等最后一条退场、然后返回"
        // ——那是位次时代的回退路）。丢掉的是"谁没了"的**来路**，不是判据：账仍然读得到，
        // `hush` 也还能逐条问。
        let mut blind = false;
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
            let tick = blind || settling || self.lanes.iter().any(|l| l.road.is_none());
            // 一、表侧惰性剔死（G3 从板那本账搬来的那一格）。
            if tick {
                sweep(&mut control.table);
            }
            // 二、等一格有事（要"顺便看一眼"时用**有界节拍**，否则永远挂起）。**`Pile` 的
            //     既定用法**：**挂起过的那一侧返回的是预置值**——内核没有第二次执行机会，故醒来
            //     必须自己按组复核，不能靠返回值拿身份。
            let wait = if tick {
                Wait::AtMost(TICK_MS)
            } else {
                Wait::Forever
            };
            if self.pile.await_(wait).is_err() {
                blind = true;
            }
            // 三、复核：每条道非阻塞地问一句"响着吗"（`hush` 未响答 `Busy`）。**位只有一位**
            //     ——道上一次死亡只响一次；一次醒来可能带走多条（两位前后脚死）。
            for lane in &self.lanes {
                let Some(road) = lane.road else {
                    continue;
                };
                if HolePie::from_token(road).hush().is_err() {
                    continue; // 这一条没事
                }
                account(&mut control.table, lane.name);
            }
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
                    debug!("system: idle {}ms with walkers alive; forcing shutdown", IDLE_MS);
                    forced = true;
                    control.stop_rest();
                    settling = true;
                    quiet_at = clock();
                } else if settling {
                    debug!("system: idle {}ms while settling; {} still alive", IDLE_MS, owed);
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
/// 一个不动点，不是一次事件）。两者都不需要 ⇒ **永远挂起**（见 [`Watch::run`]）。
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

/// **等一位收讫的上限**（毫秒）——道响了之后等它真收尾。
///
/// **为什么有界**（照实记）：这一等原先写的是 `Wait::Forever`。道响只说明"板看见那扇门封印了"，
/// 而收尾是内核那一格的事；两者之间**可以**隔很久（域里还有没收尾的线程）。无界的那一等会把
/// **监督那一趟整个钉住**——钉住之后连判决都读不到，兜底也轮不上。
const ACCOUNT_MS: usize = 1_000;

/// **表侧惰性剔死**（照实记：G3 从板那本账搬来的那一格）：内核说这一枚收尾了 ⇒ 当场落 `Dead`。
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


/// 记一位：**先等它收尾**（板报的是"门封印了"，而 `Oust` 要的前置是"域里没有还没收尾的
/// 线程"，故这一步等的是收尾事件，不是节拍），再写 `Dead`、放下它那个域、报一行。
///
/// **幂等**：已经记过（`Dead`）就什么都不做——板报的道与我们自己杀的那一位可能都指到它。
///
/// **照实记（这一等改成有界了）**：原先写 `Wait::Forever`；道响只说明板看见那扇门封印了，
/// 而收尾是内核那一格的事——两者之间可以隔很久，无界的那一等会把**监督那一趟整个钉住**
/// （钉住之后连判决都读不到）。今天有界（[`ACCOUNT_MS`]），到点照实记 `unsettled`：
/// 那一位先不落 `Dead`，由判决/兜底接着管。
fn account(table: &mut Table, name: &str) {
    let Some(row) = table.find(name) else {
        return;
    };
    if matches!(row.state, State::Dead) {
        return;
    }
    let Slot::Live { .. } = row.slot else {
        return;
    };
    let reaped = until(table, name, Wait::AtMost(ACCOUNT_MS)).unwrap_or(Reaped::Unsettled);
    mark_dead(table, name, reaped);
}

/// 写 `Dead`（**不 `detach`**：坐标是"上一个实例"，留给重启与放下用）、放下那个死域、报一行。
///
/// `reaped` = 这一位的收尾判决**及它的来路**。读数里那一格是给验收用的：`wait=now` 说明收尾
/// 早在问之前就完了，`wait=waited` 说明这一次是**等到**的；`wait=unsettled` 则是"没被确认
/// 收尾"，那时 `ousted=false` 会一起把真相摆出来。
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
    debug!(
        "system: gone {} state=Dead ousted={ousted} heir={before}→{after} wait={wait}{}",
        name,
        if team.is_none() { " inner" } else { "" }
    );
}
