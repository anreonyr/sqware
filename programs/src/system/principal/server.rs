//! principal::server — **身份服务那一台**：一枚线程守着两张表（名册与谱系）。
//!
//! 载体是 rtc 那一面已经量过的形状——**门牌自带回信孔**：客人替这一趟铸一枚回信孔借过来、
//! 把帧推上门牌，本域从**门牌那一枚**读（发送者由内核在 `Push` 那一刻盖章），办完事从那枚孔
//! 答回去、当场放下。故这里**没有客人账**：一位客人不需要本域记住任何东西，"往哪回"那一格
//! 就在这一趟的孔上。
//!
//! ```text
//!   起手：读自己的 Sire（= 装配者，名册的钥匙）
//!         → 上板（板看得见本域的死）→ 铸门牌那一枚
//!           门牌两份：一份交给生我者（装配期用它 derive + bind，不必上树查自己）
//!                     一份经 LAND 落到树上 `/sys/principal`（别的客人按名字找上门）
//!   常驻：一只组等门牌那一枚 —— 读一帧（连发送者）→ 交给核心 → 从这一趟的回信孔答回去
//! ```

use env::Wait;
use alloc::format;

use env::{HoleDir, Name, PieToken, TaskId};
use protocol::system::operator::Where;
use protocol::system::operator::call as ocall;
use protocol::system::operator::client as operator;
use protocol::system::principal::call as pcall;
use protocol::system::principal::core::{Principal, PrincipalId};
use protocol::session::Quay;
use protocol::system::board::call as bcall;
use protocol::system::board::client as board;
use runtime::core::port::{self, Access, Policy};
use runtime::core::pile::Pile;
use runtime::env::mail::{self, HolePie};
use runtime::PAGE_SIZE;


/// 等板 / 等树的总上限（毫秒）。**必须有界**：对面死在头几步时本域不能陪着挂死。
const MS: usize = 1000;

/// 起服务：**读锚 → 上板 → 铸门牌（给生我者 + 上树）→ 一枚线程招待所有客人**。
pub fn serve() -> Result<(), super::fail::Fail> {
    // 一、锚：**生我者就是装配者**。名册只认这一枚——`Sire` 是内核盖的，比任何自报都硬；
    //    它还是弱引用，装配者一退这一格就答 0（那之后没人能写名册，也不该有）。
    // **起我那一枚线程**（不是 `sire()`：那一手答的是**域级**的生我者，对住本域的
    // 这一枚指的不是编排者。见 `service::Role::args` 的照实记）。
    let Some(assembler) = crate::service::assembler() else {
        return Err(super::fail::Fail::Sire);
    };

    // 二、上板：只为让板看得见本域的死（它常驻，编排域据此记账）。
    let Ok((_link, board_link)) = board::open(assembler, Wait::AtMost(MS)) else {
        return Err(super::fail::Fail::Board);
    };
    if board::ask_hole(board_link).is_err() {
        return Err(super::fail::Fail::Board);
    }

    // 三、门牌那一枚：本域自己开（`entry` 是服务入口的通用记号）。
    let Ok(entry) = mail::unseal_hole(bcall::ENTRY_MARK) else {
        return Err(super::fail::Fail::Tree);
    };
    // **先交给生我者**：装配期要靠它 derive + bind，而那条路不必先上树查自己。
    //
    // **照实记（这一格的理由换过一次，多出来的那一格也收了）**：原写的是"给 `FETCH` 是因为
    // **装配者还要把这一枚再转授给树**（门禁那一刀：树要问 `resolve` / `heir`）"——那一版真机
    // 栽在 `coord-ship`，现在**门牌由各域自己交**（见 `operator/bridge.rs` 的 `COORD` 段），
    // 那条理由已经不存在。装配者用这一枚只有**一条**路：往里**推帧**（`derive` / `bind`）；
    // 答话走每一趟自己铸的那枚回信孔（`session::call::lend_out` ＋ `push_to`：铸孔 → 交
    // `STORE` → 把"那一格"编进帧 → 推），
    // 读端在装配者这边。⇒ **`STORE` 就是这一格的全部需要**（孔上：`STORE` = `push`、
    // `FETCH` = `pull`，见 `env::permission` 的位表）；`FETCH` 是旧理由留下的，已收。
    if port::ship(
        &HolePie::from_token(entry),
        assembler,
        Access::STORE,
        Policy::NONE,
    )
    .is_err()
    {
        return Err(super::fail::Fail::Tree);
    }

    // 四、上树：分 `/sys`、落 `/sys/principal`、再查回来验一遍（同 router / rtc 那一趟）。
    let Ok((tree, host)) = operator::open(assembler, Wait::AtMost(MS)) else {
        return Err(super::fail::Fail::Tree);
    };
    let Ok(talk) = operator::ask_hole(host) else {
        return Err(super::fail::Fail::Tree);
    };
    serve_tree(&tree, talk, host, entry);

    // 四之后：**门禁那一枚**——把这一枚门牌**直接交给持树者**（`host` = 持树者的号，
    // `operator::open` 交回来的那一格）。它据此才判得了"这一位此刻代表谁"。
    //
    // 为什么是**本域自己**交、不是装配者转授：见 `programs/src/system/operator/bridge.rs`
    // 的 `COORD` 那段照实记——装配者转授那一版真机报 `operator:coord-ship`（内核 `-1`）。
    // 这一枚在手时权限是 `FETCH|STORE|VEST`，故子集 `FETCH|STORE` 不越界。
    if port::ship(
        &HolePie::from_token(entry),
        host,
        Access::FETCH | Access::STORE,
        Policy::NONE,
    )
    .is_err()
    {
        return Err(super::fail::Fail::Tree);
    }

    // 五、两张表：名册空着，谱系只有根（零号节点）。
    let Ok(mut book) = Principal::new(assembler) else {
        return Err(super::fail::Fail::Book);
    };

    // 六、常驻：**一只组等门牌那一枚**。这是常态，故等待没有期限。
    let Ok(pile) = Pile::unseal(false) else {
        return Err(super::fail::Fail::Desk);
    };
    let entry_hole = HolePie::from_token(entry);
    if pile.attach(&entry_hole, HoleDir::Pull).is_err() {
        return Err(super::fail::Fail::Desk);
    }

    // 一问的形状是那一形（25 字节）；缓冲给**一页**（载体的界，见 `Push` 的前置条件）——
    // 于是任何一条消息一趟都取得出来，"取不出也丢不掉"那个状态不存在。
    let mut buf: alloc::vec::Vec<u8> = alloc::vec::Vec::new();
    if buf.try_reserve_exact(PAGE_SIZE).is_err() {
        return Err(super::fail::Fail::Desk);
    }
    buf.resize(PAGE_SIZE, 0);
    loop {
        if pile.await_(Wait::Forever).is_err() {
            return Err(super::fail::Fail::Desk);
        }
        // 门牌是**单槽**：一次醒来的这一批要取干净（可能不止一位客人）。
        while let Ok((len, from)) = entry_hole.pull_timeout_from(&mut buf, Wait::POLL) {
            turn(&mut book, from, &buf[..len]);
        }
    }
}

