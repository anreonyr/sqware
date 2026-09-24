#![no_std]
#![no_main]

//! root — **引导与固件**：读 boot 的两块账、把一个域起起来，之后只剩一件事——**照单发货**。
//!
//! ```text
//! 1  启动参数 → 清单（有哪些程序）与配对块（有哪些门闩）——两块都是 boot 只读借映的
//! 2  起一条：编排者（`system`，一条 `boot` 通道）
//! 3  发货循环：收一张单子 → 按坐标取原件、授出 → 回一张回单（[`protocol::driver::supply::server::serve`]）
//! 4  那枚孔**读不出** = 编排者没了 ⇒ 本域退出 ⇒ 级联扑杀 ⇒ 自然停机（srst）
//! ```
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
//! | 按坐标发货（原件与 `VEST` **都留在它手里**） | 不接死亡道、不看活、不判就绪 |
//! | 起**编排者**一条（其余服务由编排者起） | 不起第二个域、不认第二条路 |
//! | 退出即停机（唯一能结束机器的那一枚） | 不退场、不重启、不做策略 |
//!
//! 于是"发货"这条路是**单向的机制面**：编排者说"要哪几样、给谁、多大权"，本域照办，
//! 而**形态照请求、唯独 `VEST` 一律剔掉**（固件不发"再授出的权"，见 [`pairing::supply`]）。
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

// 两块账在引导域自己那一摊里（只有它读得到）；装配机器是两个装配者共用的一台。
use env::Mark;
use programs::supervisor::root::boot;
use programs::supervisor::service;

use env::Name;
use protocol::session::Quay;
// 协议侧那三档（判定 / 账 / 适配）与本地的 `service`（装配机器）**同名不同物**，故逐个取名进来。
use programs::supervisor::system::server::{self as core, until};
use protocol::system::core::Reaped;
use protocol::system::desk::{Announce, Table};

use protocol::driver::supply;
use service::Catalog;

/// 编排者那一条在清单里的名字。
const ORCH: &str = "system";

/// 本域的死法：**一格 = 死在起手的哪一步**——号与从前的 `service::die` **同值**
/// （`1` 引导那一族、`2/3/4` 归 [`service`] 那三格、`6` 是"起编排者"那一族）。
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
            Die::Manifest => service::E_MANIFEST,
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
const E_ORCH: service::Died = 6;

/// 引导那一族共用的号（"启动参数读不出来"那一格）。
const E_BOOT: service::Died = 1;

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
    let Some(catalog) = Catalog::of_boot(&boot) else {
        return Err(Die::Manifest);
    };
    let Some(orch_name) = Name::new(ORCH).ok() else {
        return Err(Die::Manifest);
    };
    let Some(slot) = Name::new(supply::BOOT).ok() else {
        return Err(Die::Manifest);
    };

    let mut table = Table::new();

    // 2. 起编排者：它有一条 `boot` 通道——配给从那里问、回单从那里回。
    let orch = mint(&mut table, &catalog, ORCH, orch_name, Announce::Channel)?;
    // 这条泊位两头都装：本域**读**自己那一枚（单子从这来），**写**对端那一枚（回单往这去）。
    let mut quay = Quay::open(orch);
    if quay.seat(slot).is_err() {
        return Err(Die::Orch(E_ORCH));
    }
    if core::start(
        &mut table,
        orch_name,
        orch,
        &[],
        Some(&mut quay),
        &[Mark::of(slot.as_str())],
        service::READY_MS,
    )
    .is_err()
    {
        return Err(Die::Orch(E_ORCH));
    }
    let Some(pier) = quay.find(slot) else {
        return Err(Die::Orch(E_ORCH));
    };

    // 3. 之后只剩发货。**探出编排者没了** ⇒ 退出 ⇒ 级联 ⇒ 停机（见 `protocol::driver::supply::server::serve` 的
    //    `alive`：本域读的那枚孔命随本端，故收场靠探活，不靠"读不出"）。
    let mut ask = [0u8; supply::ORDER_CAP];
    let mut out = [0u8; supply::REPLY_CAP];
    // 取源只有一个：boot 的配对块。持树者那条提示之路不再经过这里（见文件头）。
    let source = |key: env::Key| boot.token(key);
    // "它还活着吗"这一问**不另立判据**：用 `until` 的非阻塞那一问（判决只该有一个实现）。
    let alive = || !matches!(until(&table, orch_name, 0), Ok(Reaped::Now));
    programs::supervisor::supply::server::serve(&pier, source, alive, &mut ask, &mut out);
    Ok(programs::Report::note(env::EXIT_OK, "root: done"))
}

/// 登记一行 + 建域 + 产线程 + 挂身子（此刻它一步都还没跑）。
///
/// 失败的**原因码走返回值**（`Err(Die::…)`）：报码那一笔账现在是 `main` 的账，这一层
/// 只负责把"死在第几步"带上去——不再自己退场。**三格的号都一样**（`E_ORCH`）：这一族
/// 读的是"起编排者没走通"，具体哪一格由上面那两句 `debug` 行分辨。
fn mint(
    table: &mut Table,
    catalog: &Catalog<'_>,
    what: &str,
    name: Name,
    announce: Announce,
) -> Result<env::TaskId, Die> {
    if table.register(name, announce).is_err() {
        return Err(Die::Orch(E_ORCH));
    }
    let Some(entry) = catalog.find(what) else {
        return Err(Die::Orch(E_ORCH));
    };
    match core::mint(table, name, entry.elf, entry.kind) {
        Ok(task) => Ok(task),
        Err(_) => Err(Die::Orch(E_ORCH)),
    }
}

