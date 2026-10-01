#![no_std]
#![no_main]

//! root — **引导与固件**：读 boot 的两块账、把一个域起起来，之后只剩一件事——**照单发货**。
//!
//! ```text
//! 1  启动参数 → 清单（有哪些程序）与配对块（有哪些门闩）——两块都是 boot 只读借映的
//! 2  起一条：编排者（`system`，一条 `boot` 通道）——**裸三手**（建域 / 产线程 / 放行），不持表
//! 3  发货循环：收一张单子 → 按坐标取原件、授出 → 回一张回单（[`protocol::system::supply::server::serve`]）
//! 4  那枚孔**读不出** = 编排者没了 ⇒ 本域退出 ⇒ 级联扑杀 ⇒ 自然停机（srst）
//!    （**补记**：编排者退场之前，它自己那一趟已经把账上收不掉的收过一遍——那一趟的判据是
//!    账，见 `programs/src/system/control/supervise.rs`；级联只剩"收那些收不掉的"。）
//! ```
//!
//! # 照实记（本域的表与清单都退场了：两套装表合一）
//!
//! 从前本域也持一本 `Table`（`register` ＋ `mint` ＋ `start`）与一块 `Catalog`——与编排域那本
//! 是**两本同形的账**，而编排域起来之后本域再不碰它。按用户裁定「两套装表合一」：**表与生命
//! 周期从此只有编排域一个持有者**（`system::Assembly`），本域只剩三件事——读固件那两块账、
//! 起第一个域、照单发货。于是这台机器上"域"这一层只有一处记账。
//!
//! # 照实记（本域为什么走裸三手）
//!
//! 编排域那一套 `control::service::{mint, start}` 的每一步都要账（`admit_start` / `attach` /
//! `set_state`），而账已经归编排域。故第一个域由本域**徒手**起：建域（`build`）→ 产线程
//! （`spawn`）→ 装那条 `boot` 通道 → 放行（`hatch`）→ 等它认领那一枚。这是全树**唯一的例外**，
//! 也正是"**装配者自己不是被装配出来的**"那句话的写法：本域没有装配者，它只有 boot。
//!
//! **要哪一段字节**：按名字在 boot 的清单里取（`Root::programs`）——本域**不建 `Catalog`**
//! （那一层是编排域为"这一景有哪些台"与"来源那一格"备的），特权级也照清单那一条原样转交
//! （唯一声明处是装配表的 `kind`，打包时写进去）。
//!
//! **持树者不在这里**：它是**编排域的服务**（编排域那张单的第一条）。本域不当它的装配者、
//! 也不替它把提示之路转来转去——那是"它是引导设施"时代的形状，那一笔已经清掉。
//!
//! # 本域是固件那一层，不是编排者
//!
//! 它与编排者的关系**像 SBI 与 S 态内核**：常驻、面窄、只答"能不能"。
//!
//! | 本域做 | 本域不做 |
//! |---|---|
//! | 读 boot 的两块账（**只有它读得到**） | 不认识服务名，不排顺序，不记账 |
//! | 按坐标发货（原件留在它手里） | 不接死亡道、不看活、不判就绪 |
//! | 起**编排者**一条（其余服务由编排者起） | 不起第二个域、不认第二条路 |
//! | 退出即停机（唯一能结束机器的那一枚） | 不退场、不重启、不做策略 |
//!
//! 于是"发货"这条路是**单向的机制面**：编排者说"要哪几样、给谁、多大权"，本域照办，
//! 而**形态照请求**（`VEST` / `ONLY` 都是请求方说的——为什么这一格从前要剔掉、这一刀为什么
//! 放开，见 [`pairing::supply`] 的照实记）。
//!
//! # 本域是设备门闩的**第一个持有者**
//!
//! boot 把设备树扫成门闩、连**坐标**一起写进配对块，整块借映进本域（`platform/devices.rs`）。
//! 本域**不解释设备语义**——它只按坐标挑出要交出去的那几枚（坐标是区、或是"哪一件"）。
//! **要什么由要的人开单子**，而那张单子随请求过线，本域不手抄坐标，也不猜权与形态。
//!
//! **退出即关机**，故门的两条硬判据（自行退出 + 无 panic）照旧成立，不需要外接 timeout。
//! 常驻服务（没有"干完"这回事的那种）由本域退场时的级联收掉。

extern crate alloc;
extern crate programs;


// 两块账在引导域自己那一摊里（只有它读得到）。
use env::{Mark, Wait};
use programs::root::boot;

use protocol::communication::establish;
// 生命那一族共用的两个数（号与就绪上限）——本域只借它们，不借那一族的手（见文件头）。
use programs::system::control::{Died, E_MANIFEST, READY_MS};

use protocol::system::supply;
use runtime::env::unit as utask;

/// 编排者那一条在清单里的名字。
const ORCH: &str = "system";

/// 本域的死法：**一格 = 死在起手的哪一步**——号与从前的 `service::die` **同值**
/// （`1` 引导那一族、`2/3/4` 归 [`control`](programs::system::control) 那三格、`6` 是"起编排者"那一族）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Die {
    /// 启动参数那两块账读不出来。
    BootArgs,
    /// 清单 / 服务名 / 泊位名读不出来。
    Manifest,
    /// 起编排者那一族的号（`E_ORCH` = 6，或 `mint` 带来的格子编号）。
    Orch(env::Reason),
}

