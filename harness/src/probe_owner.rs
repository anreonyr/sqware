#![no_std]
#![no_main]

//! probe-owner — **负证客人（第二种）**：一位**有身份**的任务去顶别人声明归自己的那一格。
//!
//! [`probe_denied`](super::probe_denied) 证的是"**没身份** ⇒ 拒绝"；本程序证的是另一半：
//! **有身份、但那一格不是你的** ⇒ 也拒绝。两条合起来，`land` 的两条支路才算在真机上钉住。
//!
//! ```text
//!   1  树那条路：seat(树) + claim(生我者, 树) + 另铸一枚问话孔给持树者
//!   2  SEEK  /svc/drv/uart/rx         ⇒ 记下 uart 那枚砖**原来的号**
//!   3  LAND  /svc/drv/uart/rx（自己的孔）⇒ 期望 DENIED（那枚砖是 uart 的：它声明了归属）
//!   3.5 PART 同一块 Pane 里同一个名字 ⇒ **同一条「改」轴**，同样期望 DENIED（见下）
//!   4  SEEK  /svc/drv/uart/rx         ⇒ 期望**还是原来那个号**（拒绝没有动那一格）
//!   5  **等** `/svc/lease` 那一格的主人退场（`probe-lease` 落完就走）⇒ 再落一次
//!      ⇒ 期望**接得上**（主人不在场 ⇒ 那一格重新可落）
//!   6  报读数就退场
//! ```
//!
//! 第 5 步是"规矩属于**活着的**主人"那一格的正证：`probe-lease` 声明归属之后直接死，
//! 内核退场钩子把它开的资源封印 ⇒ 持树者一问就知道主人不在场 ⇒ 那一格不该变成墓碑。
//!
//! # 两格读数为什么与 `probe-denied` 不同
//!
//! `probe-denied` 撞的是**一个从没铸过的名字**，故它的第二格是 `UNKNOWN`（"没被占"）。
//! 本域撞的是**已经在的名字**，故第二格必须是**同一个号**——"拒绝"不能把原来的格子弄坏，
//! 也不能把它变成"剪掉"。两台的第二格形状**故意不一样**，各自钉一支。
//!
//! # 为什么它必须有身份（`bind: true`）
//!
//! 这一台要证的正是"身份**对不上**"，故它自己得是个**已绑身份**——否则它撞到的是第一道
//! 门（没身份），量到的就不是归属那一条了。装配表上它与别的客人一样（`bind` 缺省即 `true`）。

extern crate alloc;
extern crate programs;

use env::Wait;
use programs::Report;

use alloc::format;
use protocol::communication::session::Session;
use protocol::debug;
use protocol::system::operator::client as operator;
use protocol::system::operator::client::{Face as TreeFace, Mine, Pane};
use protocol::system::operator::path::Path;
use protocol::system::operator::{EntryId, Fail, Permit};

use protocol::driver;
use runtime::env::mail;
use runtime::env::unit as utask;

/// 本域要顶的那**一枚砖**：`/svc/drv/uart/rx`——`uart` 把"读行"那枚孔挂在它下面，并声明
/// **归自己**。
///
/// **照实记（为什么不是 `/svc/drv/uart`）**：控制台是**双向**的，故 `uart` 那一格从一枚砖变成
/// **一块 Pane**（`rx` / `tx` 两枚门牌），而**归属声明在砖上**——顶那块 Pane 本身没有意义
/// （它不是谁的服务格）。这一趟顶的是读口那一枚。
///
/// **照实记（`/sys` → `/svc`、`/device` → `/svc/drv` 那一刀）**：那一段目录从前由
/// [`driver::DIR`] 一处给（一段路）；后来它是两段（`/svc` ＋ `/svc/drv`），
/// 故本台那一条路也从三段变四段；今天那两段收成**一条常量** [`driver::ROAD`]。
///
/// 服务那一格（Pane）。
const SERVICE: &str = "uart";
/// 砖那一格（`uart` 声明的归属落在这一枚上）：读口。
const ME: &str = "rx";

