//! 哪一位没了、怎么记账、什么时候收场。
//! 组上醒来、按道上的名字认人、等它真收尾、写 `Dead`、放下它的域、报一行读数。

use alloc::string::String;
use alloc::vec::Vec;

use crate::system::common::life::table::{Slot, State, Table};
use crate::system::common::life::verdict::Reaped;
use env::{HoleDir, PieToken, Wait};
use protocol::communication::hand::Sender;
use protocol::debug;
use protocol::system::control as ccall;
use runtime::core::res::pile::{Pile, Sub};
use runtime::env::chrono::clock;
use runtime::env::mail::{self, HolePie};
use runtime::env::unit as utask;

use super::Control;
use crate::service::operator::bridge::Tree;

use crate::system::common::life::verdict as core;

pub struct Watch {
    /// 一次造、一直持有的那只组：来源变了是**挂/摘/订/退**，不是整只换组。
    pile: Pile,
    /// Control 的四面（boot 时挂上，之后不变）。
    faces: [Option<PieToken>; ccall::Grant::ALL.len()],
    /// 恒挂着的**入口孔**（激活入口 ∪ 主发布入口 ∪ 每个别名入口）；差分出"该摘谁"。
    members: Vec<PieToken>,
    /// 恒装着的**状态订阅**（每个在册任务的收尾完成 ∪ 自己的能力变化）；差分出"该退谁"。
    subs: Vec<Sub>,
}

impl Watch {
    pub fn new() -> Result<Watch, ()> {
        // 组是**独占**的（`shared = false`）：状态订阅只收本地持有的独占组。
        let pile = Pile::unseal(false).map_err(|_| ())?;
        Ok(Watch {
            pile,
            faces: [None; ccall::Grant::ALL.len()],
            members: Vec::new(),
            subs: Vec::new(),
        })
    }

    /// Register a Control request face for the supervisor's wait group.
    pub fn attach_face(&mut self, grant: ccall::Grant, face: PieToken) {
        self.faces[grant.index()] = Some(face);
    }

    /// 把等待集合对齐到"此刻该等的东西"；全部登记成功 ⇒ true。
    ///
    /// 期望集合的唯一来源：四面 ＋ 激活入口 ＋ 主发布入口 ＋ 每个别名入口
    /// （这些入口的增删都发生在监督线程自己经手的 `progress`/`sweep`/服务请求里，
    /// 所以同步点就在它们之后）、每个在册任务的收尾完成、以及自己的能力变化。
    ///
    /// 挂与订本身都会敲一次组（`tole::attach`/`subscribe` 收尾那一下），故"先同步、后等"
    /// 不会把落在两者之间的事件丢掉。
    fn sync(&mut self, control: &Control) -> bool {
        let mut wanted: Vec<PieToken> = Vec::new();
        {
            let hierarchy = control.hierarchy.borrow();
            let capacity = self.faces.len() + 2 + hierarchy.entries().len();
            if wanted.try_reserve(capacity).is_err() {
                return false;
            }
            wanted.extend(self.faces.iter().flatten().copied());
            wanted.extend(hierarchy.entry);
            wanted.extend(hierarchy.entries());
        }
        if let Some(activation) = &control.activation {
            wanted.push(activation.entry());
        }
        let mut at = 0;
        while at < self.members.len() {
            if wanted.contains(&self.members[at]) {
                at += 1;
            } else {
                let token = self.members.swap_remove(at);
                let _ = self.pile.detach(&HolePie::from_token(token), HoleDir::Pull);
            }
        }
        for token in wanted {
            if self.members.contains(&token) {
                continue;
            }
            if self
                .pile
                .attach(&HolePie::from_token(token), HoleDir::Pull)
                .is_err()
            {
                return false;
            }
            self.members.push(token);
        }
        let mut wanted: Vec<Sub> = Vec::new();
        let count = control
            .table
            .living()
            .filter(|row| matches!(row.slot, Slot::Live { .. }))
            .count();
        if wanted.try_reserve(count + 1).is_err() {
            return false;
        }
        wanted.extend(control.table.living().filter_map(|row| match row.slot {
            Slot::Live { task, .. } => Some(Sub::TaskCompleted(task)),
            Slot::None => None,
        }));
        wanted.push(Sub::Capabilities);
        let mut at = 0;
        while at < self.subs.len() {
            if wanted.contains(&self.subs[at]) {
                at += 1;
            } else {
                let sub = self.subs.swap_remove(at);
                let _ = self.pile.unsubscribe(sub);
            }
        }
        for sub in wanted {
            if self.subs.contains(&sub) {
                continue;
            }
            if self.pile.subscribe(sub).is_err() {
                return false;
            }
            self.subs.push(sub);
        }
        true
    }

