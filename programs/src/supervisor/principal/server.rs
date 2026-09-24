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

use alloc::format;

use env::{HoleDir, Name, PieToken, TaskId};
use protocol::operator::Where;
use protocol::operator::call as ocall;
use protocol::operator::client as operator;
use protocol::principal::call as pcall;
use protocol::principal::core::{Principal, PrincipalId};
use protocol::session::Quay;
use protocol::session::call as scall;
use protocol::system::board::call as bcall;
use protocol::system::board::client as board;
use runtime::core::port::{self, Access, Policy};
use runtime::core::tole::Tole;
use runtime::env::mail::{self, HolePie};
use runtime::env::unit as utask;


/// 等板 / 等树的总上限（毫秒）。**必须有界**：对面死在头几步时本域不能陪着挂死。
const MS: usize = 1000;

/// 起服务：**读锚 → 上板 → 铸门牌（给生我者 + 上树）→ 一枚线程招待所有客人**。
pub fn serve() -> Result<(), super::fail::Fail> {
    // 一、锚：**生我者就是装配者**。名册只认这一枚——`Sire` 是内核盖的，比任何自报都硬；
    //    它还是弱引用，装配者一退这一格就答 0（那之后没人能写名册，也不该有）。
    let Ok(assembler) = utask::sire() else {
        return Err(super::fail::Fail::Sire);
    };

    // 二、上板：只为让板看得见本域的死（它常驻，编排域据此记账）。
    let Ok((_link, board_link)) = board::open(assembler, MS) else {
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
    // 答话走每一趟自己铸的那枚回信孔（`session::call::lend`：铸孔 → 交 `STORE` → 推帧），
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
    let Ok((tree, host)) = operator::open(assembler, MS) else {
        return Err(super::fail::Fail::Tree);
    };
    let Ok(talk) = operator::ask_hole(host) else {
        return Err(super::fail::Fail::Tree);
    };
    serve_tree(&tree, talk, host, entry);

    // 四之后：**门禁那一枚**——把这一枚门牌**直接交给持树者**（`host` = 持树者的号，
    // `operator::open` 交回来的那一格）。它据此才判得了"这一位此刻代表谁"。
    //
    // 为什么是**本域自己**交、不是装配者转授：见 `programs/src/supervisor/operator/bridge.rs`
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
    let Ok(tole) = Tole::unseal(false) else {
        return Err(super::fail::Fail::Desk);
    };
    let entry_hole = HolePie::from_token(entry);
    if tole.attach(&entry_hole, HoleDir::Pull).is_err() {
        return Err(super::fail::Fail::Desk);
    }

    // 一问的上界就是 `ASK_LEN`（`pack_ask` 产出的就是这个长度）。
    let mut buf = [0u8; pcall::ASK_LEN];
    loop {
        if tole.await_(usize::MAX).is_err() {
            return Err(super::fail::Fail::Desk);
        }
        // 门牌是**单槽**：一次醒来的这一批要取干净（可能不止一位客人）。
        // 取帧走 [`scall::receive`]——它带**入站闸**：比 `ASK_LEN` 长的那一枚取出来丢掉
        // （不然这一扇门取不出也丢不掉，从此卡死）。
        loop {
            match scall::receive(&entry_hole, &mut buf, 0) {
                scall::Arrival::Ask(len, from) => turn(&mut book, from, &buf[..len]),
                scall::Arrival::Junk(n) => {
                    say(&alloc::format!("principal: ask too long n={n}"));
                }
                // 连丢它那块缓冲都备不下 ⇒ 这一扇门排不空了：报一句**然后去死**（板看得见）。
                scall::Arrival::Stuck(n) => {
                    say(&alloc::format!("principal: ask stuck n={n}"));
                    return Err(super::fail::Fail::Desk);
                }
                scall::Arrival::Idle => break,
            }
        }
    }
}

/// 门上一句话：解帧 → 交给核心 → **从这一趟自带的那枚孔答回去**。
///
/// 认那枚孔靠 [`scall::find`] 的两格正判据（谁给的 + 记号）；`from` 是**内核盖的发送者**，
/// 名册与谱系的钥匙判据（装配者 / 当前正好代表 `p`）用的就是它。
fn turn(book: &mut Principal, from: TaskId, frame: &[u8]) {
    let Some((op, a, b)) = pcall::unpack_ask(frame) else {
        // 不是那个形状：不猜、不动账、也不回话——没有可信的"往哪回"。
        return;
    };
    let Some(back) = scall::find(from, pcall::BACK) else {
        // 这一趟没把回信孔交进来（或交得不成）：没有可回的路，账一动不动。
        return;
    };
    let _ = HolePie::from_token(back).push(&answer(book, from, op, a, b));
    let _ = mail::release(back);
}

/// 把一句问交给核心，编出一句答（**一格**：读不懂也答，答 `BAD`）。
///
/// **先读动作码、再解载荷**；动作码定两格载荷的意义（`RESOLVE`/`DERIVE`/`SIRE` 只用 `a`，
/// `HEIR` 两格都用）。答案与失败分开放（见 [`pcall`]：`OK` + `flag` 是答案，负码表只装失败）。
fn answer(book: &mut Principal, from: TaskId, op: u8, a: u64, b: u64) -> [u8; pcall::REPLY_LEN] {
    match op {
        pcall::BIND => match book.bind(from, TaskId::new(a as usize), PrincipalId::new(b as usize))
        {
            Ok(()) => pcall::reply_status(pcall::OK),
            Err(fail) => pcall::reply_status(pcall::fail_to_code(Some(fail))),
        },
        pcall::RESOLVE => match book.resolve(TaskId::new(a as usize)) {
            Some(p) => pcall::reply_present(true, p),
            None => pcall::reply_present(false, PrincipalId::ROOT),
        },
        pcall::DERIVE => match book.derive(from, PrincipalId::new(a as usize)) {
            Ok(q) => pcall::reply_value(q),
            Err(fail) => pcall::reply_status(pcall::fail_to_code(Some(fail))),
        },
        pcall::SIRE => match book.sire(PrincipalId::new(a as usize)) {
            Ok(Some(q)) => pcall::reply_present(true, q),
            Ok(None) => pcall::reply_present(false, PrincipalId::ROOT),
            Err(fail) => pcall::reply_status(pcall::fail_to_code(Some(fail))),
        },
        pcall::HEIR => {
            match book.heir(PrincipalId::new(a as usize), PrincipalId::new(b as usize)) {
                Ok(yes) => pcall::reply_yes(yes),
                // "不是祖先"是一句答（`Ok(false)`），"查无此号"才是这一格。
                Err(fail) => pcall::reply_status(pcall::fail_to_code(Some(fail))),
            }
        }
        // 转换那两条都只答状态那一格（成功 = `OK`）；钥匙是**发送者**，报文里没有"我是谁"。
        pcall::ADOPT => match book.adopt(from, PrincipalId::new(a as usize)) {
            Ok(()) => pcall::reply_status(pcall::OK),
            Err(fail) => pcall::reply_status(pcall::fail_to_code(Some(fail))),
        },
        pcall::WAIVE => match book.waive(from) {
            Ok(()) => pcall::reply_status(pcall::OK),
            Err(fail) => pcall::reply_status(pcall::fail_to_code(Some(fail))),
        },
        // 没见过的动作码：与"这一问读不懂"同一格（不另立一格）。
        _ => pcall::reply_status(pcall::BAD),
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
    let dir_at = operator::part(talk, link, Where::Root, dir, MS);
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
            MS,
        ),
        Err(code) => Err(code),
    };
    let (land, pid) = match plate {
        Ok(id) => (ocall::OK, id.get()),
        Err(code) => (code, 0),
    };
    // 查回来验一遍：**按号**（名字只在上面那两格用过，此后一律按号）。
    let find = match plate {
        Ok(id) => operator::find(talk, link, id, MS).unwrap_or(ocall::BAD),
        Err(code) => code,
    };
    let got = operator::take(link, host).is_some();
    // 拿号问名：**号 ↔ 名**这一对对得起来，才算那枚号是真坐标。
    let pname = plate
        .ok()
        .and_then(|id| operator::name(talk, link, id, MS).ok());
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