/// 等树 / 办一趟的总上限（毫秒）。**必须有界**：对面死在头几步时本域不能陪着挂死。
const MS: usize = 1000;

const E_OK: usize = 0;
const E_TRIP: usize = 1;

/// 走通那一句（不是 panic；kernel 会把这一句连同域号打出来）。
///
/// **照实记（搬进用例之后）**：`BAD_NOTE`、以及"没走通"那条退场路，一起退役了——判据现在是
/// **一例一条**（`cases::Suite`），失败走 panic 通道、域当场死，故失败再也走不到出口那一手。
const OK_NOTE: &str = "probe-owner: owner rule held";

#[programs::entry]
fn main() -> Report<'static> {
    let sire = utask::sire();

    // 一、与树开会话（同 `canonical` / `probe-denied`）。
    //
    // **照实记（这一台为什么整体改走 `Face`，task-2 那一刀）**：本台每一问（`part` / `seek` /
    // `land`）都在 [`TreeFace`] 的面上，那条线上的裸孔一个都不用 ⇒ 交给（吃所有权的）
    // [`TreeFace::of`]。下面三个帮手一并从"裸 `(say, link, host)`"改收 `&TreeFace`——它们要的
    // 每一个动作都由这一面答，故不必再把那条线拆开传。
    let Ok(session) = Session::open(sire, operator::BERTH, Wait::AtMost(MS)) else {
        return bail("probe-owner: no tree link");
    };
    let tree = TreeFace::of(session);

    // 路：驱动那一族的常量（`/svc/drv`）接上服务名与砖名——一处都不自己拼。
    let Some(road) = driver::ROAD
        .try_join(SERVICE)
        .and_then(|road| road.try_join(ME))
    else {
        return bail("probe-owner: bad name");
    };

    // 二、那枚砖**原来**的号（`uart` 落的）。**有界重试**：本域可能比 `uart` 先起。
    let Some(before) = wait_id(&tree, &road) else {
        return bail("probe-owner: no /svc/drv/uart/rx");
    };

    // 三、铸一枚自己的孔，去顶那一格——**这一手该被拒**。
    let Ok(entry) = mail::unseal_hole(env::Mark::of("probe-entry")) else {
        return bail("probe-owner: no entry");
    };
    // `/svc/drv/uart` 那块 Pane（要顶的那枚砖落在它下面）——**分目录幂等 + 取回那块 Pane**。
    // 它是**那条路去掉末段**（`parent()`，std 同形）：本手因此不必再念一遍那几段。
    let Some(pane_road) = road.parent() else {
        return bail("probe-owner: no /svc/drv/uart");
    };
    let Some(pane) = wait_pane(&tree, &pane_road) else {
        return bail("probe-owner: no /svc/drv/uart");
    };
    // 那枚砖的名就是**那条路的末段**（`file_name()`，std 同形）——不再单独持一格。
    let Some(me) = road.file_name() else {
        return bail("probe-owner: no /svc/drv/uart/rx");
    };
    let land = pane.bind(me, entry, Permit::Unset, Mine::No, Wait::AtMost(MS));
    let land_code = match &land {
        Ok(id) => format!("ok id={}", id.id().get()),
        Err(fail) => format!("{fail:?}"),
    };

    // 三·五、**同一个名字、换一条原语**：`part` 与 `land` 同一把钥匙（`answer.rs` 的 `Part` 那一
    //        支：动手之前按坐标问同一格 `claimable`）——不然它会把那一格**静默顶成一块 Pane**、
    //        还顺手把 uart 那枚孔 `release` 掉。
    //
    // **照实记（这一条此前零断言）**：`land` 那一支有本台顶着，`part` 这一支**没有**——两条原语
    // 走同一把钥匙，可只有一条被量过。这一格补的就是那一半。
    let part = pane.open(me, Wait::AtMost(MS));
    let part_code = match &part {
        Ok(id) => format!("ok id={}", id.id().get()),
        Err(fail) => format!("{fail:?}"),
    };

    // 四、那一格**还在不在**（应是原来那个号）。
    //
    // **照实记（这一格为什么也走 `Pane::tile`）**：旧面用 `seek`（只译号、不动树）；新面若用
    // `Face::tile`，它内部那一趟 `find` 会**授一枚副本**进来（旧面没有这一笔）——而这一格只要号。
    let root = tree.root();
    let after = root.tile(&road, Wait::AtMost(MS));
    let seq = match &after {
        Ok(entry) => format!("id={}", entry.id().get()),
        Err(fail) => format!("err:{fail:?}"),
    };
    debug!(
        "probe-owner: tree land={land_code} part={part_code} before={} after={seq}",
        before.get()
    );

    // 五、判据两格：被拒（`Denied`）**且**那一格没动（还是原来那个号）。
    let denied = matches!(land, Err(Fail::Denied));
    let untouched = matches!(after, Ok(entry) if entry.id() == before);

    // 六、**接手那一格没主的名字**：`probe-lease` 落完 `/svc/lease`（`mine = true`）就死，
    //     故它的资源已被退场钩子封印 ⇒ 持树者该让那一格重新可落。**有界重试**：本域可能
    //     比它先跑完那几手（提示是单槽，装配者按计划顺序推）。
    let taken = take_over(&tree);

    debug!(
        "probe-owner: lease land={} (owner gone ⇒ take-over)",
        match taken {
            Ok(id) => format!("0 id={}", id.get()),
            Err(fail) => format!("{fail:?}"),
        }
    );

    // 七、判据：**一例一条**（原先三格 `&&` 成一句）。
    let took = taken.is_ok();
    {
        {
            assert!(
                denied,
                "那一格的主人还活着，land 本该被拒（land={land_code}）"
            )
        }
    }
    {
        // **同一条「改」轴的另一半**：`part` 也走那一把钥匙。
        assert!(
            matches!(part, Err(Fail::Denied)),
            "那一格的主人还活着，part 本该被拒（part={part_code}）"
        )
    }
    {
        assert!(untouched, "被拒之后那一格换号了（不再是 before 那个号）")
    }
    {
        assert!(took, "probe-lease 已经死了，那一格该重新可落")
    }

    return Report::note(E_OK, OK_NOTE);
}

