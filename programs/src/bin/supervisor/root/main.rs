#![no_std]
#![no_main]

//! root — 装配者：**按装配单起服务**，并在最后一条退场后收场。
//!
//! ```text
//! 1  启动参数 → 清单（有哪些程序）与配对块（有哪些门闩）——两块都是 boot 只读借映的
//! 2  按装配单登记整张表（`service::PLAN`）
//! 3  按装配单逐条起：建域 → 产线程 → 定会话 → 装通道 → 放行 → 等就绪 → 发门闩
//! 4  等最后一条退场；顺手把**已经收尾的**子域放下（`Oust`，每个在册服务报一行读数）
//!    ——没有这一步，那些死域的外壳会一直挂在装配者身上，直到本域退场才随级联走
//! 5  本域退出 ⇒ 级联扑杀 ⇒ 全部回收 ⇒ 自然停机（srst）
//! ```
//!
//! **本文件只剩流程**：起谁、按什么顺序、怎么发门闩、怎么算起来了，全在
//! [`service::PLAN`]（一张表）。要加第三个服务 —— 表里加一行，本文件一个字不改。
//!
//! 三件事各有其家，本文件不做：
//!
//! | 事 | 住在哪 |
//! |---|---|
//! | 起一个服务（建域/产线程/塞门闩/放行/等就绪） | `protocol::system` |
//! | 会话怎么建（交孔、认领、凑齐） | `protocol::session` |
//! | boot 的账怎么读、记录怎么编解码 | [`pairing`] |
//!
//! # 本域是设备门闩的**第一个持有者**，也是转授者
//!
//! boot 把设备树扫成门闩、连名字一起写进配对块，整块借映进本域（`platform/devices.rs`）。
//! 本域**不解释设备语义**——它只按名字挑出要交出去的那几枚。**要什么由要的人开单子**，
//! 而那张单子（[`needs`]）是**两端共用的一份**：本域不手抄名字，也不猜权与形态。
//!
//! **退出即关机**，故门的两条硬判据（自行退出 + 无 panic）照旧成立，不需要外接 timeout。
//! 常驻服务（没有"干完"这回事的那种）由本域退场时的级联收掉。

extern crate alloc;
extern crate programs;

// 共享物住在 supervisor 目录里，由两个 bin 各自声明一次（见 `needs.rs` 头注）。
#[path = "../board.rs"]
// 本域只用**板侧**那一半（客侧那三手是给服务用的）⇒ 另一半在这里是死码。
#[allow(dead_code)]
mod board;
#[path = "../needs.rs"]
mod needs;
#[path = "../pairing.rs"]
mod pairing;
#[path = "../service.rs"]
mod service;

use protocol::system::service::{Slot, Table};

use service::{E_BOOT, E_MANIFEST, E_PROGRAM, E_TABLE};

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    // 1. 启动参数 → 两块账（清单 + 配对块）。读不出就没得装配。
    let Some(boot) = pairing::Root::take() else {
        service::die(E_BOOT, "root: boot args unreadable");
    };

    // 2/3. 登记整张表，再按顺序起。
    let mut table = Table::new();
    let last = match service::assemble(&mut table, &boot) {
        Ok(last) => last,
        Err(E_MANIFEST) => service::die(E_MANIFEST, "root: manifest bad"),
        Err(E_TABLE) => service::die(E_TABLE, "root: table full"),
        Err(E_PROGRAM) => service::die(E_PROGRAM, "root: program missing"),
        Err(died) => service::die(died, "root: service failed"),
    };

    // 4. 等最后一条退场：它一走 ⇒ 本域退出 ⇒ 级联 ⇒ 全部回收 ⇒ 停机。
    service::wait_last(&mut table, last);
    // 5. **半步 restart 的读数**：顺手放下**已经收尾**的子域。没有这一步，那些死域的
    //    外壳会一直挂在装配者身上（`heir` 只增不减），直到本域退场才随级联走。
    oust_the_dead(&table);
    // 6. **会话的收尾由会话的主人负责**：常驻线程是它起的，也是它收的。本域里那枚
    //    板线程没有 `Join` 可等（`attach` 里弃权了），故按号点名收掉——同域线程之间
    //    没有寿命耦合，内核的子域级联收不到它。
    board::shut();
    service::die(service::E_OK, "root: done")
}

/// 把**已经收尾**的子域放下（`UnitCall::Oust`）：对每一条在册服务试一次，报一行。
///
/// **不探活**：`join` 答不出"死透了吗"——死透到名册条目都被 `prune_dead` 摘掉之后，
/// `Join` 只答 `Denied`（问不出"没了"与"从来没有过"）。判断在这里是**动作自带的**：
/// 收尾了就摘掉（`ousted=true`），还活着就答 `Busy`（`ousted=false`），两条都报。
///
/// 名字从表里读、不写死：加服务仍然只改 `service::PLAN`。`heir` 的计数变化是这一步
/// 唯一的可见效果。
fn oust_the_dead(table: &Table) {
    for row in table.rows() {
        let Slot::Live { team, .. } = row.slot else {
            continue;
        };
        let before = runtime::env::unit::heir_count().unwrap_or(0);
        let ousted = runtime::env::unit::oust(team).is_ok();
        let after = runtime::env::unit::heir_count().unwrap_or(0);
        let _ = runtime::env::debug::put(&alloc::format!(
            "root: oust {} ousted={ousted} heir={before}→{after}",
            row.name.as_str()
        ));
    }
}
