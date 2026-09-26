//! system — **编排域的实现侧**：一个容器（[`System`]）＋ 它容纳的那几件。
//!
//! ```text
//!   System
//!   ├── Control        Service 的创建 / 资源布线 / 生命周期（不认协议）
//!   ├── operator       持树者那一侧（树那条边 + 提示之路）
//!   ├── principal      名册那一侧（身份面 + 派号 / 绑号）
//!   ├── coalition      盟册那一侧（协调帧的第二格）
//!   └── board          板（死亡道那一侧）
//! ```
//!
//! **边住这里**：谁上板 / 谁上树 / 谁是持树者 / 它是哪一双眼睛 / 装配期给不给身份——
//! 这些由 scenario（[`assemble::Edges`]）声明，由 [`System::bring_up`] 按序落到
//! 四套协议各自的那一手上。Control 只看得到 [`Service`]，**一个协议名字都不认**。
//!
//! 三枚内件（持树者 / 名册 / 盟册）与板都住本域；它们的实现在同名的子目录里。
//!
//! - [`assemble`]：这一景有哪些 Program（节点）与关系（边）——**scenario**
//! - [`bootstrap`]：启动资源获取（与引导域会话 / 机器自述 / 载荷区清单）
//! - [`control`]：装配与生命周期（内核适配住 `control::service`，监督相住 `control::supervise`）
//! - [`program`]：静态节点的词汇（`Program` / `Source` / `Setup`）
//! - [`board`] / [`operator`] / [`principal`] / [`coalition`]：四枚本域常驻线程的实现
//! - [`machine`]：本域手里那台机器的自述（设备树）

use core::time::Duration;

use alloc::vec::Vec;

use env::{HoleDir, Mark, Name, PieToken, TaskId, Wait};
use plan::assembly::Eyes;
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

use crate::system::assemble::Node;
use crate::system::board::bridge as board_bridge;
use crate::system::bootstrap::Boot;
use crate::system::control::{Control, Died, Error, Lane, READY_MS, RETRY_MS, Service};
use crate::system::operator::bridge as operator_bridge;

pub mod assemble;
pub mod board;
pub mod bootstrap;
pub mod coalition;
pub mod control;
pub mod machine;
pub mod operator;
pub mod principal;
pub mod program;