/// 落 `/svc/lease`——**那一格的主人（`probe-lease`）已经退场**，故这一次该接得上。
///
/// 有界重试：对面那台与本域并行起来，"它死了没有"要看读数而不是靠猜。
///
/// **照实记（收 `&TreeFace`，task-2 那一刀）**：三问全是面上的方法（`open` / `road` / `bind`），
/// 故不再收裸 `(say, link, host)`——对端号与那条线都在 `Face` 里面。"接不上"这一档的落点从
/// 线上那一格码收成 [`Fail`]（`BAD` / `UNKNOWN` / 没走到同落 [`Fail::Unknown`]）。
///
/// **照实记（那一格的存在性为什么走 `Pane::tile` 而不是 `Face::tile`）**：旧面用 `seek`
/// ——只译号，**不动树**。新面若用 `Face::tile`，它内部会 `find` 一次，而 `find` 对"主人没了"
/// 的那一格答 [`Fail::Dead`] **并顺手把那一格从树上剔掉**（见 `operator::core` 的 `find`）——
/// 于是这一格的判据（"那一格还在，只是主人不在场 ⇒ 可接手"）当场翻面：存在性答假、格子还被删了。
/// `Pane::tile` 才是旧 `seek` 的同形（只译号），故这一手用它。
fn take_over(tree: &TreeFace) -> Result<EntryId, Fail> {
    // 路：容器那一段（`/svc`，只在协议那一侧说）接上那一格的名（`lease`）。
    let road = protocol::system::SVC
        .try_join("lease")
        .ok_or(Fail::Unknown)?;
    let me = road.file_name().ok_or(Fail::Unknown)?;
    let Some(dir) = protocol::system::SVC.file_name() else {
        return Err(Fail::Unknown);
    };
    // `/svc` 那块 Pane（分目录**幂等**，再取回那块 Pane）。
    let root = tree.root();
    let _ = root.open(dir, Wait::AtMost(MS));
    let Some(sys) = tree.pane(&protocol::system::SVC, Wait::AtMost(MS)).ok() else {
        return Err(Fail::Unknown);
    };
    let mut left = MS;
    loop {
        // 那一格先得**已经在树上**（`probe-lease` 落过）——否则本域量的是"落一个新名字"。
        if root.tile(&road, Wait::AtMost(MS)).is_ok() {
            let Ok(entry) = mail::unseal_hole(env::Mark::of("takeover-entry")) else {
                return Err(Fail::Unknown);
            };
            match sys.bind(me, entry, Permit::Unset, Mine::No, Wait::AtMost(MS)) {
                Ok(id) => return Ok(id.id()),
                Err(Fail::Denied) if left > 0 => {
                    // 还没死透（或我们比它先到）：等一下再来。
                    let _ = runtime::env::room::sleep(core::time::Duration::from_millis(1));
                    left = left.saturating_sub(1);
                }
                Err(fail) => return Err(fail),
            }
        } else if left > 0 {
            let _ = runtime::env::room::sleep(core::time::Duration::from_millis(1));
            left = left.saturating_sub(1);
        } else {
            return Err(Fail::Unknown);
        }
    }
}

