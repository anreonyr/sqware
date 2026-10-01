//! control::supervise — **监督相**：哪一位没了、怎么记账、什么时候收场。
//! 本文件管**起完之后一直看**：板把"某位的门封印了"变成它那条死亡道上的一格，本线程从
//! 组上醒来、按道上的名字认人、等它真收尾、写 `Dead`、放下它的域、报一行读数。

use alloc::string::String;
use alloc::vec::Vec;

use crate::system::control::core::{self, Reaped};
use crate::system::control::desk::{Slot, State, Table};
use env::{HoleDir, PieToken, Wait};
use protocol::communication::sender::Sender;
use protocol::debug;
use protocol::system::control as ccall;
use runtime::core::res::pile::Pile;
use runtime::env::chrono::clock;
use runtime::env::mail::{self, HolePie};
use runtime::env::unit as utask;

// 表那一侧的那一手（本文件只读、不重写）。
use super::Control;

/// **监督相在编排域这一侧的状态**：等"有事"的组 ＋ **control 那一面**。
pub struct Watch {
    pile: Pile,
    /// **待客那四枚入口**（一原语一面，位次即 `Grant` 那四位；`None` = 那一面没接上：
    /// 这一景没有持树者 / 那几趟没成）。
    faces: [Option<PieToken>; ccall::Grant::ALL.len()],
}

impl Watch {
    /// **立组**：本线程独享它（`shared = false`）——等"有人来问 control 那一面"。
    pub fn new() -> Result<Watch, ()> {
        // 组是**独占**的（`shared = false`）。
        let pile = Pile::unseal(false).map_err(|_| ())?;
        Ok(Watch {
            pile,
            faces: [None; ccall::Grant::ALL.len()],
        })
    }

    /// **认出某一面的待客入口**：把那一枚挂进**同一只组**（多源等待的写法）。
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
    /// 事件有两个来源，挂在**同一只组**上（多源等待，不是一个轮询圈）：
    pub fn run(&mut self, control: &mut Control) -> bool {
        // 收帧那一页：**一页**——与门那一侧同一条规则（谁能往里推，缓冲就按**载体**的界备，
        let mut buf: Vec<u8> = Vec::new();
        if buf.try_reserve_exact(runtime::PAGE_SIZE).is_err() {
            debug!("system: no room");
            return true;
        }
        buf.resize(runtime::PAGE_SIZE, 0);
        // **收场那一相的闩**：闸一旦成立就一直成立（会走的只会更少），拍下它只是
        // 为了知道"此刻在收场"——判决那一句要等，`tick` 也要跟着它开。
        let mut settling = false;
        let mut forced = false;
        // **上一次"账上少一位"是什么时候**（纳秒标量，`chrono` 那一族的钟）。
        let mut quiet_at = clock();
        let mut owed = control.table.living().count();
        loop {
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
            if clock() - quiet_at >= IDLE_NS {
                if !settling && core::walking(&control.table) {
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
const TICK_MS: usize = 10;

/// **静默上限**（毫秒）：账上一位都没少的时长上限——超过它而闸还没成立（或收场还没收讫），
/// 就**出声并收场**。
const IDLE_MS: usize = 10_000;

/// 静默上限的纳秒形（[`clock`] 那一族的标量）。
const IDLE_NS: u64 = IDLE_MS as u64 * 1_000_000;

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
        // **写端跟着这一趟走**（`tx` 落出作用域时等这只手被取走）。
        {
            let mut tx = Sender::<ccall::frame::Said>::from_token(back);
            let _ = tx.send(said);
        }
        let _ = mail::release(back);
    }
}

/// 把一问交给四手，编出一格答（**读不懂也答**，答 `BAD`）。
/// **四手就是 [`Control`] 那四手**（`mint` / `release` / `stop` / `state`）：本层不重写生命周期，
/// 只做"**复核 + 应答**"——复核的判据在那边一条一条列着；本层只把失败域翻成线上那一格。
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

/// 记一位：`Dead` ＋ 放下它那个域 ＋ 报一行。
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
    debug::put(&alloc::format!(
        "system: gone {} state=Dead ousted={ousted} heir={before}→{after} wait={wait}{}",
        name,
        if team.is_none() { " inner" } else { "" }
    ));
}
