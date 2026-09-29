//! control::supervise — **监督相**：哪一位没了、怎么记账、怎么收场。
//!
//! 本文件管**起完之后一直看**：板把"某位的门封印了"变成它那条死亡道上的一格，本线程从
//! 组上醒来、按道上的名字认人、等它真收尾、写 `Dead`、放下它的域、报一行读数；最后一条
//! 走了之后，把仍在跑的**有界地**收掉，然后收场。
//!
//! **装配那一半不在这里**（[`super`] 与 [`super::service`]）：建域 / 放行 / 等就绪 / 收一枚都是
//! **装配期**的事。两半之间只有两处来往——表里那几格状态（`State` / `Slot`），与那两枚原语
//! [`service::stop`] / [`service::until`]（本文件只读它们，不重写）。
//!
//! **道与组是这一相自己的状态**（[`Watch`]）：装配期铸、起完之后一直看——一枚孔一条道，
//! 读者只有本文件。原先它们是 `System` 上的两个裸字段，现收进这一间。

use alloc::vec::Vec;

use crate::program::Program;
use crate::system::control::core::Reaped;
use crate::system::control::desk::{Slot, State, Table};
use env::{HoleDir, Mark, Name, PieToken, Wait};
use protocol::communication::sender::Sender;
use protocol::debug;
use protocol::system::board::LANE_PREFIX;
use protocol::system::control as ccall;
use runtime::core::pile::Pile;
use runtime::env::mail::{self, HolePie};
use runtime::env::unit as utask;