/// **编排域自己**：Control ＋ 四套协议的组装态。
///
/// 五格"起过的东西"（`tree` / `otip` / `btip` / `face` / `coord`）**只有一份**：它们是
/// "起过的东西"，不是每一条各一份——与旧 `service::assemble` 里那五个累积变量逐字对应。
pub struct System {
    /// Service 的生命周期机器。
    control: Control,
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
    /// 就位：**铸死亡道**（上板的那几位一位一条，记号 `gone-<名字>`）→ 建 Control。
    ///
    /// 失败（那只组立不起来 / 备不下道表）由调用方折成 `system: no group`。
    pub fn new(boot: Boot, nodes: &[Node]) -> Result<System, ()> {
        // 组是**独占**的（`shared = false`）：本线程用它等任一道响（零轮询）。
        let pile = Pile::unseal(false).map_err(|_| ())?;
        let mut lanes: Vec<Lane> = Vec::new();
        lanes.try_reserve(nodes.len()).map_err(|_| ())?;
        for node in nodes {
            // 记号 = `LANE_PREFIX` ＋ 名字：**前缀只有一处定义**（板那一侧按同一个常量
            // 拼出来找它——见 `lane_for`）。**不上板的那几位不铸道**：没有写端的道永远不会响。
            let road = if node.edges.board {
                mail::unseal_hole(Mark::of(&alloc::format!(
                    "{LANE_PREFIX}{}",
                    node.program.name
                )))
                .ok()
            } else {
                None
            };
            if let Some(road) = road {
                let _ = pile.attach(&HolePie::from_token(road), HoleDir::Pull);
            }
            lanes.push(Lane {
                name: node.program.name,
                road,
            });
        }
        Ok(System {
            control: Control::new(boot.catalog, boot.machine, boot.pier, lanes, pile),
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
    pub fn enlist(&mut self, node: &Node) -> Result<(), Error> {
        self.control.enlist(&node.program)
    }

    /// **起一条**：spawn / 资源与通道登记 → 身份（放行前）→ 放行等就绪 → 递配给 →
    /// 板 → 树 → 认下"起过的东西"。
    ///
    /// **次序即契约**（与旧 `service::start` + `service::assemble` 逐字相同）：先起的先就绪，
    /// 后面的就能向它要东西；持树者必须先于客人（客人上树要它在），名册必须先于其余
    /// （其后的身份都从它来）。
    ///
    /// 失败一律折成 **`node.edges.died`**（按服务分的号住装配单），读数靠那两行 debug
    /// （先印程序名、再印哪一步）。
    pub fn bring_up(&mut self, node: &Node) -> Result<Service, Died> {
        let p = &node.program;
        let mut svc = p.assemble(&mut self.control).map_err(|e| fail(node, e))?;

        // 一之后、二之前：**身份**。装配者给这条服务派生一条号、把它绑到那一枚线程上
        // ——**放行之前**就做完，故服务一起来 `resolve(self)` 就答得出。（名册本身与树不走
        // 这里：它们起来时它还没在；那两条由下面的 `capture` 在它放行之后补绑。）
        if node.edges.bind {
            if let Some(face) = self.face.as_ref() {
                let mine = face
                    .derive(PrincipalId::ROOT, Wait::AtMost(READY_MS))
                    .map_err(|_| fail(node, Error::Step("derive")))?;
                face.bind(svc.task, mine, Wait::AtMost(READY_MS))
                    .map_err(|_| fail(node, Error::Step("bind")))?;
            }
        }

        // 放行 + 等就绪（有通道的那一条顺带逐条认领）；再递门闩单。
        self.control.start(&mut svc).map_err(|e| fail(node, e))?;
        self.control.wire(&svc).map_err(|e| fail(node, e))?;

        // 板：本域是板的宿主 ⇒ 起一枚待客线程，再把客人交出来的那一枚转授给它。
        // **在 records 之后**：板那条路由客人在起来之后自己装（它是问的那一侧），
        // 而它要先收到配给才轮得到板那一问。
        if node.edges.board {
            let me = utask::self_id();
            let name = Name::new(p.name).map_err(|_| fail(node, Error::Manifest))?;
            let lane = self.control.lane_of(p.name);
            let task = svc.task;
            board_bridge::attach(
                svc.quay_mut(),
                me,
                task,
                name,
                Wait::AtMost(READY_MS),
                &mut self.btip,
                lane,
            )
            .map_err(|why| fail(node, Error::Step(why)))?;
        }

        // 树：**按需**把这条服务接到持树者那棵树上。**在板之后**：两者各一条路、互不影响。
        if node.edges.operator {
            // 持树者必须先于这位客人起：提示之路还没认下就没得接。
            let host = self
                .tree
                .ok_or_else(|| fail(node, Error::Step("no tree yet")))?;
            let task = svc.task;
            operator_bridge::attach(
                svc.quay_mut(),
                task,
                host,
                Wait::AtMost(READY_MS),
                &mut self.otip,
                self.coord,
            )
            .map_err(|why| fail(node, Error::Step(why)))?;
        }

        // 它刚把提示之路交给**生我者**（= 本域）⇒ 当场认下来，此后客人上树才有路可走。
        if node.edges.holds_tree {
            self.tree = Some(svc.task);
            self.otip = None;
            operator_bridge::host_of(svc.task, Wait::AtMost(READY_MS), &mut self.otip)
                .map_err(|why| fail(node, Error::Step(why)))?;
        }

        // **哪一双眼睛**：边说了算（不是拿名字认的——`p.name == "principal"` 那种写法，
        // 改个名字就静默失灵）。
        if let Some(eyes) = node.edges.eyes {
            match eyes {
                Eyes::Roster => {
                    // **协调那一帧**要带的第一格：身份服务自己。
                    //
                    // 它那一枚门牌**由它自己**在 `serve_tree` 之后直接交给持树者
                    // （见 `operator/bridge.rs` 的 `COORD` 照实记），装配者只剩递一格号。
                    self.coord.roster = Some(svc.task);
                    let f = face_of(svc.task)
                        .ok_or_else(|| fail(node, Error::Step("no identity face")))?;
                    let mine = f
                        .derive(PrincipalId::ROOT, Wait::AtMost(READY_MS))
                        .map_err(|_| fail(node, Error::Step("derive self")))?;
                    f.bind(svc.task, mine, Wait::AtMost(READY_MS))
                        .map_err(|_| fail(node, Error::Step("bind self")))?;
                    // **树那条身份**：树起来的时候身份服务还没在，故这里补绑。
                    match self.tree {
                        Some(t) => {
                            let pt = f
                                .derive(PrincipalId::ROOT, Wait::AtMost(READY_MS))
                                .map_err(|_| fail(node, Error::Step("derive tree")))?;
                            f.bind(t, pt, Wait::AtMost(READY_MS))
                                .map_err(|_| fail(node, Error::Step("bind tree")))?;
                        }
                        None => {
                            // 走到这一行时树必已就位（内件三枚次序保证）；真到了那一天，
                            // 这一句是**唯一的响声**（原来它静默跳过）。
                            debug!("principal: no tree to bind");
                        }
                    }
                    self.face = Some(f);
                }
                // 盟册：把它的号占进**协调那一帧**的第二格。它的门牌**也是它自己交的**
                // （同 principal 那一格）——装配者这一侧只递号、不转授。
                Eyes::League => self.coord.league = Some(svc.task),
            }
        }

        Ok(svc)
    }
}

/// 报"哪一条、哪一步没成"，返**按服务分的号**（装配单上那一格）。
///
/// 只在失败路径上调：**成功不说话**（装配正常的机器不该刷屏），而失败时这两行决定
/// 还得读几遍代码——所以它报"服务名"与"步骤"两格。
fn fail(node: &Node, e: Error) -> Died {
    debug!("{}", node.program.name);
    debug!("{}", e.said());
    node.edges.died
}

/// 认下身份服务**交给生我者**的那一枚门牌（装配者自己的那一份）。
///
/// 装配期**不必上树查自己起的那一枚**：Server 起手就把门牌那一枚 `ship` 进本域表里，本域按
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
