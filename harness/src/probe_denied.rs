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
//! # 照实记（第 60 轮：这一格的**裁定是"撤"**，以及落法为什么换成子任务）
//!
//! 用户裁定 `Relation::bind` **撤**。撤之前量清了两件：
//!   · **自己"放弃身份"这条路不存在**（`service/principal/core.rs` 的 `waive`：`row.current =
//!     row.origin`——只是回到起点，仍是已绑，与上面那段照实记一致）；
//!   · **门禁按"发送那一枚线程"认人**（`operator/core/judge.rs`：`judge(f, who: TaskId, …)`，
//!     `who` 来自帧的 `from`）⇒ **一位子任务往本域的 talk 孔里推，门禁认的就是那一位子任务**。
//! ⇒ 原以为落法是"**本域生一位子任务**（装配者从没绑过它）去撞门"。**第 61 轮把这条也量掉了：
//! 它不成立**——这机器里造任务要 `utask::build(镜像, kind)` ＋ `spawn`（`root/main.rs` 用**清单**里
//! 那一段、装配者用 `Source` 里那一段），**单元自己手里没有镜像** ⇒ **一台程序生不出子任务**。
//!
//! **改道（第 61 轮的结论）**：负证要的是"**真没身份的那一位在撞门**"，那么最省的落法是让
//! **身份的持有者自己把它丢掉**——即给名册那一族补一手"**弃**"（今天的 `waive` 是"回到起点"，
//! 见上；"丢掉"是另一件事）。落点三处：
//!   · 名册核心：一行"把这一格当前号置空"（与 `bind` 相对）；
//!   · 协议：`Grant::Set` 那一面添一手（`Wire` 加一格）；
//!   · 本域：起手照旧被绑，随后**自己丢掉**，再去撞门与 `seek`（读数仍是 `probe-denied: denied`）。
//! **代价对照**：另一条路是(A)"**装配者不再替任何台绑**、需要身份的 ~22 台起手自己领"——不动协议，
//! 但 22 台各加一步。**两条都兑现"撤"**；第 61 轮选"自己丢掉"（只动一处 ＋ 一手 ＋ 这一台）。

//! ## 精确点位（第 62 轮量的，下一轮照这张表一次做完）
//!
//! | # | 那一处 | 那一行/那一格 | 怎么改 |
//! |---|---|---|---|
//! | 1 | 名册核心 `service/principal/core.rs` | `waive`（`row.current = row.origin`）旁边 | 加一手"**丢**"：把那一格的当前号**置空**（与 `bind` 相对；`waive` 是"回到起点"、身份还在，两件事） |
//! | 2 | 协议 `crates/protocol/src/service/principal/grant.rs` | `Wire` 那一型 ＋ `faces!` 里那张"线上码 → 哪一面"的表 | 加一格（落 `Grant::Set`）——**连带帧长/记号表那一套要一起对**（那一套是一处不改就编不过的） |
//! | 3 | 服务端 `service/principal/server.rs` | 分派那一串（`Wire::Bind(…)` / `Wire::Waive` 那几行旁） | 加一支：`Wire::Drop => match book.drop(from) { … }` |
//! | 4 | 本域（这一份） | 起手那几趟（树那条路已在） | 照 `probe_rule` 那一台的写法**找名册的 `Grant::Set` 面**，再叫那一手；**然后**才去撞门与 `seek` |
//!
//! **为什么四件一起做**：①③④ 少一件都编不过或行为不对（一手没人叫＝死格；有格没分派＝问不通）；
//! ②那一套（`Wire` ＋ 表的映射 ＋ 帧长）是**同一条链**，改一半编不过。

//! ## 那条链的**逐字**（第 63 轮读完，下一轮照抄）
//!
//! `frame.rs` 的 op 码现在是 `BIND = 1 … WAIVE = 7`；客侧那一手是
//! `self.face.call(frame::Req::Waive, wait)?` ＋ `decode(reply)`。故：
//!
//! | # | 文件 | 逐字加什么 |
//! |---|---|---|
//! | 1 | `.../principal/frame.rs` | `pub const DROP: u8 = 8;`（`WAIVE = 7` 旁）· `Wire` 加变体 `Drop` · `take` 里加一支 `DROP => Some(Wire::Drop)` · 客侧那一型加 `Req::Drop` 与它的编码 |
//! | 2 | `.../principal/grant.rs` | 那张"线上码 → 哪一面"的表加 `Wire::Drop => Set`；头注里"七条原语"那几处改成八条 |
//! | 3 | `.../principal/client.rs` | 照 `waive` 逐字：`pub fn drop(&self, wait: Wait) -> Result<(), Fail> { let reply = self.face.call(frame::Req::Drop, wait)?; decode(reply).map(|_| ()) }` |
//! | 4 | `.../principal/core.rs` | `pub fn drop(&mut self, from: TaskId) -> Result<(), Fail>`：找到那一格、把它的**当前号置空**（与 `bind` 相对；`waive` 是写回 `origin`、身份还在——两件事，注释里点明） |
//! | 5 | `.../principal/server.rs` | 分派那一串照 `Wire::Waive` 那一支逐字加一支 `pcall::Wire::Drop => match book.drop(from) { … }` |
//! | 6 | 本域 | 照 `probe_rule` 的写法找名册 `Grant::Set` 面 → 叫 `drop` → 再去撞门与 `seek` |
//!
//! **①②是一条链**（`Wire` ／ `Req` ／ 映射表 ／ 帧长依次对），**④⑤各自要与其上面对齐**，
//! **⑥必须最后**（没有它，前五件就是一组没人叫的格）。故六件**一次做完**才落。
//!
//! **落地顺序**（第 60 轮起）：① 本文件改成"生子任务、由它撞门"（读数仍是
//! `probe-denied: denied`，两档 16 条对齐）；② 主刀：`Relation::bind` 退场（字段 ＋ `DEFAULT` ＋
//! 23 份声明那几行），装配者那一手改成**照旧全绑**（`Roster::bind(task)`，去掉 `on`），本文件与
//! `decl/harness.rs` 的说法跟着改；`Relation::bind` 的账改成注释留档（这层的规矩：账不随格子删）。
//!
//! # 为什么"没身份"这件事落在装配表上（**这一节是撤之前的原话，留档**）
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
