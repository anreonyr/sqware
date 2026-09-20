//! system::server — **适配**：把内核答的事实写回表（建域 / 放行 / 等就绪 / 收 / 盯）
//!
//! 正文见 [`super`]；三档（判定 / 账 / 适配）分家的理由见 `system` 模块头注。

use env::{Name, Permission, PieToken, ProgramKind, TaskId};
use runtime::core::tole::Tole;
use runtime::env::mail::HolePie;
use runtime::env::unit as utask;

use protocol::session::Quay;

use protocol::system::core::{Fail, Ready, Reaped, admit_start, probe_ready};
use protocol::system::desk::{Announce, Service, Slot, State, Table};

use crate::supervisor::service::Program;

// ── 适配：原语（转发到运行时那几件）──────────────────────────

/// 起跑前要交出去的一枚门闩：给哪一枚、多大权。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Grant {
    /// 要交出去的那一枚（**在我表里**的句柄）。
    pub token: PieToken,
    /// 交出去的权限子集。
    pub perm: Permission,
}

/// 起一个 Service。
///
/// `image` = 镜像字节（v1 口径）；`kind` = 特权级（**由清单决定**，调用方转交）；
/// `grants` = **放行前**要交到它手里的门闩（空 = 什么都不预先给）；
/// `quay` = 与它的会话（`Announce::Channel` 那一种才有：它就绪的凭据长在这里）；
/// `ms` = 就绪等待（`0` 只探、`usize::MAX` 无期限、其余毫秒）。
///
/// **失败时不留下半行**：产线程**之前**失败 ⇒ 表不动；产线程**之后**失败 ⇒ 实例与
/// 状态都如实留在表里（它确实在跑），调用方用 [`stop`] 收尾。
pub fn spawn(
    table: &mut Table,
    name: Name,
    image: &[u8],
    kind: ProgramKind,
) -> Result<TaskId, Fail> {
    admit_start(table, name)?;

    let team = crate::supervisor::system::call::mint(image, kind, name)?;
    let Ok(rep) = crate::supervisor::system::call::bear(team) else {
        return Err(Fail::NoRoom);
    };
    table.attach(name, team, rep)?;
    table.set_state(name, State::Starting);
    Ok(rep)
}

/// 第二相：**放行**，并在放行前塞门闩、定会话。
///
/// `grants` = 放行前要交到它手里的门闩（空 = 什么都不预先给）；`quay` = 与它的会话
/// （`Announce::Channel` 那一种要用：它就绪的凭据长在这里）；`marks` = 放行后要逐条
/// 认领的**记号**（顺序无关；记号即泊位名，装配单给的通道名就是它）；`ms` = 就绪等待
/// （`0` 只探、`usize::MAX` 无期限、其余毫秒）。
///
/// **两相之间的窗口就是"它一步都还没跑"**——塞门闩、定会话都发生在这段窗口里，这与
/// 载体的"恒产未放行"是同一条。
///
/// **失败时不留下半行**：`spawn` 失败 ⇒ 表不动；本相失败 ⇒ 实例与状态如实留在表里
/// （它确实在跑），调用方用 [`stop`] 收尾。
///
/// # Errors
/// 见 [`Fail`]。
/// `ms` = 就绪判定的**上限族**毫秒（口径见 `env::fid` 文件头的定式：`0` 只探测、
/// `usize::MAX` 永久）——超时与"成立了"按返回值区分。
pub fn start(
    table: &mut Table,
    name: Name,
    rep: TaskId,
    grants: &[Grant],
    quay: Option<&mut Quay>,
    marks: &[Name],
    ms: usize,
) -> Result<(), Fail> {
    let launched = (|| -> Result<(), Fail> {
        for g in grants {
            crate::supervisor::system::call::accord(g.token, rep, g.perm)?;
        }
        crate::supervisor::system::call::hatch(rep)
    })();
    if let Err(e) = launched {
        crate::supervisor::system::call::ruin(rep);
        table.detach(name);
        table.set_state(name, State::Dead);
        return Err(e);
    }
    ready(table, name, quay, marks, ms)?;
    Ok(())
}

