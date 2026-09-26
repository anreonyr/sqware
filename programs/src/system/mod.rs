//! system — **编排域的实现侧**：一个容器（[`System`]）＋ 它容纳的那几件。
//!
//! ```text
//!   System
//!   ├── Control        Service 的创建 / 配置 / 启动 / 停止（不认协议）
//!   ├── operator       持树者那一侧（树那条边 + 提示之路）
//!   ├── principal      名册那一侧（身份面 + 派号 / 绑号）
//!   ├── coalition      盟册那一侧（协调帧的第二格）
//!   └── board          板（死亡道那一侧）＋ 监督相收着的道与组
//! ```
//!
//! **边住这里**：谁上板 / 谁上树 / 谁是持树者 / 它是哪一双眼睛 / 装配期给不给身份——
//! 这是**装配单上那几格**（[`plan::assembly::Plan`]），本文件按序把它们落到四套协议
//! 各自的那一手上。Control 只看得到"一枚线程 ＋ 它的码头"，**一个协议名字都不认**。
//!
//! **三枚服务（持树者 / 名册 / 盟册）是普通程序**：各自一个 bin、一个域，由本域按装配单
//! 用与其他每一台相同的 `mint` 起起来。iii 那套"与编排者共一份字节、靠 `Role` 按 args
//! 分派、同域产线程"的做法连同 `Role` / `Source` / `spawn_here` 一起退了。
//!
//! - [`assemble`]：这一景起哪些台、每台要什么——**scenario**（不再有第二张关系表）
//! - [`bootstrap`]：启动资源获取（与引导域会话 / 机器自述 / 载荷区清单）
//! - [`control`]：Service 的生命周期（内核那几手住 `control::service`，监督相住 `control::supervise`）
//! - [`program`]：静态节点的词汇（`Program` / `Setup`）
//! - [`board`] / [`operator`] / [`principal`] / [`coalition`]：四枚服务的实现
//! - [`machine`]：本域手里那台机器的自述（设备树）

use core::time::Duration;

use alloc::vec::Vec;

use env::{HoleDir, Mark, Name, PieToken, TaskId, Wait};
use plan::assembly::{Eyes, Plan, Row};
use protocol::debug;
use protocol::session::call as scall;
use protocol::system::board as bcall;
use protocol::system::board::LANE_PREFIX;
use protocol::system::principal::client::Face;
use protocol::system::principal::core::PrincipalId;
use runtime::core::pile::Pile;
use runtime::env::mail::{self, HolePie};
use runtime::env::room;
use runtime::env::unit as utask;

use crate::system::board::bridge as board_bridge;
use crate::system::bootstrap::Boot;
use crate::system::control::supervise::{self, Lane};
use crate::system::control::{Control, Died, Error, READY_MS, RETRY_MS, Service};
use crate::system::operator::bridge as operator_bridge;
use crate::system::program::{Program, Setup};

pub mod assemble;
pub mod board;
pub mod bootstrap;
pub mod coalition;
pub mod control;
pub mod machine;
pub mod operator;
pub mod principal;
pub mod program;

/// **编排域自己**：Service 的生命周期机器 ＋ 四套协议的组装态 ＋ 监督那两样（道表 / 组）。
pub struct System {
    /// Service 的生命周期机器。
    control: Control,
    /// 死亡道：上板的那几位一位一条（记号 `gone-<名字>`），监督相逐条看。
    lanes: Vec<Lane>,
    /// 等任一条道响的组（监督相用）。
    pile: Pile,
    /// 持树者那一枚线程的号（`None` = 还没起）。
    tree: Option<TaskId>,
    /// 持树者那条提示之路在**本线程表里**的那一枚（客人上树时逐条推号）。
    otip: Option<PieToken>,
    /// 板那条提示之路在**本线程表里**的那一枚。
    btip: Option<PieToken>,
    /// 身份那一面：名册放行之后本域就有，此后每条服务的身份都从它来。
    face: Option<Face>,
    /// **协调那一帧**要带的两位（名册 / 盟册）：各自那一枚起来之后占上那一格。
    coord: operator_bridge::Coord,
}

