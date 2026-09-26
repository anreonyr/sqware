//! system::server — **适配**：把内核答的事实写回表（建域 / 放行 / 等就绪 / 收 / 盯）
//!
//! 正文见 [`super`]；三档（判定 / 账 / 适配）分家的理由见 `system` 模块头注。
//!
//! # 内核那一侧也在这里（照实记：原 `system/call.rs` 已收进本文件）
//!
//! 那七手（建域 / 产线程 / 塞门闩 / 放行 / 收域 / 判收尾）原来另立一个 `call.rs`，而它们的
//! **唯一读者就是本文件**——分家分错了地方：本文件的名分本来就是"适配"（把内核答的事实
//! 写回表），那几手正是这句话的孩子。收进来之后每条链子少一跳转发，也不再有一层"转发壳"
//! 夹在适配与内核之间。
//!
//! **一处语义翻译留在这里**（[`fail`]）：内核负码 → 本协议的 [`Fail`]。它在 `env` 的词汇
//! （[`EnvFail`]）与本协议的词汇之间过一手，故它住适配层、不进「约」（那要给 `contract`
//! 加一条依赖）。码本身读 [`EnvFail::of_code`]——**一个数字都不写**。

use env::Wait;
use env::{EnvError, Fail as EnvFail, Mark, Name, ProgramKind, TaskId, TeamId};
use runtime::core::pile::Pile;
use runtime::env::mail::HolePie;
use runtime::env::unit as utask;

use protocol::session::Quay;

use protocol::system::core::{Fail, Ready, Reaped, admit_start, probe_ready};
use protocol::system::desk::{Announce, Service, Slot, State, Table};

use crate::service::{Lane, Role};

// ── 内核那一侧（原 `system/call.rs`）：每个函数只做一件事——转发一次 ──────
//
// 这几手原来住 `crate::system::call`。它们**只有本文件一个读者**，故收在这里；
// 判断一律不在这里（那在协议那一侧的判定与账里），下面每一枚都是一行转发。
//
// **"它交回了一枚孔"那件事不在这里**——归 `protocol::session`：会话的建立与认领是另一份
// 协议，本处只剩"起一个服务"需要的那几手。

/// 建域（Mint）：镜像字节 + 特权级 → 新域。
///
/// 产出的域归**调用者**（父亲 = 调用者自己）。特权级由**清单**决定、调用方转交
/// ——程序自称不了特权级（这是"放开建域不构成提权"的那一半）。
///
/// **名字不过这里**：清单名归装配账（`system::desk`），内核不收名字。
pub fn build(image: &[u8], kind: ProgramKind) -> Result<TeamId, Fail> {
    utask::build(image, kind).map_err(fail)
}

/// 产一枚线程（未放行）：`entry = 0` ⇒ 走域默认入口。
///
/// `args` 是 `Spawn` 的那一格（内核拷到新任务栈顶，子方用 `runtime::core::unit::args()` 读）。
/// 今天只有一处非空：iii 的三枚内件用**同一个入口**、靠这一格分派角色（[`crate::service::Role`]）。
pub fn spawn(team: TeamId, args: &[usize]) -> Result<TaskId, Fail> {
    utask::spawn(team, 0, args, 0).map_err(fail)
}

/// 把一枚门闩塞进目标线程手里（放行前做）。**给多大权由调用方定**——这里不替它做主。
///
/// **收的是"交成了"这一格**：内核那手回的是"它种在目标表里那一号"，本层没有读者
/// （装配期要认的是**别人**交上来的孔，不是自己塞进去的那枚号）——故 `map(|_| ())`。
pub fn accord(token: env::PieToken, task: TaskId, perm: env::Permission) -> Result<(), Fail> {
    runtime::env::mail::accord(token, task, perm)
        .map(|_| ())
        .map_err(fail)
}

/// 放行。
pub fn hatch(task: TaskId) -> Result<(), Fail> {
    utask::hatch(task).map_err(fail)
}

/// 收掉目标所属的域（连它的线程一起）。**`doom`：不靠血缘。**
///
/// 判活照旧：目标不在世 / 从未入册 ⇒ 已经是死的，视作收到（幂等）。
pub fn doom(task: TaskId) {
    let _ = runtime::env::room::doom(task);
}

/// 它收尾完了没有（非阻塞那一问：`POLL`）。
pub fn reaped(task: TaskId) -> bool {
    utask::join(task, Wait::POLL).unwrap_or(true)
}

