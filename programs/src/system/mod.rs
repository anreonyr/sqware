//! system — **编排域的实现侧**：运行时装配上下文（[`Assembly`]）＋ 它容纳的那几件。
//!
//! ```text
//!   Assembly
//!   ├── control   Service 的创建 / 配置 / 启动 / 停止（不认协议）
//!   ├── board     板那一侧：提示之路
//!   ├── tree      持树者那一侧：号 / 提示之路 / 协调帧两格
//!   ├── roster    名册那一侧：身份面
//!   └── watch     监督那一相：死亡道表 / 那只组
//! ```
//!
//! **`Program` = 声明，`Assembly` = 运行时上下文**：一台程序怎么起（谁上板 / 谁上树 / 谁是
//! 持树者 / 它是哪一双眼睛 / 装配期给不给身份）写在它自己那份 `program.rs` 里，而**装配动作**
//! 是 [`Program::assemble`]——它按自己的声明把那一台装进 [`Assembly`]。
//!
//! **本文件里没有"按位分派"**：不再有一处 `if program.operator { … }` 的大 match 替所有程序
//! 解释它们的字段；属于哪一台的语义就在那一台的 `assemble` 一趟里。
//!
//! - [`assemble`]：这一景起哪些台（**过滤 + 按 `order` 排**，就这一件事）
//! - [`bootstrap`]：启动资源获取（与引导域会话 / 机器自述 / 载荷区清单）
//! - [`control`]：Service 的生命周期（内核那几手住 `control::service`，监督相住 `control::supervise`）
//! - [`board`] / [`operator`] / [`principal`] / [`coalition`]：四枚服务的实现
//! - [`machine`]：本域手里那台机器的自述（设备树）

use env::wire::Eyes;
use env::{Name, Wait};
use protocol::debug;
use runtime::env::unit as utask;

use crate::program::{Died, Program, Setup};
use crate::system::board::bridge::Bridge;
use crate::system::bootstrap::Boot;
use crate::system::control::supervise::Watch;
use crate::system::control::{Control, Error, READY_MS, connect};
use crate::system::operator::bridge::Tree;
use crate::system::principal::bridge::Roster;

pub mod assemble;
pub mod board;
pub mod bootstrap;
pub mod coalition;
pub mod control;
pub mod machine;
pub mod operator;
pub mod principal;

