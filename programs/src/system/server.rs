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
//!
//! # 监督相已分出去（照实记：本文件原来还装着那一半）
//!
//! 收场那一半（那圈循环、死亡道那一页、记账与放下死域）搬去了 [`supervise`]。分家的判据是
//! **两相**：本文件全是**装配期**的事（建域 / 放行 / 等就绪 / 收一枚），而监督是**起完之后
//! 一直看**（直到名册上每一位都 `Dead`）——它们之间只有两处来往：表里那几格状态，与 [`stop`]
//! / [`until`] 那两枚原语。`main` 的流程本来就写着这两步（先 [`assemble`](crate::service::assemble)，
//! 再监督）。
//!
//! 顺手收掉两枚没有读者的 `pub`（`find` / `watch`）——与 `service::die` / `E_OK` 那一刀同一条
//! 纪律：`pub` 也是要有人读的。

use env::Wait;
use env::{EnvError, Fail as EnvFail, Mark, Name, ProgramKind, Reason, TaskId, TeamId};
use runtime::core::exit::Report;
use runtime::env::unit as utask;

use protocol::session::Quay;

use protocol::system::core::{Fail, Ready, Reaped, admit_start, probe_ready};
use protocol::system::desk::{Announce, Service, Slot, State, Table};

use crate::service::Role;
use plan::assembly::{E_COALITION, E_PRINCIPAL, E_TREE};

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

// ── 内件那一族的起手失败（照实记：三份同构的 `fail.rs` 已并成这一枚）─────────
//
// **为什么能并**：那三份 `operator/fail.rs` / `principal/fail.rs` / `coalition/fail.rs` 是**同一
// 件事的三份写法**——各自的 `Fail` + `code()` + `text()` + `impl Exit`，总共 143 行里只有变体名
// 与文案不同；而三者都实现 `Exit`、都被 `main` 折成同一个出口。那正是"形状相同"的机械证据。
//
// **为什么之前那三份是错的**：每一份各从 `1` 起编号 ⇒ 同一台服务"死在起手"有**两套号**在跑：
// 装配期那一条（`service::{mint,spawn_here}` / `assemble`，答 `Plan::died`）与 `serve()` 自己那
// 一条（答 1..5）。`operator` 的 `Sire = 1` 甚至与 `E_BOOT` 撞号。现在**号一律取自装配单**
// （`plan::assembly` 那一套，含内件三枚 10/14/16）⇒ `serve()` 与装配期说的是同一句话。
//
// **留下的几格只在"各域真的做过的事"上分**：`Book`（立两张表）只有名册有、`Face`（按名字找
// 身份那份门牌）只有盟册有、`Room`（收帧那一页）只有持树者有；`Sire` / `Board` / `Entry` 三台
// 同款，`Tree` 把"铸提示孔"与"上树"这两步**同一个来路**收成一格，`Desk` 收"那只组 + 收帧那
// 一页"。

/// 内件**起手**失败：三枚共用（`Role::System` 那一枚另有 [`super::main::Fail`]）。
///
/// **名字为什么不叫 `Fail`**：这一族里 `operator/server.rs` 已经 `use
/// protocol::system::operator::{…, Fail}`（那是**核心**的失败域：`Unknown` / `Dead` /
/// `NotATile`……），两个 `Fail` 在同一份文件里撞名。起手这几格与核心那几格不是一回事，
/// 故按"死在起手的哪一步"取名 [`Start`]。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Start {
    /// 认不出"起我那一枚线程"（`service::assembler`）——`Spawn` 那一格 `args` 没递对。
    Sire,
    /// 上板那一步（`board::open` / `ask_hole`）。
    Board,
    /// 树那一步：持树者铸提示孔交给装配者 / 名册与盟册分目录 + 落门牌 + 回查。
    Tree,
    /// 本域自己开的那一枚孔（服务入口记号 [`protocol::system::board::ENTRY_MARK`]）。
    Entry,
    /// 起手要备的那两样备不下：持树者**收帧那一页**（`Pile` 之外的那一样）。
    Room,
    /// 常驻那一问：那只组没立起来，或收帧那一页备不下。
    Desk,
    /// 身份服务的两张表（谱系 + 名册）没立起来；**只有名册那一台有**。
    Book,
    /// 身份服务那份门牌找不到（盟册是它的客人，**按名字找**）；**只有盟册有**。
    Face,
    /// **常驻期**：组坏了（`await_` 答不出）——与 [`Start::Desk`] 分开，是因为它不在"起手那
    /// 几步"里（起手已经过完了），而号同那一族。
    Dead,
}

impl Start {
    /// **号一律取自装配单**（[`Died`](plan::assembly::Died)）——本域自己的死法**一个数都不写**。
    pub fn code(self) -> Reason {
        match self {
            Start::Sire => E_TREE,
            Start::Board => E_TREE,
            Start::Tree => E_TREE,
            Start::Room => E_TREE,
            Start::Entry => E_PRINCIPAL,
            Start::Book => E_PRINCIPAL,
            Start::Desk => E_COALITION,
            Start::Face => E_COALITION,
            Start::Dead => E_COALITION,
        }
    }

    pub fn text(self) -> &'static str {
        match self {
            Start::Sire => "operator: no sire",
            Start::Board => "operator: board",
            Start::Tree => "operator: tree",
            Start::Entry => "principal: entry",
            Start::Room => "operator: no room",
            Start::Desk => "coalition: desk",
            Start::Book => "principal: no book",
            Start::Face => "coalition: no identity plate",
            Start::Dead => "inner: group dead",
        }
    }
}

/// **内件那一支的出口格**（`main` 那三个分支 `map_err` 的就是它）。
///
/// 与 [`super::main`] 的 `said` 同一形状：自己拼一格 `Report<'static>`（不叫 `Exit::report` 的
/// 理由见那一处）。
pub fn said(f: Start) -> Report<'static> {
    Report::note(f.code(), f.text())
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
        if !marks.is_empty()
            && marks
                .iter()
                .all(|mark| q.claim(task, *mark, millis).is_ok())
        {
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
///
/// **读者只有一处**（[`crate::service::wait_last`]）：跟着 [`until`] 一起放进本模块的公面，
/// 是因为它属于"等一条服务退场"这一族（`find` 那一枚没有读者，已删）。
pub fn watch(table: &mut Table, name: Name, millis: Wait) -> Result<bool, Fail> {
    match until(table, name, millis)? {
        Reaped::Now | Reaped::Waited => {
            table.set_state(name, State::Dead);
            Ok(true)
        }
        Reaped::Unsettled => Ok(false),
    }
}