/// 内核负码 → 本协议的失败域（按"调用方接下来干什么"分，不按内核哪一步坏了）。
///
/// **码读 `env` 的词汇**（[`EnvFail::of_code`]）：本层出现数字就等于把码表抄成第二处。
/// 三格的来路：`+` 装不上 = [`EnvFail::BadImage`]（`-6`）；`+` 内存不够 / 产不出来 =
/// [`EnvFail::OoM`]（`-4`，本协议名 [`Fail::Full`]）；其余（含表外那一格 `None`）
/// 落 [`Fail::Unknown`]——"不认识的失败"不该猜成某一种。
fn fail(e: erra::Error<EnvError>) -> Fail {
    match EnvFail::of_code(e.source.code()) {
        Some(EnvFail::BadImage) => Fail::BadImage,
        Some(EnvFail::OoM) => Fail::Full,
        _ => Fail::Unknown,
    }
}

// ── 适配：原语（把内核那一侧与表拼起来）──────────────────────

/// 起跑前要交出去的一枚门闩——定义见 [`plan::assembly::Grant`]（本处只是转发）。
pub use plan::assembly::Grant;

/// 起一个 Service。
///
/// `image` = 镜像字节（v1 口径）；`kind` = 特权级（**由清单决定**，调用方转交）；
/// `grants` = **放行前**要交到它手里的门闩（空 = 什么都不预先给）；
/// `quay` = 与它的会话（`Announce::Channel` 那一种才有：它就绪的凭据长在这里）；
/// `millis` = 就绪等待（上限族，`Wait`）。
///
/// **失败时不留下半行**：产线程**之前**失败 ⇒ 表不动；产线程**之后**失败 ⇒ 实例与
/// 状态都如实留在表里（它确实在跑），调用方用 [`stop`] 收尾。
pub fn mint(
    table: &mut Table,
    name: Name,
    image: &[u8],
    kind: ProgramKind,
) -> Result<TaskId, Fail> {
    admit_start(table, name)?;

    let team = build(image, kind)?;
    let Ok(task) = spawn(team, &[]) else {
        return Err(Fail::Full);
    };
    table.attach(name, Some(team), task)?;
    table.set_state(name, State::Starting);
    Ok(task)
}

/// **本域起一枚内件**（iii：编排域的四枚线程里，除编排者自己以外那三枚）。
///
/// 与 [`mint`] 是**同一条路的后半段**，差别只在**身子从哪来**：不建域，就在**我自己的域**里
/// 产一枚线程（`TeamId(0)` = 当前域，见 `UnitCall::Spawn`），角色由 `args` 那一格递过去
/// （[`Role::args`]；读它的是同一份 ELF 里的 `main`）。
///
/// `team` 记 **`None`**：那一行背后**没有别人的域**——`team` 唯一的用途是"放下那个域"
/// （见 [`Slot`]），而本域那一枚无域可放。
pub fn spawn_here(table: &mut Table, name: Name, role: Role) -> Result<TaskId, Fail> {
    admit_start(table, name)?;

    let Ok(me) = utask::self_id() else {
        return Err(Fail::Unknown);
    };
    let Ok(task) = spawn(TeamId::new(0), &role.args(me.get())) else {
        return Err(Fail::Full);
    };
    table.attach(name, None, task)?;
    table.set_state(name, State::Starting);
    Ok(task)
}