// 表那一侧的两手（本文件只读、不重写）。
use super::Control;
use super::service::{stop, until};

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
    /// **最后一位那条道真的在吗**——决定停机那一格**等什么**。
    ///
    /// **照实记（这一格是构造出来的保证，不是巧合）**：停机靠"最后一条走了"，而"最后一位一定
    /// 要存在信号、且它的孔一定铸得出"本来只是**声明上的巧合**。把它记成一格并在 `of` 里报一行
    /// 读数之后：`true` ⇒ 主路等**道表那枚组**；`false` ⇒ 回退路等**最后一行那一枚线程**
    /// （`Join{task, POLL}`）。两条路**共用同一个 [`sweep`] 与同一套收场**（见 [`Watch::run`]）。
    watch_last: bool,
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
        // **停机的前提不能是巧合**：`lanes` 与 `programs` 同序（上面按同一个次序推），故最后
        // 一格就是"最后一位"。它没有道 ⇒ 读一行数，`run` 改用**内核那一问**当停机触发
        // （回退路与主路共用同一个 `sweep` 与同一套收场，见 [`Watch::run`]）。
        let watch_last = lanes.last().is_some_and(|l| l.road.is_some());
        if !watch_last {
            debug!("system: no lane for the last one; supervise falls back to Join");
        }
        Ok(Watch {
            lanes,
            pile,
            faces: [None; ccall::Grant::ALL.len()],
            watch_last,
        })
    }

    /// **认出某一面的待客入口**：把那一枚挂进**同一只组**（多源等待的写法）。
    ///
    /// 调用者只有一处：`Assembly::mount_control`——**铸入口那一枚线程**（编排域主线程）在
    /// `/svc/sys/control/{…}` 落定之后，把它自己铸的那几枚**逐面**交到这里。那一枚此后归这只组管：
    /// 它的到达就是"有人来问 control 这一面了"那一格。
    /// **装不上也认**（`faces` 仍记着）：面那一侧每拍还会非阻塞地取一次（单槽的推没有丢的
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

    /// 监督循环：**发现死亡 + 记账 + 放下死域**，外加**待客**（control 那一面）。
    ///
    /// 事件有两个来源，挂在**同一只组**上（多源等待，不是一个轮询圈）：
    ///
    /// - **道表**：客人一死，它开的孔随退出钩子封印（或它自己说了退场）⇒ 板当场看出来 ⇒
    ///   往**那一位的死亡道**里推一格 ⇒ 本线程从组上醒来。**一服务一道**，故"是哪一位"由
    ///   **哪条道响**给出——不必猜、也不会两条挤一格丢名字；
    /// - **control 那一面**（[`Watch::face`]）：别的域拿着树上那枚门牌来问四手（`mint` /
    ///   `start` / `stop` / `state`）——本线程醒来把这一问交给 [`Control`] 那四手，从这一趟
    ///   借来的回信孔答回去。**这一源今天有来路**：编排域主线程把 `/svc/sys/control` 挂上树之后
    ///   就把入口交给了本线程（[`Watch::attach_face`]）；哪一景没有持树者，它就空着。
    ///
    /// 醒来先做三件事，次序即契约：**表侧惰性剔死**（内核说收尾了就落 `Dead`——没有道的那几台
    /// 只有这一档收得到）→ **道**（`until` 等它真收尾，再 `Dead` ＋ `oust` ＋ 报一行）→
    /// **面**（取干净这一批客人的问）；回退路下再加**第五格**（最后一行那一枚线程的 `Join`）。
    /// 有台不要存在信号、或最后一位没有道时用**有界节拍**（[`TICK_MS`]）；两者都不需要就
    /// **永远挂起**（事件一到就醒，零轮询）。
    ///
    /// 最后一条没了之后，对**仍在跑的**逐个 `stop`——它们的死会再走同一条路回来；在册的每一行
    /// 都 `Dead` 之后才收场。
    ///
    /// **停机的触发有两个来源，收场只有一套**（照实记）：主路等**道表那枚组**（最后一位那条道
    /// 真的响）；回退路（[`Watch::of`] 的 `watch_last = false`——最后一位不要存在信号、或它那条
    /// 孔铸不出）等**最后一行那一枚线程**（`Join{task, POLL}`）。两条路**共用同一个 [`sweep`]
    /// 与同一套收场**（[`stop_running`] ＋ 本圈那个 `stopping` 分支），差别只在"等什么"。
    pub fn run(&mut self, control: &mut Control, last: Name) {
        // 收帧那一页：**一页**——与门那一侧同一条规则（谁能往里推，缓冲就按**载体**的界备，
        // 不按"这条路上平常走几个字节"备）。备不下 ⇒ 报一句就交给退场时的级联，不在这里赌。
        // 道与面共用这一页（两者不同时读）。
        let mut buf: Vec<u8> = Vec::new();
        if buf.try_reserve_exact(runtime::PAGE_SIZE).is_err() {
            debug!("system: no room");
            return;
        }
        buf.resize(runtime::PAGE_SIZE, 0);
        // **需不需要那格"顺便看一眼"**：两条来路——① 这一景里有**不要存在信号**的台
        // （`road = None`：它们的死没有道来报，只有表侧那一扫收得到）；② 最后一位没有道
        // （`!watch_last`：停机那一格得靠内核那一问看出来）。两者都没有 ⇒ 回到**永远挂起**
        // （事件驱动，零轮询——本线程改动前的样子）。
        //
        // **照实记（为什么不能一律零轮询）**：`relation.presence = false` 那几台（harness 那一片
        // 大多如此）没有任何写端会替它们叫醒本线程；不扫的话它们的死**永远没人记账**。
        let tick = !self.watch_last || self.lanes.iter().any(|l| l.road.is_none());
        let mut stopping = false;
        loop {
            // 一、表侧惰性剔死（G3 从板那本账搬来的那一格）。
            if tick {
                sweep(&mut control.table);
            }
            // 二、等一格有事（有台要"顺便看一眼"时用**有界节拍**，否则永远挂起）。**`Pile` 的
            //     既定用法**：**挂起过的那一侧返回的是预置值**——内核没有第二次执行机会，故醒来
            //     必须自己按组复核，不能靠返回值拿身份。
            let wait = if tick {
                Wait::AtMost(TICK_MS)
            } else {
                Wait::Forever
            };
            if self.pile.await_(wait).is_err() {
                // 组坏了：退回"等最后一条退场"，行为与改动前一致。
                control.wait_last(last);
                return;
            }
            // 三、复核：每条道非阻塞地问一句"有货吗"。**单槽**——道上一次死亡只响一次；一次
            //     醒来可能带走多条（两位前后脚死）。
            for lane in &self.lanes {
                let Some(road) = lane.road else {
                    continue;
                };
                if HolePie::from_token(road)
                    .pull_timeout(&mut buf, Wait::POLL)
                    .is_err()
                {
                    continue; // 这一条没货
                }
                let Ok(name) = Name::new(lane.name) else {
                    continue;
                };
                account(&mut control.table, name);
                // 最后一条走了 ⇒ 会话结束：把仍在跑的显式收掉（只下一次）。
                if name == last && !stopping {
                    stopping = true;
                    stop_running(&mut control.table, &self.lanes);
                }
            }
            // 四、四面：有人来问 control 吗（**逐面**非阻塞地取干净这一批——每枚孔单槽，
            //     各自的批各自取）。位次翻回是哪一面：醒来的是哪一枚孔，就是哪一位。
            for i in 0..ccall::Grant::ALL.len() {
                let Some(face) = self.faces[i] else {
                    continue;
                };
                serve_face(control, ccall::Grant::ALL[i], face, &mut buf);
            }
            // 五、**最后一位没有道时的停机触发**：内核那一问（`Join{task, POLL}`）。与上面道那
            //     一支是**同一句"最后一条走了"**——同一个 `stopping` 分支、同一套收场
            //     （[`stop_running`]），差别只在触发源（主路等道响，回退路等这一枚线程收尾）。
            if !self.watch_last && !stopping && last_reaped(&control.table, last) {
                stopping = true;
                stop_running(&mut control.table, &self.lanes);
            }
            if stopping {
                // 收场：仍在跑的已经**有界地**下过一刀并等过（见 [`stop_running`]）；等不到的
                // 那些交给本域退场时的级联——那条路是既有的可靠收场路径，不在这里等。
                return;
            }
        }
    }
}