/// `/svc/drv/uart` 那块 Pane（分目录**幂等三趟** + 取回那块 Pane）：要顶的那枚砖落在它下面。
///
/// **照实记（两段 → 三段那一刀，以及"趟数"这件事）**：驱动那一段路从 `/device`（顶上一层）
/// 变成 `/svc/drv` ⇒ 容器链从两段变三段，那一版的本手要跟着多一趟。**这一格当场栽过**（实测）：
/// 只把 `dir`（`driver::DIR`）换成新名字、忘了它上面还有 `driver::SVC`，于是本手在
/// `/drv/uart` 那**另一块** Pane 上落砖——落在一块**没有主人**的新格上，当然不被拒，
/// `probe-owner` 当场红（`land=ok id=26`，而基线是 `owner rule held`）。
///
/// 今天这一手**不再自己数趟数**：一趟一条路（[`Path`] 自带段数），逐段 `open`（幂等）＋ 最后
/// 取回那一块 Pane。"忘掉头一段"那一类错在形状上写不出来了。
fn wait_pane<'a>(tree: &'a TreeFace, road: &Path) -> Option<Pane<'a>> {
    let mut at: Option<EntryId> = None;
    for seg in road.iter() {
        let here = match at {
            Some(id) => Pane::of(tree, id),
            None => tree.root(),
        };
        if let Ok(next) = here.open(*seg, Wait::AtMost(MS)) {
            at = Some(next.id());
        }
    }
    tree.pane(road, Wait::AtMost(MS)).ok()
}

/// 等 `uart` 把门牌落上（有界）：本域可能与它并行起来。
///
/// **照实记（同上：`Pane::tile` 是旧 `seek` 的同形）**：这一格只要那一枚**号**，不要那一枚
/// 门闩——故不走会 `find`（并惰性剔死 / 授一枚副本）的 `Face::tile`。
fn wait_id(tree: &TreeFace, road: &Path) -> Option<EntryId> {
    let root = tree.root();
    let mut left = MS;
    loop {
        match root.tile(road, Wait::AtMost(MS)) {
            Ok(entry) => return Some(entry.id()),
            Err(Fail::Unknown) if left > 0 => {
                let _ = runtime::env::room::sleep(core::time::Duration::from_millis(1));
                left = left.saturating_sub(1);
            }
            Err(_) => return None,
        }
    }
}

/// 哪里算不下去就报哪一句（kernel 收场时把这一句连同域号打出来）。
fn bail<'a>(note: &'a str) -> Report<'a> {
    debug!("{}", note);
    return Report::note(E_TRIP, note);
}
