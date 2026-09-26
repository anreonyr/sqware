//! system::desk — **账**：一张定长表与一行的形状（名字、身子、生命阶段、就绪凭据）
//!
//! 正文见 [`super`]；三档（判定 / 账 / 适配）分家的理由见 `system` 模块头注。

use alloc::vec::Vec;

use env::{Name, PieToken, TaskId, TeamId};

use super::core::{Fail, VestedBy};

// ── 核心：类型 ──────────────────────────────────────────────

/// Service 的生命阶段。**失败不在这里**——失败由 [`Fail`] 承载（两者是两件事）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    /// 表里有这一行，但还没起过。
    NeverStarted,
    /// 起了，正在等它宣布就绪。
    Starting,
    /// 已就绪。
    Ready,
    /// 已下令收，还没确认收干净。
    Stopping,
    /// 起过、现在是死的（被收掉或自己走的）。
    Dead,
}

/// **最近一次实例的坐标**：域 + 那一枚线程。生死看 [`State`]——`State::Dead` 与坐标并存
/// 是合法的（"起过、现在死了"），坐标留给重启与放下用：**死亡记账不清它**（清了就没得
/// 放下、也没得重启）。
///
/// **两格绑在同一个变体里**是刻意的：分开成两个字段就允许"有域、没线程"这种半死状态
/// 被写出来，而现在它不可表达。
///
/// `team = None` = **那一枚线程住本域**（iii：编排域的四枚线程里，除编排者自己以外那三枚）。
/// 这个 `None` 不是"省一格"：`team` 的**唯一读者**是 `mark_dead` 那一格（要"放下那个域"），
/// 而本域那一枚**没有别人的域可放下**——放下它就是扑杀本域自己（板线程那一格量过：
/// `system: done` 在 1005 份 soak 日志里一次都没有）。把"没有别人的域"写成 `None`，
/// 那一刀就写不出来。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Slot {
    None,
    Live { team: Option<TeamId>, task: TaskId },
}

/// **怎么知道它起来了**——定义与理由见 [`plan::assembly::Announce`]（本处只是转发，调用点不动）。
pub use plan::assembly::Announce;

/// 表里的一行。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Service {
    /// 服务名（清单名，≤ 31 字节）。**表内的唯一坐标**。
    pub name: Name,
    /// 这一次运行的载体。
    pub slot: Slot,
    /// 生命阶段。
    pub state: State,
    /// 怎么算"起来了"。
    pub announce: Announce,
    /// 它交回来的通道句柄（未交 = `None`）。`Announce::Channel` 的就绪证据就是它。
    pub root: Option<PieToken>,
}

/// 一行的初值（表是定长数组，故要一个可复制的空行）。
const EMPTY: Service = Service {
    name: Name::EMPTY,
    slot: Slot::None,
    state: State::NeverStarted,
    announce: Announce::None,
    root: None,
};

/// Service 表：**定长、线性查**。
///
/// 上限与程序清单同值（装不进机器的程序，也起不出服务）；三五个服务不需要哈希，
/// 也就省掉一层分配——本表可以在没有任何堆的宿主上存在。
pub struct Table {
    rows: [Service; Table::CAP],
}

impl Table {
    /// 行数上限（= `plan::manifest::MAX_PROGRAMS`）。
    pub const CAP: usize = 28;

    /// 空表：每一行都"占着位但没名字"。
    pub const fn new() -> Table {
        Table {
            rows: [EMPTY; Table::CAP],
        }
    }

    /// **登记一行**：只知道名字与它"怎么算起来"——此刻还没有身子。
    ///
    /// 这是"起之前"唯一的入口；身子由 [`Table::attach`] 在真的起了之后挂上。
    pub fn register(&mut self, name: Name, announce: Announce) -> Result<(), Fail> {
        if self.find(name).is_some() {
            return Err(Fail::Unknown);
        }
        let Some(row) = self.rows.iter_mut().find(|s| s.name.is_empty()) else {
            return Err(Fail::Full);
        };
        row.name = name;
        row.announce = announce;
        Ok(())
    }

    /// 按名字找那一行（没名字的行不算）。
    pub fn find(&self, name: Name) -> Option<&Service> {
        self.rows.iter().find(|s| s.name == name)
    }