/// **运行时装配上下文**：这台机器**已经装配到了什么**——生命周期的机器、板 / 树 / 名册三个
/// 协议手柄，加监督那一相的道表与那只组。
///
/// 它不是"配置表"：配置在 `Program` 上；这里只有**跑起来的东西**（已起的域与线程、已铸的孔、
/// 已成的关系）。
pub struct Assembly {
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

impl Assembly {
    /// 就位：铸道 + 立组（在 [`Watch::of`]）→ 建 Control；三个协议手柄是空的。
    ///
    /// `programs` = 这一景真有的那几台（[`assemble::programs`]）。死亡道跟着它铸：**上板的那
    /// 几位一位一条**——故道表在装配之前就位。
    ///
    /// 失败（那只组立不起来 / 备不下道表）由调用方折成 `system: no group`。
    pub fn new(boot: Boot, programs: &[&'static Program]) -> Result<Assembly, ()> {
        Ok(Assembly {
            control: Control::new(boot.catalog, boot.machine, boot.pier),
            board: Bridge::default(),
            tree: Tree::default(),
            roster: Roster::default(),
            watch: Watch::of(programs)?,
        })
    }

    /// 交棒给监督相（道表与那只组都在 [`Watch`] 手里）。
    pub fn supervise(&mut self, last: Name) {
        self.watch.run(&mut self.control, last);
    }
}

impl Program {
    /// **起一条**——这一台自己的装配，按它自己的声明走：
    ///
    /// 立账 → 建域产线程 → 装通道（放行前）→ 绑身份（放行前）→ 放行等就绪 → 递配给 → 板 →
    /// 树 → 认下那两双眼睛。
    ///
    /// **次序即契约**：先起的先就绪，后面的就能向它要东西；持树者必须先于客人（客人上树要它
    /// 在），名册必须先于其余（其后的身份都从它来）。
    ///
    /// 失败一律折成 [`Program::died`]，读数靠那两行 debug（先印程序名、再印哪一步）。
    pub fn assemble(&self, assembly: &mut Assembly) -> Result<(), Died> {
        // 登记：**"怎么算它起来了"由这一台的 `setup` 推出**（有通道 ⇒ `Announce::Channel`）。
        // 席满 / 名字非法 ⇒ 装配表那一格（与旧 `enlist` 那一趟同号，与具体哪一台无关）。
        assembly
            .control
            .enlist(self.name, self.setup)
            .map_err(|_| crate::system::control::E_TABLE)?;

        let name = Name::new(self.name).map_err(|_| fail(self, Error::Manifest))?;
        let mut service = assembly.control.spawn(self.name).map_err(|e| fail(self, e))?;

        // 通信：放行前把 `setup` 里那几条通道逐条装上（记号 = 通道名，放行后逐条认领）。
        // 一件一件来：`connect` 返的是**那条通道的持有者**（一次一手、一手一对孔）。
        for s in self.setup {
            if let Setup::Channel(ch) = s {
                service
                    .1
                    .try_reserve(1)
                    .map_err(|_| fail(self, Error::Step("no room for channels")))?;
                let channel = connect(service.0, ch).map_err(|e| fail(self, e))?;
                service.1.push(channel);
            }
        }

        // 身份：**放行之前**就做完——故服务一起来 `resolve(self)` 就答得出。（名册本身与树不
        // 走这里：它们起来时名册还没在；那两条由下面的 `roster.adopt` 在它放行之后补绑。）
        assembly
            .roster
            .bind(service.0, self.bind)
            .map_err(|why| fail(self, Error::Step(why)))?;

        // 放行 + 等就绪（有通道的那一条顺带逐条认领）；再递门闩单。
        assembly
            .control
            .start(name, &mut service, self.setup)
            .map_err(|e| fail(self, e))?;
        assembly
            .control
            .wire(name, &service, self.setup)
            .map_err(|e| fail(self, e))?;

        // 板：本域是板的宿主 ⇒ 把客人交出来的那一枚转授过去。**在 records 之后**：板那条路
        // 由客人在起来之后自己装（它是问的那一侧），而它要先收到配给才轮得到板那一问。
        if self.board {
            let lane = assembly.watch.lane_of(self.name);
            assembly
                .board
                .attach(
                    utask::self_id(),
                    service.0,
                    name,
                    Wait::AtMost(READY_MS),
                    lane,
                )
                .map_err(|why| fail(self, Error::Step(why)))?;
        }

        // 树：**按需**把这条服务接到持树者那棵树上。**在板之后**：两者各一条路、互不影响。
        if self.operator {
            // 持树者必须先于这位客人起：提示之路还没认下就没得接。
            assembly
                .tree
                .attach(service.0, Wait::AtMost(READY_MS))
                .map_err(|why| fail(self, Error::Step(why)))?;
        }

        // 它刚把提示之路交给**生我者**（= 本域）⇒ 当场认下来，此后客人上树才有路可走。
        if self.holds_tree {
            assembly
                .tree
                .adopt(service.0, Wait::AtMost(READY_MS))
                .map_err(|why| fail(self, Error::Step(why)))?;
        }

        // **哪一双眼睛**：声明上那一格说了算（不是拿名字认的——`p.name == "principal"`
        // 那种写法，改个名字就静默失灵）。名册那一位要认下面 + 补绑自己与树；盟册只报号。
        if let Some(eyes) = self.eyes {
            let who = match eyes {
                Eyes::Roster => assembly
                    .roster
                    .adopt(service.0, assembly.tree.host())
                    .map_err(|why| fail(self, Error::Step(why)))?,
                Eyes::League => service.0,
            };
            assembly.tree.eye(eyes, who);
        }

        Ok(())
    }
}

/// 报"哪一条、哪一步没成"，返**这一台自己的号**（[`Program::died`]）。
///
/// 只在失败路径上调：**成功不说话**（装配正常的机器不该刷屏），而失败时这两行决定还得读几遍
/// 代码——所以它报"程序名"与"步骤"两格。
fn fail(program: &Program, e: Error) -> Died {
    debug!("{}", program.name);
    debug!("{}", e.said());
    program.died
}