/// 门上一句话：解帧 → 交给核心 → **从这一趟自带的那枚孔答回去**。
///
/// 认那枚孔靠**帧里那一格** ＋ **一次 [`mail::reserve`] 验**（用户裁定甲′）：那一格是
/// "客人借来的那枚回信孔**在我表里**是几号"，而"是谁给的、刻的什么"仍要当场读出来核对——
/// 否则客人能让本域往**别人的孔**里写。旧写法是扫本表按"谁给的 ＋ 记号"找（每趟请求一遍全表，
/// 见 `session::call::find` 与其 `Collect` 的价钱）。判据一字未改，只是从"扫遍全表找 match"
/// 变成"验这一格 match"。
///
/// `from` 是**内核盖的发送者**，名册与谱系的钥匙判据（装配者 / 当前正好代表 `p`）用的就是它。
fn turn(book: &mut Principal, from: TaskId, frame: &[u8]) {
    let Some((ask, back)) = pcall::Wire::take(frame) else {
        // 不是那个形状（长度不对）：不猜、不动账、也不回话——没有可信的"往哪回"。
        return;
    };
    if !matches!(
        mail::reserve(back),
        Ok((_vestor, owner, mark)) if owner == from && mark == pcall::BACK
    ) {
        // 这一趟没把回信孔交进来、或那一格指的是别人的孔：没有可回的路，账一动不动。
        return;
    }
    let _ = HolePie::from_token(back).push(&answer(book, from, ask));
    let _ = mail::release(back);
}

