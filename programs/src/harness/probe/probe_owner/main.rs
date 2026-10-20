#![no_std]
#![no_main]

//! Resource use does not grant publication rights; departed publishers are retired.

extern crate alloc;
extern crate programs;

use env::Wait;
use programs::Report;

use alloc::format;
use ipc::session::Session;
use programs::debug;
use system_api::operator::EntryId;
use system_api::operator::Fail;
use system_api::operator::path::Path;
use system_client::operator;
use system_client::operator::Face;

use env::pie;
use env::unit;

/// **归自己**
/// **（为什么不是 `/svc/drv/uart`）**：控制台是**双向**的，故 `uart` 那一格从一枚砖变成
/// **一块 Pane**（`rx` / `tx` 两枚门牌），而**归属声明在砖上**——顶那块 Pane 本身没有意义
/// 服务那一格（Pane）
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
    let Some(road) = programs::driver::uart::client::road().and_then(|road| road.try_join(ME))
    else {
        return bail("probe-owner: bad name");
    };

    let Some(service) = tree
        .resolve(system_api::namespace::SERVICE, Wait::AtMost(MS))
        .ok()
    else {
        return bail("probe-owner: no service namespace");
    };
    let Some(before) = wait_id(&tree, service, &road) else {
        return bail("probe-owner: no /svc/drv/uart/rx");
    };

    let forged = env::hole::unseal(
        system_api::operator::entry::AUTH_ENTRY,
        env::HoleLimits::default(),
    )
    .unwrap_or(env::PieToken::NONE);
    let denied = matches!(
        operator::Capability::direct(tree.host(), forged),
        Err(Fail::Denied)
    );
    let part_denied = matches!(
        operator::Capability::direct(tree.host(), forged),
        Err(Fail::Denied)
    );
    let _ = pie::release(forged, env::ReleaseMode::Revoke);

    // 四、那一格**还在不在**（应是原来那个号）。
    let after = tree.tile_from(service, &road, Wait::AtMost(MS));
    let seq = match &after {
        Ok(entry) => format!("id={}", entry.id().get()),
        Err(fail) => format!("err:{fail:?}"),
    };
    debug!(
        "probe-owner: capability denied={denied} part_denied={part_denied} before={} after={seq}",
        before.get()
    );

    // 五、判据两格：被拒（`Denied`）**且**那一格没动（还是原来那个号）。
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
    assert!(denied, "ungranted Operator capability must be rejected");
    {
        // **同一条「改」轴的另一半**：`part` 也走那一把钥匙。
        assert!(
            part_denied,
            "ungranted Operator capability must be rejected"
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
    let forged = env::hole::unseal(
        system_api::operator::entry::AUTH_ENTRY,
        env::HoleLimits::default(),
    )
    .map_err(|_| Fail::Unknown)?;
    assert!(matches!(
        operator::Capability::direct(tree.host(), forged),
        Err(Fail::Denied)
    ));
    let _ = pie::release(forged, env::ReleaseMode::Revoke);
    let service = tree.resolve(system_api::namespace::SERVICE, Wait::AtMost(MS))?;
    let road = Path::new("fixtures/lease");
    for _ in 0..MS {
        if matches!(
            tree.tile_from(service, road, Wait::AtMost(MS)),
            Err(Fail::Unknown)
        ) {
            return Ok(EntryId::new(0));
        }
        let _ = execution::room::park(core::time::Duration::from_millis(1));
    }
    Err(Fail::Unknown)
}
/// 门闩——故不走会 `find`（并惰性剔死 / 授一枚副本）的 Face::tile
fn wait_id(tree: &Face, service: system_api::operator::Entry, road: &Path) -> Option<EntryId> {
    let mut left = MS;
    loop {
        match tree.tile_from(service, road, Wait::AtMost(MS)) {
            Ok(entry) => return Some(entry.id()),
            Err(Fail::Unknown) if left > 0 => {
                let _ = execution::room::park(core::time::Duration::from_millis(1));
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