/// **有界节拍**（毫秒）：有台"要顺便看一眼"时的等待上限。**不是轮询圈**——事件一到就醒；
/// 这一格只为**表侧惰性剔死**兜底（`relation.presence = false` 那几台没有道来叫醒本线程）。
///
/// **照实记（它为什么是 10 而不是 1）**：表侧那一扫要对每一行在册的服务问一次
/// `Join{task, POLL}`，故节拍直接是这趟开销的倍数；10ms 够让"某位静默地没了"在监督读数里
/// 及时落定，又不把本线程变成一台压着内核问的机器。**有道的景、且最后一位也有道时，根本不走
/// 这一格**（见 [`Watch::run`]）。
const TICK_MS: usize = 10;

/// **表侧惰性剔死**（照实记：G3 从板那本账搬来的那一格）：内核说这一枚收尾了 ⇒ 当场落 `Dead`。
///
/// 与道表那一档的分工：道报的是**板看见的**死（客人开的那扇门封印），本手看的是**内核的事实**
/// （`Join{task, POLL}`）——**不要存在信号的那几台**（`Relation::presence = false`）只有这一档
/// 收得到。判决只认**非阻塞那一问**（与 [`service::until`] 同一条口径：挂起过的那一问读回的
/// 是预置值，不含信息）。
///
/// **`Starting` 与 `Ready` 都扫**（照实记：先前只扫 `Ready`，理由不成立）：`service::mint` 把
/// 已产未放行的身子也置成 `Starting`，故"扫 `Starting` 会误杀 Mint 之后、Start 之前那一枚"
/// 曾是这一格的顾虑——**内核那本账把这件事分开了**：未放行是内核的 `TaskState::Held`，而 `Join`
/// 的判据精确表示**收尾已完成**（`TaskState::Reaped`，`kernel/src/work/unit/task.rs` 的正文）。
/// 于是 `Held` 的身子答 **"没收尾"**（不扫），而起手一段里真死掉的那一枚答"收尾了"（扫掉、
/// 记 `Dead`）。
///
/// 记账与收尾都走 [`mark_dead`]（同一具身体）：落 `Dead`、放下它那个域、报一行读数。
fn sweep(table: &mut Table) {
    // 先把名字抄下来（表是定长的、行数有上界；拿名字再动表——与 `Desk` 那本账同一个形状）。
    let mut gone = [Name::EMPTY; Table::CAP];
    let mut n = 0usize;
    for row in table.rows() {
        if !matches!(row.state, State::Starting | State::Ready) {
            continue;
        }
        let Slot::Live { task, .. } = row.slot else {
            continue;
        };
        if utask::join(task, Wait::POLL).unwrap_or(true) {
            gone[n] = row.name;
            n += 1;
        }
    }
    for name in &gone[..n] {
        mark_dead(table, *name, Reaped::Now);
    }
}