/// 等它就绪。`true` = **调用开始时就已就绪**（挂起过的一律 `false`）。
///
/// 唯一会改表的地方就是这个函数：确认它的宣布之后置 [`State::Ready`]，确认它死了之后
/// 落 [`State::Dead`]——**内核的事实在这里变成表里的事实**。而"它宣布了"那件事本身
/// **归 [`session`](crate::session)**：会话在对方交回孔并归位时成立，本函数只去问那句
/// "成立了没有"。
pub fn ready(
    table: &mut Table,
    name: Name,
    quay: Option<&mut Quay>,
    marks: &[Name],
    ms: usize,
) -> Result<bool, Fail> {
    // 先看表：上一次问过的事实（按这一行自己声明的说法解读）。
    if let Ready::Up = probe_ready(table, name) {
        return Ok(true);
    }
    let Some(Service {
        slot: Slot::Live { rep, .. },
        announce,
        ..
    }) = table.find(name)
    else {
        return Err(Fail::NotReady);
    };
    let (rep, announce) = (*rep, *announce);

    // 它不宣布的那一种：放行之后只要还活着就算起来了，没有可等的东西。
    if announce == Announce::None {
        if crate::supervisor::system::call::running(rep) {
            table.set_state(name, State::Ready);
            return Ok(false);
        }
        table.detach(name);
        table.set_state(name, State::Dead);
        return Err(Fail::NotReady);
    }

    // 它会交回一枚孔 ⇒ 那件事归会话：`claim` 把它认下来（凑齐了才算）。
    if let Some(q) = quay {
        // 它会交回孔、也会自己装一条 ⇒ 那件事归会话：**逐条按记号认领**（记号 = 那条
        // 泊位的名字 = 装配单给的通道名），每条都配齐才算起来。认领的是**它**交上来的
        // 那一批（`owner` = 这个孩子）：孔交给的是"生我者"（建域那一枚线程），而
        // **"谁的孔"与"我认的对端"是两件事**——见 `Quay::claim` 的正文。
        if !marks.is_empty() && marks.iter().all(|mark| q.claim(rep, *mark, ms).is_ok()) {
            table.set_state(name, State::Ready);
            return Ok(false);
        }
    }
    if !crate::supervisor::system::call::running(rep) {
        table.detach(name);
        table.set_state(name, State::Dead);
        return Err(Fail::NotReady);
    }
    // 还活着、只是没宣布：它确实在跑，**实例如实留在表里**（调用方用 `stop` 收尾）。
    if ms == 0 {
        return Ok(false);
    }
    Err(Fail::NotReady)
}

/// 收掉一个 Service。**下令即回，不等它收完**。
pub fn stop(table: &mut Table, name: Name) -> Result<(), Fail> {
    let Some(Service {
        slot: Slot::Live { rep, .. },
        ..
    }) = table.find(name)
    else {
        return Err(Fail::Unknown);
    };
    let rep = *rep;
    crate::supervisor::system::call::ruin(rep);
    table.set_state(name, State::Stopping);
    Ok(())
}

/// 等它收尾：`ms` 三态与 [`ready`] 同款（`0` 只探、`usize::MAX` 挂到它收尾、其余毫秒）。
/// **只读：不动表**。
///
/// 形状是 **问 → 等 → 问**，判决只认两次**非阻塞问**（`Join{rep, 0}`）；等只是为了少问几次。
/// `Join{rep, ms}` 挂起过之后的返回值不含信息（见 [`Reaped`]），故醒来必须复探——**"他杀
/// 偶发不生效"那条错读就是漏了这一步**：把"醒过"当成了"没收到"。
///
/// 为什么有界等不需要 clock、也不必睡满 `ms`：`WakeKey::Task{id}` 上的投信方只有 `wipe`，
/// 而它只在 `bury` 里、`Reaped` 置位**之后**调（`kernel/src/work/room/messenger/reap.rs`）
/// ⇒ 等待里的"早醒"只可能来自收尾；"到点"那一支由复探分出来（答 [`Reaped::Unsettled`]）。
///
/// `Err(Fail::Unknown)` = 表里没这一行、或这一行还没有身子的坐标。问不出（`Denied` =
/// 已入土 / 从未入册）按"收尾了"处理——与 [`running`](crate::supervisor::system::call) 同一折法。
/// `ms` = **上限族**（口径见 `env::fid` 文件头的定式）；超时那支答 [`Reaped::Unsettled`]，
/// 不写表。
pub fn until(table: &Table, name: Name, ms: usize) -> Result<Reaped, Fail> {
    let Some(rep) = live_rep(table, name) else {
        return Err(Fail::Unknown);
    };
    if !crate::supervisor::system::call::running(rep) {
        return Ok(Reaped::Now);
    }
    if ms == 0 {
        return Ok(Reaped::Unsettled);
    }
    // 挂起等一记：醒来自收尾（`wipe`）或到点，两者当场分不开 ⇒ 醒来复探，判决只认它。
    let _ = runtime::env::unit::join(rep, ms);
    if crate::supervisor::system::call::running(rep) {
        Ok(Reaped::Unsettled)
    } else {
        Ok(Reaped::Waited)
    }
}

/// 这一行身子的代表线程（没有身子 = 没有可等的坐标）。
fn live_rep(table: &Table, name: Name) -> Option<TaskId> {
    match table.find(name) {
        Some(Service {
            slot: Slot::Live { rep, .. },
            ..
        }) => Some(*rep),
        _ => None,
    }
}

/// 盯着它：`true` = 它收尾了（`Now` / `Waited`）。
///
/// 内核的事实优先：它说收了就是收了，表随之落定 `Dead`——**坐标留着**（见 [`Slot`]：
/// 清了就没得放下、也没得重启）。`Unsettled`（有界期内没等出来）**一个字都不写**：
/// 那是"还没收干净"，不是"收了"。
/// `ms` = **上限族**（口径见 `env::fid` 文件头的定式）。
pub fn watch(table: &mut Table, name: Name, ms: usize) -> Result<bool, Fail> {
    match until(table, name, ms)? {
        Reaped::Now | Reaped::Waited => {
            table.set_state(name, State::Dead);
            Ok(true)
        }
        Reaped::Unsettled => Ok(false),
    }
}