impl System {
    /// 就位：**铸死亡道**（上板的那几位一位一条）→ 建 Control。
    ///
    /// 失败（那只组立不起来 / 备不下道表）由调用方折成 `system: no group`。
    pub fn new(boot: Boot, rows: &[&'static Row]) -> Result<System, ()> {
        // 组是**独占**的（`shared = false`）：本线程用它等任一道响（零轮询）。
        let pile = Pile::unseal(false).map_err(|_| ())?;
        let mut lanes: Vec<Lane> = Vec::new();
        lanes.try_reserve(rows.len()).map_err(|_| ())?;
        for row in rows {
            // 记号 = `LANE_PREFIX` ＋ 名字：**前缀只有一处定义**（板那一侧按同一个常量
            // 拼出来找它）。**不上板的那几位不铸道**：没有写端的道永远不会响。
            let board = row.plan.as_ref().map(|p| p.board).unwrap_or(false);
            let road = if board {
                mail::unseal_hole(Mark::of(&alloc::format!("{LANE_PREFIX}{}", row.name))).ok()
            } else {
                None
            };
            if let Some(road) = road {
                let _ = pile.attach(&HolePie::from_token(road), HoleDir::Pull);
            }
            lanes.push(Lane {
                name: row.name,
                road,
            });
        }
        Ok(System {
            control: Control::new(boot.catalog, boot.machine, boot.pier),
            lanes,
            pile,
            tree: None,
            otip: None,
            btip: None,
            face: None,
            coord: operator_bridge::Coord::default(),
        })
    }

    /// 这台生命周期的机器（[`Control`]）。
    pub fn control_mut(&mut self) -> &mut Control {
        &mut self.control
    }

    /// 立账：**先立整张账，再逐条起**（名字 + 怎么算起来）。
    pub fn enlist(&mut self, program: &Program) -> Result<(), Error> {
        self.control.enlist(program.name, program.setup)
    }

    /// **起一条**：建域产线程 → 装通道 → 身份（放行前）→ 放行等就绪 → 递配给 → 板 → 树 →
    /// 认下"起过的东西"。
    ///
    /// **次序即契约**（与 iii 之前逐字相同）：先起的先就绪，后面的就能向它要东西；持树者必须
    /// 先于客人（客人上树要它在），名册必须先于其余（其后的身份都从它来）。
    ///
    /// 失败一律折成 **`plan.died`**（按服务分的号住装配单），读数靠那两行 debug
    /// （先印程序名、再印哪一步）。
    pub fn bring_up(&mut self, program: &Program, plan: &Plan) -> Result<Service, Died> {
        let name = Name::new(program.name).map_err(|_| fail(program, plan, Error::Manifest))?;
        let mut service = self
            .control
            .spawn(program.name)
            .map_err(|e| fail(program, plan, e))?;

        // 通信：放行前把 `setup` 里那几条通道装上（记号 = 通道名，放行后逐条认领）。
        for s in program.setup {
            if let Setup::Channel(ch) = s {
                control::connect(&mut service.1, ch).map_err(|e| fail(program, plan, e))?;
            }
        }

        // 身份：**放行之前**就做完——故服务一起来 `resolve(self)` 就答得出。（名册本身与树
        // 不走这里：它们起来时它还没在；那两条由下面的"认下起过的东西"在它放行之后补绑。）
        if plan.bind {
            if let Some(face) = self.face.as_ref() {
                let mine = face
                    .derive(PrincipalId::ROOT, Wait::AtMost(READY_MS))
                    .map_err(|_| fail(program, plan, Error::Step("derive")))?;
                face.bind(service.0, mine, Wait::AtMost(READY_MS))
                    .map_err(|_| fail(program, plan, Error::Step("bind")))?;
            }
        }

        // 放行 + 等就绪（有通道的那一条顺带逐条认领）；再递门闩单。
        self.control
            .start(name, &mut service, program.setup)
            .map_err(|e| fail(program, plan, e))?;
        self.control
            .wire(name, &service, program.setup)
            .map_err(|e| fail(program, plan, e))?;

        // 板：本域是板的宿主 ⇒ 把客人交出来的那一枚转授过去。**在 records 之后**：板那条路
        // 由客人在起来之后自己装（它是问的那一侧），而它要先收到配给才轮得到板那一问。
        if plan.board {
            let me = utask::self_id();
            let lane = self
                .lanes
                .iter()
                .find(|l| l.name == program.name)
                .and_then(|l| l.road);
            board_bridge::attach(
                &mut service.1,
                me,
                service.0,
                name,
                Wait::AtMost(READY_MS),
                &mut self.btip,
                lane,
            )
            .map_err(|why| fail(program, plan, Error::Step(why)))?;
        }

        // 树：**按需**把这条服务接到持树者那棵树上。**在板之后**：两者各一条路、互不影响。
        if plan.operator {
            // 持树者必须先于这位客人起：提示之路还没认下就没得接。
            let host = self
                .tree
                .ok_or_else(|| fail(program, plan, Error::Step("no tree yet")))?;
            operator_bridge::attach(
                &mut service.1,
                service.0,
                host,
                Wait::AtMost(READY_MS),
                &mut self.otip,
                self.coord,
            )
            .map_err(|why| fail(program, plan, Error::Step(why)))?;
        }

        // 它刚把提示之路交给**生我者**（= 本域）⇒ 当场认下来，此后客人上树才有路可走。
        if plan.holds_tree {
            self.tree = Some(service.0);
            self.otip = None;
            operator_bridge::host_of(service.0, Wait::AtMost(READY_MS), &mut self.otip)
                .map_err(|why| fail(program, plan, Error::Step(why)))?;
        }

        // **哪一双眼睛**：装配单上那一格说了算（不是拿名字认的——`p.name == "principal"`
        // 那种写法，改个名字就静默失灵）。
        if let Some(eyes) = plan.eyes {
            match eyes {
                Eyes::Roster => {
                    // **协调那一帧**要带的第一格：名册自己。它那一枚门牌**由它自己**在
                    // `serve_tree` 之后直接交给持树者，装配者只剩递一格号（`operator/bridge.rs`
                    // 的 `COORD` 照实记）。
                    self.coord.roster = Some(service.0);
                    let f = face_of(service.0)
                        .ok_or_else(|| fail(program, plan, Error::Step("no identity face")))?;
                    let mine = f
                        .derive(PrincipalId::ROOT, Wait::AtMost(READY_MS))
                        .map_err(|_| fail(program, plan, Error::Step("derive self")))?;
                    f.bind(service.0, mine, Wait::AtMost(READY_MS))
                        .map_err(|_| fail(program, plan, Error::Step("bind self")))?;
                    // **树那条身份**：树起来的时候名册还没在，故这里补绑。
                    match self.tree {
                        Some(t) => {
                            let pt = f
                                .derive(PrincipalId::ROOT, Wait::AtMost(READY_MS))
                                .map_err(|_| fail(program, plan, Error::Step("derive tree")))?;
                            f.bind(t, pt, Wait::AtMost(READY_MS))
                                .map_err(|_| fail(program, plan, Error::Step("bind tree")))?;
                        }
                        None => {
                            // 走到这一行时树必已就位（持树者排第一）；真到了那一天，这一句
                            // 是**唯一的响声**（原来它静默跳过）。
                            debug!("principal: no tree to bind");
                        }
                    }
                    self.face = Some(f);
                }
                // 盟册：把它的号占进**协调那一帧**的第二格。它的门牌**也是它自己交的**
                // （同名册那一格）——装配者这一侧只递号、不转授。
                Eyes::League => self.coord.league = Some(service.0),
            }
        }

        Ok(service)
    }

    /// 交棒给监督相（道表与那只组都在本域手里）。
    pub fn supervise(&mut self, last: Name) {
        supervise::run(&mut self.control, &self.lanes, &self.pile, last);
    }
}

/// 报"哪一条、哪一步没成"，返**按服务分的号**（装配单上那一格）。
///
/// 只在失败路径上调：**成功不说话**（装配正常的机器不该刷屏），而失败时这两行决定
/// 还得读几遍代码——所以它报"服务名"与"步骤"两格。
fn fail(program: &Program, plan: &Plan, e: Error) -> Died {
    debug!("{}", program.name);
    debug!("{}", e.said());
    plan.died
}

/// 认下名册**交给生我者**的那一枚门牌（装配者自己的那一份）。
///
/// 装配期**不必上树查自己起的那一枚**：名册起手就把门牌那一枚 `ship` 进本域表里，本域按
/// `(开者 = 它, 记号 = entry)` 两格认出来（[`scall::find`] 的两格正判据）。它起手就交，
/// 故这里是**短等**：还没到就隔一拍再问，问到期限为止。
fn face_of(host: TaskId) -> Option<Face> {
    let mut left = READY_MS;
    loop {
        if let Some(entry) = scall::find(host, bcall::ENTRY_MARK) {
            return Face::of(entry).ok();
        }
        if left == 0 {
            return None;
        }
        let _ = room::sleep(Duration::from_millis(RETRY_MS as u64));
        left = left.saturating_sub(RETRY_MS);
    }
}
