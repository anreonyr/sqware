//! system — **编排域的实现侧**：一个容器（[`System`]）＋ 它容纳的那几件。
//!
//! ```text
//!   System
//!   ├── control   Service 的创建 / 配置 / 启动 / 停止（不认协议）
//!   ├── board     板那一侧：提示之路
//!   ├── tree      持树者那一侧：号 / 提示之路 / 协调帧两格
//!   ├── roster    名册那一侧：身份面
//!   └── watch     监督那一相：死亡道表 / 那只组
//! ```
//!
//! **每一间管自己的状态与自己的步骤**：装配者这一侧只剩五个手柄，没有裸的
//! `tree` / `otip` / `btip` / `face` / `coord` 散在容器上；接树那三步、认门牌那一手、
//! 铸道那一圈也各回各的文件（`operator/bridge.rs` / `principal/bridge.rs` /
//! `control/supervise.rs`）。
//!
//! **边仍是装配单上那几格**（[`plan::assembly::Plan`]）：谁上板 / 谁上树 / 谁是持树者 /
//! 它是哪一双眼睛 / 装配期给不给身份——[`System::bring_up`] 按序把它们落到各自那一手上。
//!
//! **三枚服务（持树者 / 名册 / 盟册）是普通程序**：各自一个 bin、一个域，由本域按装配单
//! 用与其他每一台相同的 `mint` 起起来。
//!
//! - [`assemble`]：这一景起哪些台、每台要什么——**scenario**（不再有第二张关系表）
//! - [`bootstrap`]：启动资源获取（与引导域会话 / 机器自述 / 载荷区清单）
//! - [`control`]：Service 的生命周期（内核那几手住 `control::service`，监督相住 `control::supervise`）
//! - [`program`]：静态节点的词汇（`Program` / `Setup`）
//! - [`board`] / [`operator`] / [`principal`] / [`coalition`]：四枚服务的实现
//! - [`machine`]：本域手里那台机器的自述（设备树）

use env::{Name, Wait};
use plan::assembly::{Eyes, Plan, Row};
use protocol::debug;
use runtime::env::unit as utask;

use crate::system::board::bridge::Bridge;
use crate::system::bootstrap::Boot;
use crate::system::control::supervise::Watch;
use crate::system::control::{Control, Died, Error, READY_MS, Service, connect};
use crate::system::operator::bridge::Tree;
use crate::system::principal::bridge::Roster;
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

/// **编排域自己**：五个手柄，各管自己那一间。
pub struct System {
    /// Service 的生命周期机器。
    control: Control,
    /// 板那一侧（提示之路）。
    board: Bridge,
    /// 持树者那一侧（号 / 提示之路 / 协调帧两格）。
    tree: Tree,
    /// 名册那一侧（身份面）。
    roster: Roster,
    /// 监督那一相（死亡道表 / 那只组）。
    watch: Watch,
}

impl System {
    /// 就位：铸道 + 立组（在 [`Watch::of`]）→ 建 Control；三个协议手柄是空的。
    ///
    /// 失败（那只组立不起来 / 备不下道表）由调用方折成 `system: no group`。
    pub fn new(boot: Boot, rows: &[&'static Row]) -> Result<System, ()> {
        Ok(System {
            control: Control::new(boot.catalog, boot.machine, boot.pier),
            board: Bridge::default(),
            tree: Tree::default(),
            roster: Roster::default(),
            watch: Watch::of(rows)?,
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
    /// 认下那两双眼睛。
    ///
    /// **次序即契约**：先起的先就绪，后面的就能向它要东西；持树者必须先于客人（客人上树要
    /// 它在），名册必须先于其余（其后的身份都从它来）。
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
                connect(&mut service.1, ch).map_err(|e| fail(program, plan, e))?;
            }
        }

        // 身份：**放行之前**就做完——故服务一起来 `resolve(self)` 就答得出。（名册本身与树
        // 不走这里：它们起来时名册还没在；那两条由下面的 `roster.adopt` 在它放行之后补绑。）
        self.roster
            .bind(service.0, plan.bind)
            .map_err(|why| fail(program, plan, Error::Step(why)))?;

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
            let lane = self.watch.lane_of(program.name);
            self.board
                .attach(
                    &mut service.1,
                    utask::self_id(),
                    service.0,
                    name,
                    Wait::AtMost(READY_MS),
                    lane,
                )
                .map_err(|why| fail(program, plan, Error::Step(why)))?;
        }

        // 树：**按需**把这条服务接到持树者那棵树上。**在板之后**：两者各一条路、互不影响。
        if plan.operator {
            // 持树者必须先于这位客人起：提示之路还没认下就没得接。
            self.tree
                .attach(&mut service.1, service.0, Wait::AtMost(READY_MS))
                .map_err(|why| fail(program, plan, Error::Step(why)))?;
        }

        // 它刚把提示之路交给**生我者**（= 本域）⇒ 当场认下来，此后客人上树才有路可走。
        if plan.holds_tree {
            self.tree
                .adopt(service.0, Wait::AtMost(READY_MS))
                .map_err(|why| fail(program, plan, Error::Step(why)))?;
        }

        // **哪一双眼睛**：装配单上那一格说了算（不是拿名字认的——`p.name == "principal"`
        // 那种写法，改个名字就静默失灵）。名册那一位要认下面 + 补绑自己与树；盟册只报号。
        if let Some(eyes) = plan.eyes {
            let who = match eyes {
                Eyes::Roster => self
                    .roster
                    .adopt(service.0, self.tree.host())
                    .map_err(|why| fail(program, plan, Error::Step(why)))?,
                Eyes::League => service.0,
            };
            self.tree.eye(eyes, who);
        }

        Ok(service)
    }

    /// 交棒给监督相（道表与那只组都在 [`Watch`] 手里）。
    pub fn supervise(&mut self, last: Name) {
        self.watch.run(&mut self.control, last);
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