/// 按名字找那一行的只读视图（枚举的入口）。
pub fn find(table: &Table, name: Name) -> Option<&Service> {
    table.find(name)
}

/// 监督循环：**发现死亡 + 记账 + 放下死域**（见编排域的头注第 5 条）。
///
/// 事件来自**板**：客人一死，它开的孔随退出钩子封印（或它自己说了退场）⇒ 板当场看出来
/// ⇒ 往**那一位的死亡道**里推一格 ⇒ 本线程从组上醒来。**一服务一道**，故"是哪一位"由
/// **哪条道响**给出——不必猜、也不会两条挤一格丢名字。
///
/// 醒来做两件事：先 `service::until` 等它真的收尾（板报的是"门封印了"，而 `Oust` 要的
/// 前置是"域里没有还没收尾的线程"——这一步等的是**事件**，不是节拍）；再写 `State::Dead`
/// （**不 `detach`**：坐标是"上一个实例"，留给重启与放下用）、`oust(team)` 放下那个死域、
/// 报一行。最后一条（单子最后一条）没了之后，对**仍在跑的**逐个 `stop`（`Ruin` = 域粒度
/// `Doom`）——它们的死会再走同一条路回来；在册的每一行都 `Dead` 之后才收场。
pub fn supervise(
    table: &mut Table,
    last: Name,
    lanes: &[Option<PieToken>],
    tole: &Tole,
    plan: &[Program],
) {
    let mut stopping = false;
    loop {
        // 等任一条道响。**`Tole` 的既定用法**（板那一轮同款）：**挂起过的那一侧返回的是
        // 预置值**——内核没有第二次执行机会，故醒来必须自己按组复核，不能靠返回值拿身份。
        if tole.await_(usize::MAX).is_err() {
            // 组坏了：退回"等最后一条退场"，行为与改动前一致。
            crate::supervisor::service::wait_last(table, last);
            return;
        }
        // 复核：每条道非阻塞地问一句"有货吗"。**单槽**——道上一次死亡只响一次；一次醒来
        // 可能带走多条（两位前后脚死）。
        for (i, lane) in lanes.iter().enumerate() {
            let Some(lane) = *lane else {
                continue;
            };
            let mut one = [0u8; 1];
            if HolePie::from_token(lane).pull_timeout(&mut one, 0).is_err() {
                continue; // 这一条没货
            }
            let Some(p) = plan.get(i) else {
                continue;
            };
            let Some(name) = Name::new(p.name).ok() else {
                continue;
            };
            account(table, name);
            // 最后一条走了 ⇒ 会话结束：把仍在跑的显式收掉（只下一次）。
            if name == last && !stopping {
                stopping = true;
                stop_running(table, plan);
            }
        }
        if stopping {
            // 收场：仍在跑的已经**有界地**下过一刀并等过（见 [`stop_running`]）；等不到的
            // 那些交给本域退场时的级联——那条路是既有的可靠收场路径，不在这里等。
            return;
        }
    }
}

/// 收场那一刀：给**仍在跑的**每一位 `stop`（`Ruin` = 域粒度 `Doom`），**有界地**等它
/// 收尾并记账；等不到就报一行，交给本域退场时的级联。
///
/// 为什么有界：`stop` 是"送到即回"（`kill` 的口径），收场不能被一个收不掉的域拖住。
pub fn stop_running(table: &mut Table, plan: &[Program]) {
    for q in plan.iter() {
        let Some(name) = Name::new(q.name).ok() else {
            continue;
        };
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
        match until(table, name, STOP_MS) {
            Ok(Reaped::Now) => mark_dead(table, name, Reaped::Now),
            Ok(Reaped::Waited) => mark_dead(table, name, Reaped::Waited),
            // 有界期内没等出来：照实报，交出这一位。**不是"没收到"**——判决只认非阻塞
            // 那一问，这里说的是"还没收干净"。
            Ok(Reaped::Unsettled) | Err(_) => {
                let _ = runtime::env::debug::put(&alloc::format!(
                    "system: stuck {} （退场级联接管）",
                    q.name
                ));
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
    let reaped = until(table, name, usize::MAX).unwrap_or(Reaped::Unsettled);
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
    let before = utask::heir_count().unwrap_or(0);
    let ousted = utask::oust(team).is_ok();
    let after = utask::heir_count().unwrap_or(0);
    let wait = match reaped {
        Reaped::Now => "now",
        Reaped::Waited => "waited",
        Reaped::Unsettled => "unsettled",
    };
    let _ = runtime::env::debug::put(&alloc::format!(
        "system: gone {} state=Dead ousted={ousted} heir={before}→{after} wait={wait}",
        name.as_str()
    ));
}