/// 第二相：**放行**，并在放行前塞门闩、定会话。
///
/// `grants` = 放行前要交到它手里的门闩（空 = 什么都不预先给）；`quay` = 与它的会话
/// （`Announce::Channel` 那一种要用：它就绪的凭据长在这里）；`marks` = 放行后要逐条
/// 认领的**记号**（顺序无关；记号即泊位名，装配单给的通道名就是它）；`millis` = 就绪等待
/// （上限族，`Wait`）。
///
/// **两相之间的窗口就是"它一步都还没跑"**——塞门闩、定会话都发生在这段窗口里，这与
/// 载体的"恒产未放行"是同一条。
///
/// **失败时不留下半行**：`spawn` 失败 ⇒ 表不动；本相失败 ⇒ 实例与状态如实留在表里
/// （它确实在跑），调用方用 [`stop`] 收尾。
///
/// # Errors
/// 见 [`Fail`]。
/// `millis` = **上限族**（`Wait`，口径见 `env::fid` 文件头的定式）——超时与"成立了"
/// 按返回值区分。
pub fn start(
    table: &mut Table,
    name: Name,
    task: TaskId,
    grants: &[Grant],
    quay: Option<&mut Quay>,
    marks: &[Mark],
    millis: Wait,
) -> Result<(), Fail> {
    let launched = (|| -> Result<(), Fail> {
        for g in grants {
            accord(g.token, task, g.perm)?;
        }
        hatch(task)
    })();
    if let Err(e) = launched {
        doom(task);
        table.detach(name);
        table.set_state(name, State::Dead);
        return Err(e);
    }
    ready(table, name, quay, marks, millis)?;
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
    marks: &[Mark],
    millis: Wait,
) -> Result<bool, Fail> {
    // 先看表：上一次问过的事实（按这一行自己声明的说法解读）。
    if let Ready::Up = probe_ready(table, name) {
        return Ok(true);
    }
    let Some(Service {
        slot: Slot::Live { task, .. },
        announce,
        ..
    }) = table.find(name)
    else {
        return Err(Fail::NotReady);
    };
    let (task, announce) = (*task, *announce);

    // 它不宣布的那一种：放行之后只要还活着就算起来了，没有可等的东西。
    if announce == Announce::None {
        if !reaped(task) {
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
        if !marks.is_empty() && marks.iter().all(|mark| q.claim(task, *mark, millis).is_ok()) {
            table.set_state(name, State::Ready);
            return Ok(false);
        }
    }
    if reaped(task) {
        table.detach(name);
        table.set_state(name, State::Dead);
        return Err(Fail::NotReady);
    }
    // 还活着、只是没宣布：它确实在跑，**实例如实留在表里**（调用方用 `stop` 收尾）。
    if millis == Wait::POLL {
        return Ok(false);
    }
    Err(Fail::NotReady)
}

/// 收掉一个 Service。**下令即回，不等它收完**。
pub fn stop(table: &mut Table, name: Name) -> Result<(), Fail> {
    let Some(Service {
        slot: Slot::Live { task, .. },
        ..
    }) = table.find(name)
    else {
        return Err(Fail::Unknown);
    };
    let task = *task;
    doom(task);
    table.set_state(name, State::Stopping);
    Ok(())
}

/// 等它收尾：`millis` 与 [`ready`] 同款（上限族，`Wait`）。
/// **只读：不动表**。
///
/// 形状是 **问 → 等 → 问**，判决只认两次**非阻塞问**（`Join{task, 0}`）；等只是为了少问几次。
/// `Join{task, millis}` 挂起过之后的返回值不含信息（见 [`Reaped`]），故醒来必须复探——**"他杀
/// 偶发不生效"那条错读就是漏了这一步**：把"醒过"当成了"没收到"。
///
/// 为什么有界等不需要 clock、也不必睡满那一格：`WakeKey::Task{id}` 上的投信方只有 `wipe`，
/// 而它只在 `bury` 里、`Reaped` 置位**之后**调（`kernel/src/work/room/messenger/reap.rs`）
/// ⇒ 等待里的"早醒"只可能来自收尾；"到点"那一支由复探分出来（答 [`Reaped::Unsettled`]）。
///
/// `Err(Fail::Unknown)` = 表里没这一行、或这一行还没有身子的坐标。问不出（`Denied` =
/// 已入土 / 从未入册）按"收尾了"处理——与 [`reaped`] 同一折法。
/// `millis` = **上限族**（`Wait`，口径见 `env::fid` 文件头的定式）；超时那支答
/// [`Reaped::Unsettled`]，
/// 不写表。
pub fn until(table: &Table, name: Name, millis: Wait) -> Result<Reaped, Fail> {
    let Some(task) = live_task(table, name) else {
        return Err(Fail::Unknown);
    };
    if reaped(task) {
        return Ok(Reaped::Now);
    }
    if millis == Wait::POLL {
        return Ok(Reaped::Unsettled);
    }
    // 挂起等一记：醒来自收尾（`wipe`）或到点，两者当场分不开 ⇒ 醒来复探，判决只认它。
    let _ = runtime::env::unit::join(task, millis);
    if reaped(task) {
        Ok(Reaped::Unsettled)
    } else {
        Ok(Reaped::Waited)
    }
}

/// 这一行身子那一枚线程（没有身子 = 没有可等的坐标）。
fn live_task(table: &Table, name: Name) -> Option<TaskId> {
    match table.find(name) {
        Some(Service {
            slot: Slot::Live { task, .. },
            ..
        }) => Some(*task),
        _ => None,
    }
}

/// 盯着它：`true` = 它收尾了（`Now` / `Waited`）。
///
/// 内核的事实优先：它说收了就是收了，表随之落定 `Dead`——**坐标留着**（见 [`Slot`]：
/// 清了就没得放下、也没得重启）。`Unsettled`（有界期内没等出来）**一个字都不写**：
/// 那是"还没收干净"，不是"收了"。
/// `millis` = **上限族**（`Wait`，口径见 `env::fid` 文件头的定式）。
pub fn watch(table: &mut Table, name: Name, millis: Wait) -> Result<bool, Fail> {
    match until(table, name, millis)? {
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
/// 报一行。最后一条（单子最后一条）没了之后，对**仍在跑的**逐个 `stop`（`doom` = 域粒度
/// `Doom`）——它们的死会再走同一条路回来；在册的每一行都 `Dead` 之后才收场。
pub fn supervise(table: &mut Table, last: Name, lanes: &[Lane], pile: &Pile) {
    // 死亡道那一格：**一页**——与门那一侧同一条规则（谁能往里推，缓冲就按**载体**的界备，
    // 不按"这条路上平常走几个字节"备）。一枚更长的推落进道里时，1 字节的读法取不出也丢不掉，
    // 那一位的死就永远记不上账。备不下 ⇒ 报一句就交给退场时的级联（那条路本来就是可靠收场
    // 路径，见本函数尾注），不在这里赌。
    let mut lane_buf: alloc::vec::Vec<u8> = alloc::vec::Vec::new();
    if lane_buf.try_reserve_exact(runtime::PAGE_SIZE).is_err() {
        let _ = runtime::env::debug::put("system: no room");
        return;
    }
    lane_buf.resize(runtime::PAGE_SIZE, 0);
    let mut stopping = false;
    loop {
        // 等任一条道响。**`Pile` 的既定用法**（板那一轮同款）：**挂起过的那一侧返回的是
        // 预置值**——内核没有第二次执行机会，故醒来必须自己按组复核，不能靠返回值拿身份。
        if pile.await_(Wait::Forever).is_err() {
            // 组坏了：退回"等最后一条退场"，行为与改动前一致。
            crate::service::wait_last(table, last);
            return;
        }
        // 复核：每条道非阻塞地问一句"有货吗"。**单槽**——道上一次死亡只响一次；一次醒来
        // 可能带走多条（两位前后脚死）。
        for lane in lanes {
            let Some(road) = lane.road else {
                continue;
            };
            if HolePie::from_token(road).pull_timeout(&mut lane_buf, Wait::POLL).is_err() {
                continue; // 这一条没货
            }
            // **道自己带着名字**（不用下标去装配单里翻：见 [`Lane`] 那段照实记）。
            let Ok(name) = Name::new(lane.name) else {
                continue;
            };
            account(table, name);
            // 最后一条走了 ⇒ 会话结束：把仍在跑的显式收掉（只下一次）。
            if name == last && !stopping {
                stopping = true;
                stop_running(table, lanes);
            }
        }
        if stopping {
            // 收场：仍在跑的已经**有界地**下过一刀并等过（见 [`stop_running`]）；等不到的
            // 那些交给本域退场时的级联——那条路是既有的可靠收场路径，不在这里等。
            return;
        }
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
        // **本域那一枚不在这里收**：它的"域"就是本域，收它就是扑杀本域自己（板线程那一格
        // 量过）。它随本域退场时的"域亡＝成员清零"一起走。
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
                let _ = runtime::env::debug::put(&alloc::format!(
                    "system: stuck {} （退场级联接管）",
                    lane.name
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
    let before = utask::heir_count().unwrap_or(0);
    // **本域那一枚没有别人的域可放下**（`team = None`）：放下它就是扑杀本域自己——板线程
    // 那一格量过（`system: done` 在 1005 份 soak 日志里一次都没有）。那一枚随"域亡＝成员
    // 清零"一起走，故这里什么都不做，读数照实说 `inner`。
    let ousted = match team {
        Some(team) => utask::oust(team).is_ok(),
        None => false,
    };
    let after = utask::heir_count().unwrap_or(0);
    let wait = match reaped {
        Reaped::Now => "now",
        Reaped::Waited => "waited",
        Reaped::Unsettled => "unsettled",
    };
    let _ = runtime::env::debug::put(&alloc::format!(
        "system: gone {} state=Dead ousted={ousted} heir={before}→{after} wait={wait}{}",
        name.as_str(),
        if team.is_none() { " inner" } else { "" }
    ));
}
