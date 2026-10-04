#![no_std]
#![no_main]

//! Resource use does not grant publication rights; departed publishers are retired.

extern crate alloc;
extern crate programs;

use env::Wait;
use programs::Report;

use alloc::format;
use alloc::string::ToString;
use protocol::common::path::Path;
use protocol::communication::session::Session;
use protocol::debug;
use protocol::system::operator::client as operator;
use protocol::system::operator::{Face as Face, Mine, Pane, EntryId, Fail, Permit};

use protocol::driver;
use env::unit;
use env::pie;

/// **归自己**
/// **（为什么不是 `/svc/drv/uart`）**：控制台是**双向**的，故 `uart` 那一格从一枚砖变成
/// **一块 Pane**（`rx` / `tx` 两枚门牌），而**归属声明在砖上**——顶那块 Pane 本身没有意义
/// 服务那一格（Pane）
const SERVICE: &str = "uart";
/// 砖那一格（`uart` 声明的归属落在这一枚上）：读口
const ME: &str = "rx";

const MS: usize = 1000;

const E_OK: usize = 0;
const E_TRIP: usize = 1;

/// 走通那一句（不是 panic；kernel 会把这一句连同域号打出来）
const OK_NOTE: &str = "probe-owner: owner rule held";

#[programs::entry]
fn main() -> Report<'static> {
    let sire = unit::sire();

    // 一、与树开会话（同 `terminal` / `probe-denied`）。
    let Ok(session) = Session::open(sire, operator::BERTH, Wait::AtMost(MS)) else {
        return bail("probe-owner: no tree link");
    };
    let tree = Face::of(session);

    // 路：驱动那一族的常量（`/svc/drv`）接上服务名与砖名——一处都不自己拼。
    let Some(road) = driver::ROAD
        .try_join(SERVICE)
        .and_then(|road| road.try_join(ME))
    else {
        return bail("probe-owner: bad name");
    };

    let Some(before) = wait_id(&tree, &road) else {
        return bail("probe-owner: no /svc/drv/uart/rx");
    };

    let Ok(entry) = pie::unseal_hole(env::Mark::of("probe-entry")) else {
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
    let land = pane.bind(
        me.to_string(),
        entry,
        Permit::Public,
        Mine::No,
        Wait::AtMost(MS),
    );
    let land_code = match &land {
        Ok(id) => format!("ok id={}", id.id().get()),
        Err(fail) => format!("{fail:?}"),
    };

    // 三·五、**同一个名字、换一条原语**：`part` 与 `land` 同一把钥匙（`answer.rs` 的 `Part` 那一
    //        还顺手把 uart 那枚孔 `release` 掉。
    // **（这一条此前零断言）**：`land` 那一支有本台顶着，`part` 这一支**没有**——两条原语
    let part = pane.open(me.to_string(), Wait::AtMost(MS));
    let part_code = match &part {
        Ok(id) => format!("ok id={}", id.id().get()),
        Err(fail) => format!("{fail:?}"),
    };

    // 四、那一格**还在不在**（应是原来那个号）。
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
    //     比它先跑完那几手（提示是单槽，装配者按计划顺序推）。
    let taken = take_over(&tree);

    debug!(
        "probe-owner: lease land={} (owner gone ⇒ retired; raw mutation still denied)",
        match taken {
            Ok(id) => format!("0 id={}", id.get()),
            Err(fail) => format!("{fail:?}"),
        }
    );

    // 七、判据：一例一条。
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
        assert!(took, "departed publisher entry must disappear")
    }

    return Report::note(E_OK, OK_NOTE);
}

fn take_over(tree: &Face) -> Result<EntryId, Fail> {
    let entry = pie::unseal_hole(env::Mark::of("takeover-entry")).map_err(|_| Fail::Unknown)?;
    assert!(matches!(
        tree.root().bind(
            "fixtures".into(),
            entry,
            Permit::Public,
            Mine::No,
            Wait::AtMost(MS)
        ),
        Err(Fail::Denied)
    ));
    let road = Path::new("svc/fixtures/lease");
    for _ in 0..MS {
        if matches!(tree.root().tile(road, Wait::AtMost(MS)), Err(Fail::Unknown)) {
            return Ok(EntryId::new(0));
        }
        let _ = runtime::core::task::sleep(core::time::Duration::from_millis(1));
    }
    Err(Fail::Unknown)
}
fn wait_pane<'a>(tree: &'a Face, road: &Path) -> Option<Pane<'a>> {
    tree.pane(road, Wait::AtMost(MS)).ok()
}

/// 门闩——故不走会 `find`（并惰性剔死 / 授一枚副本）的 Face::tile
fn wait_id(tree: &Face, road: &Path) -> Option<EntryId> {
    let root = tree.root();
    let mut left = MS;
    loop {
        match root.tile(road, Wait::AtMost(MS)) {
            Ok(entry) => return Some(entry.id()),
            Err(Fail::Unknown) if left > 0 => {
                let _ = runtime::core::task::sleep(core::time::Duration::from_millis(1));
                left = left.saturating_sub(1);
            }
            Err(_) => return None,
        }
    }
}

/// 哪里算不下去就报哪一句（kernel 收场时把这一句连同域号打出来）
fn bail<'a>(note: &'a str) -> Report<'a> {
    debug!("{}", note);
    return Report::note(E_TRIP, note);
}
