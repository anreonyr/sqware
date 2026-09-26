//! echo::adapt::tree — **上树两趟**：落自己那块牌子（五步）＋ 一串（列号 / 翻名 / 列目录 / 问空号）。
//!
//! 两趟都拿本域自己那一枚入口（`board::ENTRY_MARK` 那枚）：一趟证明树上那四支判据都走得通，
//! 一趟走"名与号分开"那一刀的四格读数。判定全在这里——它们的输入是**会话答话**，不是本域的
//! 本质功能（故纯核 `core/` 不参与）。

use super::{ME, MS};
use alloc::format;
use alloc::string::String;
use env::{Name, PieToken, TaskId, Wait};
use protocol::debug;
use protocol::session::Quay;
use protocol::system::board as bcall;
use protocol::system::operator as ocall;
use protocol::system::operator::client as operator;
use protocol::system::operator::{EntryId, Listing, Where};
use runtime::env::mail;

/// 上树一趟（装配单里本域 `operator: true`）：**分 → 落 → 寻 → 收 → 剪**五步。
///
/// 返最后那一格（剪的答码，`ocall::OK` = 五步都成）。中间任何一步不成 ⇒ 当场的答码就是
/// 返回值——**一格里已经有"死在哪一步"**，不需要另立读数。
///
/// 为什么这五步都要走：树上那四支判据各有各的门（分出第二层、落一枚真 Pie、寻回来把 Pie
/// 经会话授出、剪掉一块空 `Pane`），少走一步就有半条路从来没被走过。挂的是本域自己那一枚
/// 入口（与上板那一枚同一个记号），故它在树上是一枚普通 `Tile`，不是特权。
pub fn trip(link: &Quay, talk: PieToken, host: TaskId) -> u8 {
    let Ok(entry) = mail::unseal_hole(bcall::ENTRY_MARK) else {
        return ocall::BAD;
    };
    let Ok(name) = Name::new(ME) else {
        return ocall::BAD;
    };
    // 分出一块空 `Pane`、再在**同一格**上换绑一枚 `Tile`——**第二层**因此是实打实走出来的。
    // 那两趟落在同一个（容器，名字）上 ⇒ **号不动**：两格答的是同一枚号。
    let part = operator::part(talk, link, Where::Root, name, Wait::AtMost(MS));
    let a = match part {
        Ok(_) => ocall::OK,
        Err(code) => code,
    };
    let plate = operator::land(
        talk,
        link,
        host,
        Where::Root,
        name,
        entry,
        ocall::Rule::Public,
        false,
        Wait::AtMost(MS),
    );
    let b = match plate {
        Ok(_) => ocall::OK,
        Err(code) => code,
    };
    // 寻回来那一趟：**按号**（名字只在上面用过，此后一律按号）。
    let (c, got) = match plate {
        Ok(id) => match operator::find(talk, link, id, Wait::AtMost(MS)) {
            Ok((code, entry)) => (code, entry.is_some()),
            Err(_) => (ocall::BAD, false),
        },
        Err(code) => (code, false),
    };
    // **`got` 换了来路**（乙′）：从前是"扫本端表、按'谁给的'认出一枚"，今天是"答话里带回了
    // 那一格"——判据由持树者那侧一次 `Reserve` 验过（见 `ocall::Union::Seed`）。
    // 拿号问名——这一格**下一步就被剪掉**，故号与名都得赶在 `trim` 之前取。
    let pname = plate
        .ok()
        .and_then(|id| operator::name(talk, link, id, Wait::AtMost(MS)).ok());
    let d = match plate {
        Ok(id) => operator::trim(talk, link, id, Wait::AtMost(MS)).unwrap_or(ocall::BAD),
        Err(code) => code,
    };
    debug!(
        "echo: tree part={a} land={b} find={c} got={got} trim={d} plate={} pname={}",
        plate.ok().map(|id| id.get()).unwrap_or(0),
        pname.as_ref().map(|n| n.as_str()).unwrap_or("-"),
    );

    // **这一趟的判据**（值那几格从门那边搬进来：门只剩"这一行还在不在"）。
    {
        assert_eq!(a, ocall::OK)
    }
    assert_eq!(b, ocall::OK);
    {
        assert_eq!(c, ocall::OK)
    }
    assert!(got);
    {
        assert_eq!(d, ocall::OK)
    }
    {
        assert_eq!(pname.as_ref().map(|n| n.as_str()), Some(ME))
    }

    d
}

/// 上树第二趟（**一串**）：列根 → 逐枚翻名 → 列 `/device` → 问一枚没铸过的号。
///
/// **名与号分开**那一刀的四格读数就落在这里：`list` 答号、`name` 按号答名（名字在答话那一侧，
/// 长短由那一帧说）。返这一趟的答码（`ocall::OK` = 全成）——每一格自己打一行，故中途断了也
/// 看得出断在哪一条。
pub fn serial(link: &Quay, talk: PieToken) -> u8 {
    // 根那一层：**`Where::Root` 就是根**（根没有号，故它占的是坐标那一格，不是一个号）。
    let Ok(root) = operator::list(talk, link, Where::Root, Wait::AtMost(MS)) else {
        return ocall::UNKNOWN;
    };
    debug!("echo: list root={}", ids_of(&root));

    // 逐枚翻名：`0` 是真的第一个格子（**根没有号**），故它也该翻得出名字。
    let mut names = String::new();
    let mut code = ocall::OK;
    for id in root.iter() {
        if !names.is_empty() {
            names.push(',');
        }
        match operator::name(talk, link, id, Wait::AtMost(MS)) {
            Ok(name) => names.push_str(name.as_str()),
            Err(_) => {
                names.push('?');
                code = ocall::UNKNOWN;
            }
        }
    }
    debug!("echo: list names={names}");

    // 第二块 Pane：`/device`（那一段名字本域已经知道——`user/echo/mod.rs` 头注里那两条来路之一）。
    let Ok(dir) = Name::new(protocol::driver::DIR) else {
        return ocall::UNKNOWN;
    };
    // **间接寻址那一手**：先把那一段名字译成号，再按号列（`list` 收的是容器坐标）。
    match operator::seek(talk, link, &[dir], Wait::AtMost(MS))
        .and_then(|at| operator::list(talk, link, Where::At(at), Wait::AtMost(MS)))
    {
        Ok(sub) => {
            debug!("echo: list device={}", ids_of(&sub));
        }
        Err(code) => {
            debug!("echo: list device=err:{code}");
            return ocall::UNKNOWN;
        }
    }

    // 一枚没铸过的号：**`UNKNOWN`，不是"答了一格空名字"**。
    let miss = operator::name(talk, link, EntryId::new(4095), Wait::AtMost(MS)).is_err();
    debug!("echo: name miss={miss}");

    // 一枚**本域自己选的**没铸过的号 ⇒ 该答不出（命名空间的契约，不是装配事实）。
    assert!(miss);

    if miss { code } else { ocall::BAD }
}

/// 一串号拼成 `0,3` 这样一段（读数用；一枚都没有拼成 `-`）。
fn ids_of(ids: &Listing) -> String {
    let mut out = String::new();
    for id in ids.iter() {
        if !out.is_empty() {
            out.push(',');
        }
        out.push_str(&format!("{}", id.get()));
    }
    if out.is_empty() {
        out.push('-');
    }
    out
}