    /// 全部有名字的行（含没起过的）——枚举的读面。
    pub fn rows(&self) -> impl Iterator<Item = &Service> {
        self.rows.iter().filter(|s| !s.name.is_empty())
    }

    /// 改状态。找不到 = 名字不对 ⇒ 不动任何东西。
    pub fn set_state(&mut self, name: Name, state: State) {
        if let Some(s) = self.row_mut(name) {
            s.state = state;
        }
    }

    /// 挂上身子：**一次给全**（域 + 线程）。没登记过 ⇒ `Unknown`。
    ///
    /// `team = None` = 那一枚线程**住本域**（见 [`Slot`]）。
    pub fn attach(&mut self, name: Name, team: Option<TeamId>, task: TaskId) -> Result<(), Fail> {
        let Some(s) = self.row_mut(name) else {
            return Err(Fail::Unknown);
        };
        s.slot = Slot::Live { team, task };
        s.root = None;
        s.state = State::NeverStarted;
        Ok(())
    }

    /// 摘掉身子与通道（行留着：状态要能说出"起过、现在死了"）。
    pub fn detach(&mut self, name: Name) {
        if let Some(s) = self.row_mut(name) {
            s.slot = Slot::None;
            s.root = None;
        }
    }

    fn row_mut(&mut self, name: Name) -> Option<&mut Service> {
        self.rows.iter_mut().find(|s| s.name == name)
    }
}

// ── 客人账 ──────────────────────────────────────────────────
//
// **两本并成一本**（照实记）：板那一本（`board/desk.rs`）与持树者那一本（`operator/desk.rs`）
// 原是**同一个 `Guest`、同一句"一位客人一格（谁 / 问 / 答）"逐字抄了两份**，只是一本用
// `[Option<Guest>; CAP = 16]`、一本用 `Vec`。并的理由不是"少一个文件"：**板那一本的上限史
// 正是树那一本踩过的那一脚**——定长上限在树那一侧撞满过三次（`8` → `12` → 还是满，
// `b1d1ae1` 才改 `Vec`），而板那一侧还停在 `CAP = 16` 上，靠 `programs/src/system/inner.rs`
// 末尾一条 `const _` 断言挡着。**同一课两个人各学一遍**就是漏；并成一本之后它只有一处可记。
//
// 两本原本的差别**全部落在手上**，不落在容器上（并完之后两处调用方各取所需）：
// `evict`（听来的那一档：客人说了 `EVICT`）只有板用；`arm_pending` / `sweep_each` 两边都用。

/// 这本账的两种不成——**每格一个不同的下一步**：**重放**不动账（接着办下一件事），
/// **满了**报一句（别静默丢一位客人）。
///
/// **照实记（这一格原来借的是别人的名字）**：`admit` 原先答板那一份 `Fail` 的 `NonEmpty`
/// ——那个名字在协议里说的是"**这块 Pane 非空**，要动它先清空"，与"这位客人已经在账上"
/// （重放，无事）**不是同一个下一步**。借名字的代价正是这一格：同一个码两种读法。
///
/// 它也**不是**协议那一份 `Fail`：这本账的失败**一句都不上线**（只有招待客人的那一枚线程
/// 自己听得见），故不必进那张双射码表（`fail_codes!` 加一格 = 加一个线上码）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DeskFail {
    /// 这一位已经在账上（重放）：**不动账**——换掉就把原来那位的问话孔丢了。
    Already,
    /// 备不下下一格（`try_reserve`）。
    Full,
}

/// 一位客人：**谁 / 问 / 答**。
///
/// `ask` 是 [`Option`]：客人"到了"（答话路到手）与"能问话了"（问话孔挂进来）是两步，
/// 中间隔着装配者转授与客人自己交孔那两段路——故 `None` 是**一个状态**（还没挂上问话孔），
/// 不是错误。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Guest {
    who: TaskId,
    ask: Option<PieToken>,
    reply: PieToken,
}

impl Guest {
    /// 哪位客人。
    pub fn who(&self) -> TaskId {
        self.who
    }

    /// 它的问话孔在**本表里的号**（`None` = 还没挂上）。
    pub fn ask(&self) -> Option<PieToken> {
        self.ask
    }

    /// 答话往哪儿走（本表里的号）。
    pub fn reply(&self) -> PieToken {
        self.reply
    }

    /// 这一格挂上问话孔了没有（`armed` 才可能被组唤醒）。
    pub fn armed(&self) -> bool {
        self.ask.is_some()
    }
}