    /// 监督循环：**发现死亡 + 记账 + 放下死域 + 待客 + 收场**
    /// Control、Hub 激活及发布入口共用一只组；任务退出与新 LINK 有界轮询。
    pub fn run(&mut self, control: &mut Control, tree: &mut Tree) -> bool {
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
            control.activate_hub();
            if let Err(why) = control.progress(tree) {
                debug::put(&alloc::format!("system: {why}"));
                return false;
            }
            let recovered = if settling {
                sweep(control);
                false
            } else {
                match self.recover_identity(control, tree) {
                    Ok(recovered) => recovered,
                    Err(why) => {
                        debug::put(&alloc::format!(
                            "system: identity replacement failed: {why}"
                        ));
                        return false;
                    }
                }
            };
            if recovered {
                owed = control.table.living().count();
                quiet_at = clock();
            }
            // 四、四面：有人来问 control 吗（**逐面**非阻塞地取干净这一批——每枚孔单手，
            //     各自的批各自取）。位次翻回是哪一面：醒来的是哪一枚孔，就是哪一位。
            for i in 0..ccall::Grant::ALL.len() {
                let Some(face) = self.faces[i] else {
                    continue;
                };
                serve_face(control, tree, ccall::Grant::ALL[i], face, &mut buf);
            }
            // 五、把等待集合对齐到"此刻该等的东西"：这一趟里 `progress`/`sweep`/服务请求
            //     都可能增删入口（激活、别名）或增删任务。挂与订收尾那一下会敲一次组，
            //     所以落在"同步"与"睡下"之间的事件不会丢。
            let armed = self.sync(control);
            // 六、先看账有没有少一位（"有动静"是兜底唯一的复位信号）。
            let living = control.table.living().count();
            if living < owed {
                owed = living;
                quiet_at = clock();
            }
            // 七、收场那两句。**闸**读账（会走的都走了、听令的已经发过话）⇒ 下刀；**判决**读账
            //     （一个不剩）⇒ 收场。两者都在 `Control` 那一列上，本相只负责"什么时候问"。
            if !settling && control.due() {
                control.stop_rest();
                settling = true;
                quiet_at = clock();
            }
            if settling && control.done() {
                return !forced;
            }
            // 八、静默兜底：**还有"会自己走"的台，静了 `IDLE_MS` 就出声并收场**；已经在收场而
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
            // 九、睡。**没有事件、也没有业务期限时真睡着**；登记没成全的那一趟退回有界重试。
            if !armed {
                let _ = self.pile.await_(Wait::AtMost(RETRY_MS));
                continue;
            }
            let wait = bound(settling, core::walking(&control.table), quiet_at);
            let _ = self.pile.await_(wait);
        }
    }
}

impl Watch {
    pub(crate) fn recover_identity(
        &self,
        control: &mut Control,
        tree: &mut Tree,
    ) -> Result<bool, &'static str> {
        let authority = control.roster.authority();
        if !sweep(control) {
            return Ok(false);
        }
        if let Some(authority) = authority {
            control.hierarchy.borrow_mut().retire(tree, authority)?;
        }
        control.replace_identity(tree, |control, tree| self.refresh_control(control, tree))?;
        Ok(true)
    }

    fn refresh_control(&self, control: &Control, tree: &mut Tree) -> Result<(), &'static str> {
        let principal = control
            .roster
            .control()
            .ok_or("replacement control identity")?;
        for grant in ccall::Grant::ALL {
            let Some(face) = self.faces[grant.index()] else {
                continue;
            };
            let road = ccall::DIR.try_join(grant.name()).ok_or("control path")?;
            let permit = if grant == ccall::Grant::State {
                protocol::service::operator::Permit::Public
            } else {
                protocol::service::operator::Permit::Identity(
                    protocol::service::identity::Selector::Exact(principal),
                )
            };
            control.hierarchy.borrow_mut().internal(
                tree,
                &road,
                face,
                permit,
                runtime::env::unit::self_id(),
                control.roster.authority(),
            )?;
        }
        Ok(())
    }
}

/// 登记没成的那一趟退回的有界重试间隔（毫秒）：**只在失败路径上出现**，
/// 成事之后回到事件等待，正常路径永不触发。
const RETRY_MS: usize = 10;

/// **静默上限**（毫秒）：账上一位都没少的时长上限——超过它而闸还没成立（或收场还没收讫）
/// 就**出声并收场**
const IDLE_MS: usize = 10_000;

/// 静默上限的纳秒形（clock 那一族的标量）
const IDLE_NS: u64 = IDLE_MS as u64 * 1_000_000;