/// 把一句问交给核心，编出一句答（**一格**：读不懂也答，答 `BAD`）。
///
/// **形状由 [`pcall::Wire`] 说**（收帧那一侧已按动作解好了——两格载荷的意义随之定，不再是一枚
/// 裸码 ＋ 两个裸数）。答案与失败分开放（见 [`pcall`]：`OK` + `flag` 是答案，负码表只装失败）。
fn answer(book: &mut Principal, from: TaskId, ask: Option<pcall::Wire>) -> [u8; pcall::REPLY_LEN] {
    // 表外的动作码：这一问有回信的路，只是这一码我不认（与"读不懂"同一格）。
    let Some(ask) = ask else {
        return pcall::reply_status(pcall::BAD);
    };
    match ask {
        pcall::Wire::Bind(tid, p) => match book.bind(from, tid, p) {
            Ok(()) => pcall::reply_status(pcall::OK),
            Err(fail) => pcall::reply_status(pcall::fail_to_code(Some(fail))),
        },
        pcall::Wire::Resolve(tid) => match book.resolve(tid) {
            Some(p) => pcall::reply_present(true, p),
            None => pcall::reply_present(false, PrincipalId::ROOT),
        },
        pcall::Wire::Derive(p) => match book.derive(from, p) {
            Ok(q) => pcall::reply_value(q),
            Err(fail) => pcall::reply_status(pcall::fail_to_code(Some(fail))),
        },
        pcall::Wire::Sire(p) => match book.sire(p) {
            Ok(Some(q)) => pcall::reply_present(true, q),
            Ok(None) => pcall::reply_present(false, PrincipalId::ROOT),
            Err(fail) => pcall::reply_status(pcall::fail_to_code(Some(fail))),
        },
        pcall::Wire::Heir(a, b) => {
            match book.heir(a, b) {
                Ok(yes) => pcall::reply_yes(yes),
                // "不是祖先"是一句答（`Ok(false)`），"查无此号"才是这一格。
                Err(fail) => pcall::reply_status(pcall::fail_to_code(Some(fail))),
            }
        }
        // 转换那两条都只答状态那一格（成功 = `OK`）；钥匙是**发送者**，报文里没有"我是谁"。
        pcall::Wire::Adopt(p) => match book.adopt(from, p) {
            Ok(()) => pcall::reply_status(pcall::OK),
            Err(fail) => pcall::reply_status(pcall::fail_to_code(Some(fail))),
        },
        pcall::Wire::Waive => match book.waive(from) {
            Ok(()) => pcall::reply_status(pcall::OK),
            Err(fail) => pcall::reply_status(pcall::fail_to_code(Some(fail))),
        },
    }
}

/// 上树那一趟：**分目录 → 落门牌 → 查回来验一遍**（同 rtc 那一趟）。
///
/// `got` 只是"认出了那一枚"；它指不指得回原物，由**真客人**（`harness/src/subject.rs`）证——它照同一条路
/// 找上门、问一句、拿回一条号。故本域不自问自答。
fn serve_tree(link: &Quay, talk: PieToken, host: TaskId, entry: PieToken) {
    let (Ok(dir), Ok(me)) = (Name::new(pcall::DIR), Name::new(pcall::NAME)) else {
        say("principal: tree: bad name");
        return;
    };
    // **分目录 → 落门牌 → 查回来验一遍**：分与落各自**答出那一格的号**（"号出门"那一手）。
    // **分目录**：`part` 是**幂等**的——那块目录已经在就答它那个号（里面有没有东西不管）。
    let dir_at = operator::part(talk, link, Where::Root, dir, Wait::AtMost(MS));
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
            me,
            entry,
            ocall::Rule::Public,
            false,
            Wait::AtMost(MS),
        ),
        Err(code) => Err(code),
    };
    let (land, pid) = match plate {
        Ok(id) => (ocall::OK, id.get()),
        Err(code) => (code, 0),
    };
    // 查回来验一遍：**按号**（名字只在上面那两格用过，此后一律按号）。
    let (find, got) = match plate {
        Ok(id) => match operator::find(talk, link, id, Wait::AtMost(MS)) {
            Ok((code, entry)) => (code, entry.is_some()),
            Err(_) => (ocall::BAD, false),
        },
        Err(code) => (code, false),
    };
    // **`got` 换了来路**（乙′）：见 `ocall::Rep::Seed` 的照实记。
    // 拿号问名：**号 ↔ 名**这一对对得起来，才算那枚号是真坐标。
    let pname = plate
        .ok()
        .and_then(|id| operator::name(talk, link, id, Wait::AtMost(MS)).ok());
    say(&format!(
        "principal: tree part={part} dir={dir_id} land={land} find={find} got={got} entry={} plate={pid} pname={}",
        entry.get(),
        pname.as_ref().map(|n| n.as_str()).unwrap_or("-"),
    ));
}

/// 打一行。调试面是"服务还没起来的嘴"：本域没有控制台，只有它。
fn say(msg: &str) {
    let _ = runtime::env::debug::put(msg);
}
