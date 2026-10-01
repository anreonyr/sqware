#![no_std]
#![no_main]

//! root — **引导与固件**：读 boot 的两块账、把一个域起起来，之后只剩一件事——**照单发货**。
//! ```text
//! 1  启动参数 → 清单（有哪些程序）与配对块（有哪些门闩）——两块都是 boot 只读借映的
//! 2  起一条：编排者（`system`，一条 `boot` 通道）——**裸三手**（建域 / 产线程 / 放行），不持表
//! 3  发货循环：收一张单子 → 按坐标取原件、授出 → 回一张回单（[`protocol::system::supply::server::serve`]）
//! 4  那枚孔**读不出** = 编排者没了 ⇒ 本域退出 ⇒ 级联扑杀 ⇒ 自然停机（srst）

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
    // 那一枚——它此刻一步都还没跑，真正的认领由下面那一手做。
    // **持有者活到本函数结束**：这一对孔在 `channels` 里（`Endpoint` 落出作用域才放下本端那一枚）。
    let mut channels = [
        establish::endpoint(orch, Mark::of(supply::BOOT), Wait::POLL)
            .map_err(|_| Die::Orch(E_ORCH))?,
    ];
    utask::hatch(orch).map_err(|_| Die::Orch(E_ORCH))?;
    // **等它就绪 = 认下它交回的那一枚孔**（它那一条通道的凭据）。认不到 ⇒ 这条服务没起来
    // ——本域不另做收尾：域是它生的，本域退出即级联扑杀。
    if !channels[0].claim(orch, Mark::of(supply::BOOT), Wait::AtMost(READY_MS)) {
        return Err(Die::Orch(E_ORCH));
    }

    // 4. 之后只剩发货。**探出编排者没了** ⇒ 退出 ⇒ 级联 ⇒ 停机（见 `protocol::system::supply::server::serve` 的
    //    `alive`：本域读的那枚孔命随本端，故收场靠探活，不靠"读不出"）。
    // 收帧那一只由本域给（**发**那一侧的缓冲在 `Sender::send` 的栈帧上，见 `serve`）。
    let mut ask = [0u8; supply::ORDER_CAP];
    // 取源只有一个：boot 的配对块。持树者那条提示之路不再经过这里（见文件头）。
    let source = |key: env::Key| boot.token(key);
    let alive = || !utask::join(orch, Wait::POLL).unwrap_or(true);
    programs::root::supply::server::serve(&channels[0], source, alive, &mut ask);
    Ok(programs::Report::note(env::EXIT_OK, "root: done"))
}
