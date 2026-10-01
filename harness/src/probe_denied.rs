#![no_std]
#![no_main]

//! probe-denied — **负证客人**：一位**没有身份**的任务去撞树的门，期望被拒。
//!
//! 门禁那条判据里有一格是"**没绑身份 ⇒ 拒绝**"（`operator::core::judge` 的第一格）。
//! 真机上没有反例：所有客人都是装配期绑好的身份，全部放行。本程序就是把反例搬到真机上。
//!
//! ```text
//!   1  树那条路：seat(树) + claim(生我者, 树) + 另铸一枚问话孔给持树者
//!   2  起手照旧被装配者绑过；本域主动**问** `Grant::Ask` 面拿自己的号，再**写** `Grant::Set`
//!      面 `drop` 把身份从名册里删掉（"丢"是 `Drop` 那手，不是 `Waive`：前者把那一行删掉，
//!      `resolve` 答 `None` ⇒ "没身份"；后者只是把 `current` 写回 `origin`，身份仍在）。
//!   3  LAND 一枚自己的孔到 /svc/probe  ⇒ 期望 DENIED（本域没身份）
//!   4  SEEK /svc/probe                ⇒ 期望 UNKNOWN（**拒绝不是换绑**：那一格没被占）
//!   5  报一行读数就退场
//! ```
//!
//! # 为什么这样落到"自己丢掉"那一档
//!
//! 门禁按**发送那一枚线程**认人（`operator::core::judge`：`judge(f, who: TaskId, …)`，
//! `who` 来自帧的 `from`），故负证必须是"**真没身份的那一位在撞门**"，不是别的身份替身。
//! 唯一能造出"真没身份"的人就是**装配者**——但本仓"该绑就绑"是装配表的默认，
//! 那条路要 22 台起手各加一步。本域改走"**自己丢掉**"：装配者照旧全绑，本域起手后
//! 立刻 `drop` 自己——判据"撞门答 `DENIED`、`seek` 答 `UNKNOWN`"不变。
//!
//! # 两条判据为什么缺一不可
//!
//! - `land != OK`（应是 `DENIED`）：**拒得住**。这一格松掉，门禁就成了一条"不去绑身份即可
//!   绕过"的后门；
//! - 随后 `seek == UNKNOWN`：**拒绝发生在动树之前**。若被拒的那一手顺手把那一格占了，
//!   "拒绝"与"换绑"就分不开了——那正是把裁决放在 `tree.land` 之前要买的东西。
//!
//! # 它为什么也挂树上（`operator: true`）
//!
//! 没有树那条路就撞不到门。而"没身份"与"有树路"并不冲突：树路是**装配期发的一条通道**
//! （`operator::attach`），身份是**名册里的一格**（`derive` + `bind`）——这一台正是要把这两件
//! 事分开读出来。

// 本文件是一份**独立的 bin**（`harness/Cargo.toml` 的 `prog-probe-denied`），**不进 lib**
// ——与 `canonical` / `guest` 同一条：`programs/src/user/mod.rs` 里没有它。
//
// 两条 `extern crate` 缺一不可（实测）：`alloc` 是 `format!` 要用；`programs` **不是**为了
// 用它里面的东西，而是为了把 `libprograms` 链进来——**panic handler 与 `_start` 都住那份
// lib**（`programs/src/entry.rs`）。少了它，链接期报 `` `#[panic_handler]` function required ``。
extern crate alloc;
extern crate programs;

use env::PieToken;
use env::Wait;
use programs::Report;
use protocol::service::operator::path::Path;

use alloc::format;
use alloc::string::ToString;
use protocol::communication::session::Session;
use protocol::debug;
use protocol::service::operator::client as operator;
use protocol::service::operator::client::{Face as TreeFace, Mine};
use protocol::service::operator::{Fail, Permit};
use protocol::service::principal as pcall;
use protocol::service::principal::client::Face as PrincipalFace;

use runtime::env::mail;
use runtime::env::unit as utask;

/// 本域要落的那一格的名字（在根下，**不进 `/svc/drv`**：本域不是设备）。
const ME: &str = "probe";

/// 等树 / 办一趟的总上限（毫秒）。**必须有界**：对面死在头几步时本域不能陪着挂死。
const MS: usize = 1000;

/// 本地失败编号（读数用）。
const E_OK: usize = 0;
const E_TRIP: usize = 1;

/// 走通那一句（不是 panic；kernel 会把这一句连同域号打出来）。
const OK_NOTE: &str = "probe-denied: denied";