impl Die {
    fn code(self) -> env::Reason {
        match self {
            Die::BootArgs => E_BOOT,
            Die::Manifest => E_MANIFEST,
            Die::Orch(code) => code,
        }
    }

    const fn text(self) -> &'static str {
        match self {
            Die::BootArgs => "root: boot args unreadable",
            Die::Manifest => "root: manifest bad",
            Die::Orch(_) => "root: orchestrator",
        }
    }
}

impl programs::Exit for Die {
    fn report(&self) -> programs::Report<'_> {
        programs::Report::note(self.code(), self.text())
    }
}

/// 起编排者那一族共用的号（`mint` 的失败格与后面三步都用它）。
const E_ORCH: Died = 6;

/// 引导那一族共用的号（"启动参数读不出来"那一格）。
const E_BOOT: Died = 1;

#[programs::entry]
fn main() -> Result<programs::Report<'static>, Die> {
    // 1. 启动参数 → 两块账（清单 + 配对块）。读不出就没得装配。
    let Some(boot) = boot::Root::take() else {
        return Err(Die::BootArgs);
    };
    // 配对块的自述（一行）：按坐标分账——`region` 是区段的条数（设备 + 载荷区），
    // `dtb` / `irq` 各一件，`bad` 是读不懂的条数。**照实记**：从前这一行报的是"重名"
    // （同一节点的多段 `reg` 造出两条同名记录，而按名取只够得到第一枚）——坐标换成区之后
    // 那笔账不存在了（两段各有各的基址）。
    boot.report_pairs();


    // 2. 从清单里挑出编排者那一条：**它那一段字节 ＋ 它那个特权级**（后者是打包时按装配表的
    //    `kind` 写进清单的，本域只原样转交）。
    let mut list = boot.programs();
    let entry = loop {
        let Some(entry) = list.next() else {
            return Err(Die::Manifest);
        };
        let entry = entry.map_err(|_| Die::Manifest)?;
        if entry.name == ORCH {
            break entry;
        }
    };

    // 3. **裸三手**起它（没有账）：建域 → 产线程 → 装那条 `boot` 通道 → 放行。
    let team = utask::build(entry.elf, entry.kind).map_err(|_| Die::Orch(E_ORCH))?;
    let orch = utask::spawn(team, 0, &[], 0).map_err(|_| Die::Orch(E_ORCH))?;
    // 这条通道**两头都装**（一手就是 `establish::endpoint`：铸本端那一枚交给它、并试认它那一枚）：
    // 本域**读**自己那一枚（单子从这来），**写**对端那一枚（回单往这去）。`POLL` = 这一趟不等它
    // 那一枚——它此刻一步都还没跑，真正的认领由下面那一手做（判据同一个记号）。
    //
    // **持有者活到本函数结束**：这一对孔在 `channels` 里（`Endpoint` 落出作用域才放下本端那一枚）。
    let mut channels = [
        establish::endpoint(orch, Mark::of(supply::BOOT), Wait::POLL)
            .map_err(|_| Die::Orch(E_ORCH))?,
    ];
    // 放行：**这一刀之后它就跑了**。
    utask::hatch(orch).map_err(|_| Die::Orch(E_ORCH))?;
    // **等它就绪 = 认下它交回的那一枚孔**（它那一条通道的凭据）。认不到 ⇒ 这条服务没起来
    // ——本域不另做收尾：域是它生的，本域退出即级联扑杀（原 `service::ready` 那一格同一判据）。
    if !channels[0].claim(orch, Mark::of(supply::BOOT), Wait::AtMost(READY_MS)) {
        return Err(Die::Orch(E_ORCH));
    }

    // 4. 之后只剩发货。**探出编排者没了** ⇒ 退出 ⇒ 级联 ⇒ 停机（见 `protocol::system::supply::server::serve` 的
    //    `alive`：本域读的那枚孔命随本端，故收场靠探活，不靠"读不出"）。
    // 收帧那一只由本域给（**发**那一侧的缓冲在 `Sender::send` 的栈帧上，见 `serve`）。
    let mut ask = [0u8; supply::ORDER_CAP];
    // 取源只有一个：boot 的配对块。持树者那条提示之路不再经过这里（见文件头）。
    let source = |key: env::Key| boot.token(key);
    // **"它还活着吗"这一问换了来路**（照实记）：从前它读表（`until(&table, …)` 的非阻塞那一问），
    // 而表已经归编排域；本域手里只剩那一枚线程的号，故直接问内核：`join` 的非阻塞那一问，
    // `true` = 已回收 ⇒ 没了。判决仍然只有一处（`join`），不另立一条判据。
    let alive = || !utask::join(orch, Wait::POLL).unwrap_or(true);
    programs::root::supply::server::serve(&channels[0], source, alive, &mut ask);
    Ok(programs::Report::note(env::EXIT_OK, "root: done"))
}