/// **最后一行那一枚线程收尾了吗**——回退路的停机触发（[`Watch::run`] 第五格）。
///
/// 判据与 [`sweep`] 同一条（`Join{task, POLL}` 只答"收尾已完成"）：**只读表**，不动它——
/// 收场那一套仍走 [`stop_running`]（与主路同一套）。
///
/// 没有身子（没登记过 / 从未挂上 / 已 `detach`）也算"不用再等"：没有可等的坐标，停机不该
/// 压在等不到的东西上。
fn last_reaped(table: &Table, name: Name) -> bool {
    match table.find(name) {
        Some(row) => match row.slot {
            Slot::Live { task, .. } => utask::join(task, Wait::POLL).unwrap_or(true),
            Slot::None => true,
        },
        None => true,
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
    // 入口是**单槽**：一次醒来的这一批要取干净（可能不止一位客人）。
    while let Ok((len, from)) = entry.pull_timeout_from(buf, Wait::POLL) {
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
                let _ = Sender::<ccall::frame::Said>::from_token(back)
                    .send(
                        ccall::frame::said_status(ccall::frame::DENIED),
                        Wait::Forever,
                    )
                    .ok();
                let _ = mail::release(back);
                continue;
            }
        }
        let said = answer(control, ask);
        // 答一句走这一趟那枚孔；装不上按构造到不了（`.ok()` 与板那一台同款）。
        let _ = Sender::<ccall::frame::Said>::from_token(back)
            .send(said, Wait::Forever)
            .ok();
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

/// 收场那一刀：给**仍在跑的**每一位 `stop`（`doom` = 域粒度 `Doom`），**有界地**等它
/// 收尾并记账；等不到就报一行，交给本域退场时的级联。
///
/// 为什么有界：`stop` 是"送到即回"（`kill` 的口径），收场不能被一个收不掉的域拖住。
pub fn stop_running(table: &mut Table, lanes: &[Lane]) {
    for lane in lanes {
        let Some(name) = Name::new(lane.name).ok() else {
            continue;
        };
        // **本域那一枚不在这里收**：它的"域"就是本域，收它就是扑杀本域自己。它随本域退场
        // 时的"域亡＝成员清零"一起走。
        //
        // **照实记（订正：这一格今天仓内没有生产者）**：这一支接的从前是"三枚内件住编排域
        // 自己的域"那条路（同域 `spawn_here`，`TeamId(0)`）——那三枚已随 `0560dd8` 回普通程序、
        // 各成各的域，而 `service::mint` 今天只写 `Table::attach(_, Some(team), _)` ⇒ `team = None`
        // 这半格**一处也产不出来**。该状态仍经 `Table::attach(_, None, _)` 可表达，故此支是
        // **护栏、不是活路径**：它护的是"别扑杀本域自己"这条规矩（`mark_dead` 那一格另有
        // `team = None ⇒ ousted = false` 的对照），删了它，将来谁再起一枚同域线程，收场这一刀
        // 会当场扑杀本域。代价照实说：这一支今天编得过、走不到，**没有用例钉它**（表侧那七条
        // 宿主靶已随靶删）——留着它，是把"为什么不能收"留在代码里，而不是留一句只在人脑里的规矩。
        if matches!(
            table.find(name).map(|s| s.slot),
            Some(Slot::Live { team: None, .. })
        ) {
            continue;
        }
        let running = matches!(
            table.find(name),
            Some(s) if matches!(s.state, State::Ready | State::Starting)
        );
        if !running {
            continue;
        }
        let _ = stop(table, name);
        // 表里没有可等的坐标（`stop` 也答了 `Unknown`）：没得等，也不算"卡住"。
        if !matches!(table.find(name).map(|s| s.slot), Some(Slot::Live { .. })) {
            continue;
        }
        match until(table, name, Wait::AtMost(STOP_MS)) {
            Ok(Reaped::Now) => mark_dead(table, name, Reaped::Now),
            Ok(Reaped::Waited) => mark_dead(table, name, Reaped::Waited),
            // 有界期内没等出来：照实报，交出这一位。**不是"没收到"**——判决只认非阻塞
            // 那一问，这里说的是"还没收干净"。
            Ok(Reaped::Unsettled) | Err(_) => {
                debug!("system: stuck {} （退场级联接管）", lane.name);
            }
        }
    }
}

/// 收场那一刀的等待上限（毫秒）。**必须有界**：收场不能被一个收不掉的域拖住。
const STOP_MS: usize = 300;

/// 记一位：**先等它收尾**（板报的是"门封印了"，而 `Oust` 要的前置是"域里没有还没收尾的
/// 线程"，故这一步等的是收尾事件，不是节拍），再写 `Dead`、放下它那个域、报一行。
///
/// **幂等**：已经记过（`Dead`）就什么都不做——板报的道与我们自己杀的那一位可能都指到它。
fn account(table: &mut Table, name: Name) {
    let Some(row) = table.find(name) else {
        return;
    };
    if matches!(row.state, State::Dead) {
        return;
    }
    let Slot::Live { .. } = row.slot else {
        return;
    };
    let reaped = until(table, name, Wait::Forever).unwrap_or(Reaped::Unsettled);
    mark_dead(table, name, reaped);
}

/// 写 `Dead`（**不 `detach`**：坐标是"上一个实例"，留给重启与放下用）、放下那个死域、报一行。
///
/// `reaped` = 这一位的收尾判决**及它的来路**。读数里那一格是给验收用的：`wait=now` 说明收尾
/// 早在问之前就完了，`wait=waited` 说明这一次是**等到**的；`wait=unsettled` 则是"没被确认
/// 收尾"，那时 `ousted=false` 会一起把真相摆出来。
fn mark_dead(table: &mut Table, name: Name, reaped: Reaped) {
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
        name.as_str(),
        if team.is_none() { " inner" } else { "" }
    );
}