/// 客人账：一叠格子，每格写着「谁 | 问在哪 | 答往哪」。
///
/// ```text
///   Admit        收一位客人（答话路到手）      —— 来客人了
///   Evict        客人说了"我走了"，撤它那一格  —— 与 admit 成对；只有板用（听来的那一档）
///   Arm          记下它的问话孔在本表里的号    —— 先 arm 才 attach
///   Guest        由问话孔的号直达那一格        —— 醒来时唯一要问的一句
///   Sweep        剔走已经答不出的格子          —— 惰性，不是轮询（看出来的那一档）
/// ```
///
/// **可增长**：不是定长数组——按需 `try_reserve`，备不下如实报 [`DeskFail::Full`]，不 `abort`。
/// 条数是策略、容器要有界，那一格落在**分配**上，不落在常数上。
pub struct Desk {
    guests: Vec<Option<Guest>>,
    vested_by: VestedBy,
}

impl Desk {
    /// 立一本账：**探活**跟着账走——它对每一格同值，故不必逐个作参数传。
    ///
    /// **不预分配**（`Vec::new()`）：这本账起手时只装几位客人——让 [`Desk::admit`] 按需
    /// `try_reserve` 那一格去报 `Full`，比这里先按一个猜的数占一片更诚实。
    pub const fn new(vested_by: VestedBy) -> Desk {
        Desk {
            guests: Vec::new(),
            vested_by,
        }
    }

    /// 收一位客人（答话路到手时叫）。返它的格子号。
    ///
    /// 两种不成见 [`DeskFail`]：**满**与**这一位已经在账上**（后者是提示那条单槽路上的重放：
    /// 一位客人只能占一格，重来的那位要**报出来**，不能静默换掉原来那位——换掉就把它的
    /// 问话孔丢了）。
    pub fn admit(&mut self, who: TaskId, reply: PieToken) -> Result<usize, DeskFail> {
        if self.guests.iter().flatten().any(|g| g.who == who) {
            return Err(DeskFail::Already);
        }
        // 先找空格；没有就**新开一格**——备不下如实报 `Full`。
        match self.guests.iter().position(|cell| cell.is_none()) {
            Some(slot) => {
                self.guests[slot] = Some(Guest {
                    who,
                    ask: None,
                    reply,
                });
                Ok(slot)
            }
            None => {
                self.guests.try_reserve(1).map_err(|_| DeskFail::Full)?;
                self.guests.push(Some(Guest {
                    who,
                    ask: None,
                    reply,
                }));
                Ok(self.guests.len() - 1)
            }
        }
    }

    /// 撤这一位客人的格子，返**撤掉的格子号**；不在账上 ⇒ `None`。
    ///
    /// 与 [`Desk::admit`] 成对：一位客人一格，进来一格、走了一格。**只动账**——把它的问话孔
    /// 从组里摘掉那一手不归它（组不在这一层）。
    ///
    /// 与 [`Desk::sweep`] 的分工：这一句撤的是**客人自己说了走**的那一格（听来的），
    /// `sweep` 剔的是**那一枚答不出**的那一格（看出来的）——故两句都在。
    pub fn evict(&mut self, who: TaskId) -> Option<usize> {
        let (slot, cell) = self
            .guests
            .iter_mut()
            .enumerate()
            .find(|(_, cell)| cell.as_ref().is_some_and(|g| g.who == who))?;
        *cell = None;
        Some(slot)
    }

    /// 记下"这位客人的问话孔是**本表里的哪一枚**"。返**成不成**。
    ///
    /// **两桩不成合成一格**（号不在账上 / 这一格已经挂着一枚）：调用方的下一步相同——这一格
    /// 这一轮不 arm。**先 `arm` 才 `attach`**：`arm` 之后的号才会进组，故 [`Desk::guest`]
    /// 在"被组唤醒的那一枚"上**永不为 `None`**。
    pub fn arm(&mut self, slot: usize, ask: PieToken) -> bool {
        let Some(guest) = self.guests.get_mut(slot).and_then(Option::as_mut) else {
            return false;
        };
        if guest.ask.is_some() {
            return false;
        }
        guest.ask = Some(ask);
        true
    }