#[programs::entry]
fn main() -> Report<'static> {
    let sire = utask::sire();

    // 一、与树开会话：本端那一枚交给生我者（它再转授给持树者），另铸一枚问话孔给它。
    let Ok(session) = Session::open(sire, operator::BERTH, Wait::AtMost(MS)) else {
        return bail("probe-denied: no tree link");
    };
    let tree = TreeFace::of(session);
    // 丢掉自己的身份：本域照旧被装配者绑过，这里主动丢掉（"丢"是 `Drop` 那手，不是 `Waive`）。
    // 读走 `Ask` 面、写走 `Set` 面：把读手（`Resolve`）问在写面上会被门拒。
    let Some(aroad) = pcall::DIR.try_join(pcall::Grant::Ask.name()) else {
        return bail("probe-denied: bad principal ask name");
    };
    let Some(aentry) = find_face(&tree, &aroad) else {
        return bail("probe-denied: no principal ask face");
    };
    let Ok(aread) = PrincipalFace::of(aentry) else {
        return bail("probe-denied: bad principal ask face");
    };
    let ask_view = aread.task(utask::self_id());
    let Ok(Some(who)) = ask_view.principal(Wait::AtMost(MS)) else {
        return bail("probe-denied: no identity to drop");
    };
    let Some(sroad) = pcall::DIR.try_join(pcall::Grant::Set.name()) else {
        return bail("probe-denied: bad principal set name");
    };
    let Some(sentry) = find_face(&tree, &sroad) else {
        return bail("probe-denied: no principal set face");
    };
    let Ok(awrite) = PrincipalFace::of(sentry) else {
        return bail("probe-denied: bad principal set face");
    };
    if awrite.principal(who.id()).drop(Wait::AtMost(MS)).is_err() {
        return bail("probe-denied: cannot drop its own identity");
    }

    // 二、铸一枚自己的孔当"要落上去的那一枚"（与 `uart` / `rtc` 上树那一趟同一形状）。
    let Ok(entry) = mail::unseal_hole(env::Mark::of("probe-entry")) else {
        return bail("probe-denied: no entry");
    };
    let Some(dir) = protocol::system::SVC.file_name() else {
        return bail("probe-denied: bad name");
    };
    let me = ME.to_string();

    // 二·二、它要落进 `/svc`（**已经在**：principal / coalition 起的头）——分那一块目录
    // （**幂等**），拿到的就是那块 Pane。**这一手不过门禁**（`part` 不在闸口里），故本域虽然
    // 没有身份，它照旧答得出。
    let root = tree.root();
    let sys = match root.open(dir.to_string(), Wait::AtMost(MS)) {
        Ok(sys) => sys,
        Err(fail) => {
            // 读数带那一格码：`bail` 那句话只说"没拿到"，而"为什么"——门禁判"不"还是
            // "判不了"、还是根本没走到——只有这行说得清。
            debug!("probe-denied: open {} {fail:?}", dir);
            return bail("probe-denied: no /svc");
        }
    };

    // 三、落牌——**这一手该被拒**。
    let land = sys.bind(me, entry, Permit::Unset, Mine::No, Wait::AtMost(MS));
    let land_code = match &land {
        // 居然成了：把号也报出来（读数要能指认"哪一格被占了"）。
        Ok(id) => format!("ok id={}", id.id().get()),
        Err(fail) => format!("{fail:?}"),
    };

    // 四、拒绝之后那一格**在不在**——`Unknown` 才是"没被占"。
    //
    // 这一格走 `Pane::tile` 而不是 `Face::tile`：`Face::tile` 内部会 `find`，
    // 而 `find` 对"主人没了"那一格答 `Dead` 并顺手剔掉那一格（`operator::core` 的 `find`），
    // 那不是只读——存在性答假、格子还被删了。`Pane::tile` 只译号、不动树。
    let Some(road) = protocol::system::SVC.try_join(ME) else {
        return bail("probe-denied: bad name");
    };
    let after = root.tile(&road, Wait::AtMost(MS));
    let seq = match &after {
        Ok(entry) => format!("id={}", entry.id().get()),
        Err(fail) => format!("err:{fail:?}"),
    };
    debug!(
        "probe: tree land={land_code} seek={seq} dir={}",
        sys.id().get()
    );

    // 五、判据：一例一条，名字即结论。
    let denied = matches!(land, Err(Fail::Denied));
    let unplaced = matches!(after, Err(Fail::Unknown));
    {
        assert!(denied, "本该被拒，land={land_code}")
    }
    {
        assert!(unplaced, "拒了，可那一格动过了（seek 答的不是 Unknown）")
    }

    return Report::note(E_OK, OK_NOTE);
}

/// 哪里算不下去就报哪一句（kernel 收场时把这一句连同域号打出来）。
fn bail<'a>(note: &'a str) -> Report<'a> {
    debug!("{}", note);
    return Report::note(E_TRIP, note);
}

fn find_face(tree: &TreeFace, road: &Path) -> Option<PieToken> {
    let root = tree.root();
    let mut left = MS;
    loop {
        match root
            .tile(road, Wait::AtMost(MS))
            .and_then(|entry| entry.token(Wait::AtMost(MS)))
        {
            Ok(entry) => return Some(entry),
            Err(Fail::Unknown) if left > 0 => {
                let _ = runtime::env::room::sleep(core::time::Duration::from_millis(1));
                left = left.saturating_sub(1);
            }
            Err(_) => return None,
        }
    }
}
