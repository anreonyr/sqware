//! tree — **上树那一趟**：三台驱动都要走一遍的那段登记（门牌 = 本域的服务入口）。
//!
//! ```text
//!   PART ["device"]            → 0 = 拿到那块目录的号（本域建的 / 已经在了——`part` 幂等）
//!   LAND ["device",<服务名>]    → 0 = 门牌落上（那枚孔经会话交给持树者）
//!   FIND ["device",<服务名>]    → 0 = 查得到，且那一枚经会话授回本域表里
//!   按号问名                    → 号 ↔ 名对得起来，才算那枚号是真坐标
//!   got                         → 本域在表里认出刚授回来的那一枚了吗
//! ```
//!
//! **一份源码三处走**（`router` / `uart` / `rtc`）：三台那一趟**逐字同构**——次序、五条判据、
//! 那一行读数的字段全一样，只差两格：**叫什么**（`me`）与**声不声明归属**（[`Mine`]，
//! `land` 的最后一格）。故它住驱动这一族的一级，与 [`super::assemble`] 同款。
//!
//! `got` 只是"认出了那一枚"；它指不指得回原物，由**真客人**证（`echo` 读行、`sleeper` 问钟、
//! 房客占线）。故本域不自问自答。
//!
//! **失败即断言**（与三处旧文逐字同）：`part` / `land` / `find` 任一非 `OK`、`got` 假、
//! 号 ↔ 名对不上 ⇒ 当场死。它不是一条错误分支，而是"这一域没登记上就不该活着"的判据。

use env::Wait;
use env::{Name, PieToken, TaskId};
use protocol::debug;
use protocol::session::Quay;
use protocol::system::operator as ocall;
use protocol::system::operator::Where;
use protocol::system::operator::client as operator;

/// 门牌那一格声不声明归属（[`operator::land`] 的最后一格）。
///
/// 三台今天都是**公开可查**（`Rule::Public`），只在这一格上分家：`uart` 说"这枚读行的孔是
/// 我的"（[`Mine::Yes`](Mine::Yes)），`rtc` / `router` 不说（[`Mine::No`](Mine::No)）。
#[derive(Clone, Copy)]
pub enum Mine {
    /// 这一格是我的。
    Yes,
    /// 不声明归属。
    No,
}

/// 上树那一趟：**分目录 → 落门牌 → 查回来 → 按号问名**（五条判据 ＋ 一行读数）。
///
/// `me` 既是 `LAND` / `FIND` 的那一段，也是读数前缀——三台是**同一个串**（服务名）。
///
/// 前置：`talk` / `link` / `host` 是本域那**一条** `operator` 会话（同一个域只开一条，见 `driver/uart/adapt/boot.rs` 头注）；`entry` 是本域那枚服务入口。
pub fn plate(
    me: &str,
    mine: Mine,
    link: &Quay,
    talk: PieToken,
    host: TaskId,
    entry: PieToken,
    millis: Wait,
) {
    let (Ok(dir), Ok(name)) = (Name::new(protocol::driver::DIR), Name::new(me)) else {
        debug!("{me}: tree: bad name");
        return;
    };
    // **分目录 → 落门牌 → 查回来验一遍**：分与落各自**答出那一格的号**（"号出门"那一手）。
    // **分目录**：`part` 是**幂等**的——那块目录已经在就答它那个号（里面有没有东西不管）。
    let dir_at = operator::part(talk, link, Where::Root, dir, millis);
    let (part, dir_id) = match dir_at {
        Ok(id) => (ocall::OK, id.get()),
        Err(code) => (code, 0),
    };
    // **落门牌**：答的是门牌自己那一格的号。
    let plate = match dir_at {
        Ok(at) => operator::land(
            talk,
            link,
            host,
            Where::At(at),
            name,
            entry,
            ocall::Rule::Public,
            matches!(mine, Mine::Yes),
            millis,
        ),
        Err(code) => Err(code),
    };
    let (land, pid) = match plate {
        Ok(id) => (ocall::OK, id.get()),
        Err(code) => (code, 0),
    };
    // 查回来验一遍：**按号**（名字只在上面那两格用过，此后一律按号）。
    let (find, got) = match plate {
        Ok(id) => match operator::find(talk, link, id, millis) {
            Ok((code, entry)) => (code, entry.is_some()),
            Err(_) => (ocall::BAD, false),
        },
        Err(code) => (code, false),
    };
    // **`got` 换了来路**（乙′）：见 `ocall::Union::Seed` 的照实记。
    // 拿号问名：**号 ↔ 名**这一对对得起来，才算那枚号是真坐标。
    let pname = plate
        .ok()
        .and_then(|id| operator::name(talk, link, id, millis).ok());
    debug!(
        "{me}: tree part={part} dir={dir_id} land={land} find={find} got={got} entry={} plate={pid} pname={}",
        entry.get(),
        pname.as_ref().map(|n| n.as_str()).unwrap_or("-"),
    );
    // **这一趟的判据**（值那几格从门那边搬进来：门只剩"这一行还在不在"）。
    {
        assert_eq!(part, ocall::OK)
    }
    assert_eq!(land, ocall::OK);
    {
        assert_eq!(find, ocall::OK)
    }
    assert!(got);
    assert_eq!(pname.as_ref().map(|n| n.as_str()), Some(me))
}