    /// 摘掉这一格挂的问话孔（换孔或退场时用）；本就没挂即无事。返**成不成**（同
    /// [`Desk::arm`]：号不在账上 = 不成，而调用方那一侧同样无事可做）。
    pub fn unarm(&mut self, slot: usize) -> bool {
        let Some(guest) = self.guests.get_mut(slot).and_then(Option::as_mut) else {
            return false;
        };
        guest.ask = None;
        true
    }

    /// 这枚号是哪位客人的（醒来时唯一要问的一句）。认不出返 `None`——**不是失败**
    /// （提示孔那一路也叫醒同一次等待，它的号自然不在这本账上）。
    pub fn guest(&self, ask: PieToken) -> Option<&Guest> {
        self.guests.iter().flatten().find(|g| g.ask == Some(ask))
    }

    /// 还没挂上问话孔的那几格：`(格子号, 谁)`——**只读**那一半。
    pub fn unarmed(&self) -> impl Iterator<Item = (usize, TaskId)> + '_ {
        self.guests
            .iter()
            .enumerate()
            .filter_map(|(slot, cell)| cell.as_ref().filter(|g| !g.armed()).map(|g| (slot, g.who)))
    }

    /// 还没挂上问话孔的那几格——**改得动**那一半：每格叫一次 `ask_of`，挂得上就 arm 并
    /// `attach`；返"**还有没有没补齐的**"。
    ///
    /// **为什么这两手（`ask_of` / `attach`）要收进来**：调用方今天得先"抄一份'还没挂上的'"
    /// 到自己的栈上（`unarmed()` 借住这本账，而循环里要改它）；那一抄就是**一份按客人数的
    /// 分配**，或者**按一个常数开的数组**——后者正是这本账放弃的那件事（见上面的并本记）。
    /// 收进来之后调用方一格缓冲都不需要：遍历按**格子号**走，`arm` / `unarm` 都在本账里。
    ///
    /// **次序是硬的**：先 `arm` 再 `attach`；两步任一步不成 ⇒ `unarm` 回退（本就不挂即无事）
    /// 并报"还没补齐"。
    pub fn arm_pending(
        &mut self,
        mut ask_of: impl FnMut(TaskId) -> Option<PieToken>,
        mut attach: impl FnMut(PieToken) -> bool,
    ) -> bool {
        let mut pending = false;
        for slot in 0..self.guests.len() {
            let Some((who, armed)) = self.guests[slot].as_ref().map(|g| (g.who, g.armed())) else {
                continue;
            };
            if armed {
                continue;
            }
            match ask_of(who) {
                Some(ask) => {
                    let hung = self.arm(slot, ask) && attach(ask);
                    if !hung {
                        let _ = self.unarm(slot);
                        pending = true;
                    }
                }
                None => pending = true,
            }
        }
        pending
    }

    /// 账上还有几位。
    pub fn occupied(&self) -> usize {
        self.guests.iter().flatten().count()
    }

    /// 剔走**已经答不出**的客人，返剔了几格；幂等。
    ///
    /// 判据是注入的那一格（在这两棵树里 = [`VestedBy`]：**客人答话路那一枚还答得出吗**）——
    /// 那一枚答 `None`（不在我表里，**或**它那扇门已经封印）就剔。**看出来的**那一档；
    /// **听来的**那一档是 [`Desk::evict`]（客人自己说了走，账当场撤，不等它的门封印）。
    /// 两档都在，因为没说就走的那种也得有人收。
    pub fn sweep(&mut self) -> usize {
        self.sweep_each(|_| {})
    }

    /// 与 [`Desk::sweep`] **同判据**，但每剔一位叫一次 `f`（**趁它还认得出**）。
    ///
    /// 板要用这个号去做第二件事：**推那一位的死亡道**。号只在这里拿得到——客人一旦退场，
    /// 它挂在板上的牌子随时会被摘掉，摘了就认不出"这一位叫什么"（道的记号是名字）。
    /// 有了它，调用方那一侧那个 `out: &mut [TaskId]` 出口缓冲（按常数开的那一张）就不必存在了。
    pub fn sweep_each(&mut self, mut f: impl FnMut(TaskId)) -> usize {
        let vested_by = self.vested_by;
        let mut gone = 0;
        for cell in self.guests.iter_mut() {
            if let Some(guest) = cell
                && vested_by(guest.reply).is_none()
            {
                f(guest.who());
                *cell = None;
                gone += 1;
            }
        }
        gone
    }
}