/// 这一等最多到哪儿：**静默期限只在它真能兑现动作时才挂**。
///
/// 期限那两支（在收场 / 还有"会自己走"的台）照旧按 `quiet_at + IDLE_MS` 算剩余额度；
/// 其余那一格跑满期限唯一的效果是把 `quiet_at` 拨回去（零可观察动作）⇒ 直接 `Forever`。
///
/// 等价关系的形状：`settling` 只由本循环赋值、`walking()` 只读表而表的行只在本线程或
/// 死亡事件里动 ⇒ 睡在 `Forever` 上不会漏掉任何一次期限。
fn bound(settling: bool, walking: bool, quiet_at: u64) -> Wait {
    if !settling && !walking {
        return Wait::Forever;
    }
    let left = IDLE_NS.saturating_sub(clock() - quiet_at);
    Wait::AtMost(left.div_ceil(1_000_000).max(1) as usize)
}

fn sweep(control: &mut Control) -> bool {
    // 先把名字抄下来（表是定长的、行数有上界；拿名字再动表——与 `Desk` 那本账同一个形状）。
    let mut gone = [const { String::new() }; Table::CAP];
    let mut n = 0usize;
    for row in control.table.living() {
        let Slot::Live { task, .. } = row.slot else {
            continue;
        };
        if utask::join(task, Wait::POLL).unwrap_or(true) {
            gone[n] = row.name.clone();
            n += 1;
        }
    }
    let identity_lost = gone[..n].iter().any(|name| name == "identity");
    if identity_lost {
        control.roster.retire();
    }
    for name in &gone[..n] {
        if let Some(task) = control.task(name.as_str()) {
            if let Err(why) = control.roster.unbind(task) {
                debug::put(&alloc::format!("system: departed {name}: {why}"));
            }
        }
        mark_dead(&mut control.table, name.as_str(), Reaped::Now);
    }
    identity_lost
}

/// 答回去
/// 认那枚回信孔靠**帧里那一格** ＋ **一次 mail::reserve 验**（同 `principal/server.rs::turn`
/// 那一门）：那一格是"客人借来的那枚回信孔**在本表里**是几号"——"是谁给的、刻的什么"仍要当场
fn serve_face(
    control: &mut Control,
    tree: &mut Tree,
    grant: ccall::Grant,
    face: PieToken,
    buf: &mut [u8],
) {
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
            continue;
        }
        if let Some(wire) = &ask {
            let asked = ccall::Grant::for_wire(wire);
            if asked != grant {
                debug!("control: face={} denied as={}", grant.name(), asked.name());
                // Push 已复制答话；Sender 可以随本次调用结束销毁。
                {
                    let mut tx = Sender::<ccall::frame::Said>::from_token(back);
                    let _ = tx.send(ccall::frame::said_status(ccall::frame::DENIED));
                }
                let _ = mail::release(back);
                continue;
            }
        }
        let said = answer(control, tree, from, ask);
        {
            let mut tx = Sender::<ccall::frame::Said>::from_token(back);
            let _ = tx.send(said);
        }
        let _ = mail::release(back);
    }
}

/// 把一问交给四手，编出一格答（**读不懂也答**，答 `BAD`）
/// **四手就是 Control 那四手**（`mint` / `release` / `stop` / `state`）：本层不重写生命周期
/// 只做"**复核 + 应答**"——复核的判据在那边一条一条列着；本层只把失败域翻成线上那一格
/// **两格语义一个字不省**：`stop` 只到 `Stopping`（Control::stop 就是 service::stop）
fn answer(
    control: &mut Control,
    tree: &mut Tree,
    from: env::TaskId,
    ask: Option<ccall::frame::Wire>,
) -> ccall::frame::Said {
    let code = |fail: crate::system::common::life::verdict::Fail| {
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
        ccall::frame::Wire::Start(name) => {
            match control.release(name, from, |control| control.progress(tree)) {
                // 的东西。通道那本账留在 Control 里——`Endpoint` 的孔交不出去（见 `frame` 那一节）。
                Ok(service) => ccall::frame::said_task(service.0),
                Err(fail) => ccall::frame::said_status(code(fail)),
            }
        }
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
/// 是本端产生的，不在这一路——它由 answer 那两处"读不懂"直接落）
fn wire_fail(fail: crate::system::common::life::verdict::Fail) -> ccall::Fail {
    use crate::system::common::life::verdict::Fail as Model;
    match fail {
        Model::Unknown => ccall::Fail::Unknown,
        Model::BadImage => ccall::Fail::BadImage,
        Model::Full => ccall::Fail::Full,
        Model::NotReady => ccall::Fail::NotReady,
    }
}

/// 表里那一格状态 → 线上那一格：两套 `State` 五格逐格同形（见协议那一份的头注）
fn wire_state(state: State) -> ccall::State {
    match state {
        State::NeverStarted => ccall::State::NeverStarted,
        State::Starting => ccall::State::Starting,
        State::Ready => ccall::State::Ready,
        State::Stopping => ccall::State::Stopping,
        State::Dead => ccall::State::Dead,
    }
}

/// 记一位：`Dead` ＋ 放下它那个域 ＋ 报一行
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
