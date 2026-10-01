#![no_std]
#![no_main]

//! probe-denied — **负证客人**：一位**没有身份**的任务去撞树的门，期望被拒。
//!
//! 门禁那条判据里有一格是"**没绑身份 ⇒ 拒绝**"（`operator::core::judge` 的第一格）。在这一台之前，
//! 真机上**没有反例**：11 台客人全都是装配期绑好的身份，全部放行——那条判据只在宿主靶上喂假事
//! 实证过（**那台靶已删**，用户裁定"protocol-case 没必要"）。本程序就是把反例搬到真机上。
//!
//! ```text
//!   1  树那条路：seat(树) + claim(生我者, 树) + 另铸一枚问话孔给持树者
//!   2  LAND 一枚自己的孔到 /svc/probe  ⇒ 期望 DENIED（本域没身份）
//!   3  SEEK /svc/probe                ⇒ 期望 UNKNOWN（**拒绝不是换绑**：那一格没被占）
//!   4  报一行读数就退场
//! ```
//!
//! # 为什么"没身份"这件事落在装配表上
//!
//! 装配期每一条服务的 `derive(ROOT)` + `bind` 都是装配者做的；本域要**真的没身份**，就只能
//! 由装配者**不绑它**——`UnitFile::bind = false`（见 `programs/src/service.rs`）。
//! 本域自己不做任何"放弃身份"的动作：若自己 `waive`，那也只是回到起点，仍是已绑。
//!
//! # 两条判据为什么缺一不可
//!
//! - `land != OK`（应是 `DENIED`）：**拒得住**。这一格松掉，门禁就成了一条"不去绑身份即可
//!   绕过"的后门；
//! - 随后 `seek == UNKNOWN`：**拒绝发生在动树之前**。若被拒的那一手顺手把那一格占了，
//!   "拒绝"与"换绑"就分不开了——那正是把裁决放在 `tree.land` 之前要买的东西。
//!
//! # 照实记：它为什么也挂树上（`operator: true`）
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

use env::Wait;
use programs::Report;

use alloc::format;
use alloc::string::ToString;
use protocol::communication::session::Session;
use protocol::debug;
use protocol::service::operator::client as operator;
use protocol::service::operator::client::{Face as TreeFace, Mine};
use protocol::service::operator::{Fail, Permit};

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
///
/// **照实记（搬进用例之后）**：`BAD_NOTE`、以及"没走通"那条退场路，一起退役了——判据现在是
/// **一例一条**（`cases::Suite`），失败走 panic 通道、域当场死，故失败再也走不到出口那一手。
const OK_NOTE: &str = "probe-denied: denied";

#[programs::entry]
fn main() -> Report<'static> {
    let sire = utask::sire();

    // 一、与树开会话：本端那一枚交给生我者（它再转授给持树者），另铸一枚问话孔给它。
    //
    // **照实记（这一台为什么整体改走 `Face`，task-2 那一刀）**：本台每一问（`part` / `seek` /
    // `land`）都在 [`TreeFace`] 的面上，裸孔一个都不用 ⇒ 交给（吃所有权的）[`TreeFace::of`]；
    // 那两问也从"坐标 + 名字 + 裸布尔"改说成"那块 Pane 上的两手"（`Pane::open` / `Pane::bind`）。
    let Ok(session) = Session::open(sire, operator::BERTH, Wait::AtMost(MS)) else {
        return bail("probe-denied: no tree link");
    };
    let tree = TreeFace::of(session);

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
            // **读数带那一格码**：`bail` 那句话只说"没拿到"（旧注里那句 `/svc` 也是历史），
            // 而"为什么"——门禁判"不"还是"判不了"、还是根本没走到——只有这行说得清。
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
    // **照实记（这一格为什么走 `Pane::tile` 而不是 `Face::tile`）**：旧面用的是 `seek`
    // （只译号，**不动树**）；新面若用 `entry`，它内部会 `find`——而 `find` 对"主人没了"的
    // 那一格答 `Dead` **并顺手剔掉那一格**（`operator::core` 的 `find`），那是读取之外的一笔账。
    // `Pane::tile` 是旧 `seek` 的同形。
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

    // 五、判据：**一例一条**（原先两格 `&&` 成一句）。名字即结论。
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
