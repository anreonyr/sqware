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

use crate::system::server::Start;
use env::Wait;

use env::{HoleDir, Name, PieToken, TaskId};
use protocol::debug;
use protocol::session::Quay;
use protocol::session::slip::Slip;
use protocol::system::board as bcall;
use protocol::system::board::client as board;
use protocol::system::operator as ocall;
use protocol::system::operator::Where;
use protocol::system::operator::client as operator;
use protocol::system::principal as pcall;
use protocol::system::principal::core::{Principal, PrincipalId};
use runtime::PAGE_SIZE;
use runtime::core::pile::Pile;
use runtime::core::port::{self, Access, Policy};
use runtime::env::mail::{self, HolePie};

/// 等板 / 等树的总上限（毫秒）。**必须有界**：对面死在头几步时本域不能陪着挂死。
const MS: usize = 1000;

/// 起服务：**读锚 → 上板 → 铸门牌（给生我者 + 上树）→ 一枚线程招待所有客人**。
///
/// **起手那几步收在一个闭包**（与持树者 / 盟册那两台同形）：它们清一色是"不成 ⇒ 这域
/// 起不来"的早退步，从前每步一段 `let Ok(..) = .. else { return Err(..) }`——报的是同一个死法、
/// 写的是七段岔口，主脉络因此被岔口切碎。收进闭包之后全走 `?`、失败域在末尾**折一次**。
/// `serve` 的主干于是只剩两步：**起手 → 常驻**。
///
/// 起手要交出去的四样：常驻那一问的两格（`pile` 是那一组、`entry_hole` 是组下那枚门牌）、
/// `book`（`turn` 收它）、`buf`（收帧那一页，循环里也用）。
pub fn serve() -> Result<(), Start> {
    // 一～六：起手（读锚 → 上板 → 铸门牌 → 上树 → 两张表 → 常驻那只组）。
    let (mut book, pile, entry_hole, mut buf) = (|| {
        // 一、锚：**生我者就是装配者**。名册只认这一枚——`Sire` 是内核盖的，比任何自报都硬；
        //    它还是弱引用，装配者一退这一格就答 0（那之后没人能写名册，也不该有）。
        // **起我那一枚线程**（不是 `sire()`：那一手答的是**域级**的生我者，对住本域的
        // 这一枚指的不是编排者）。
        let assembler = crate::service::assembler().ok_or(Start::Sire)?;

        // 二、上板：只为让板看得见本域的死（它常驻，编排域据此记账）。
        let (_link, board_link) =
            board::open(assembler, Wait::AtMost(MS)).map_err(|_| Start::Board)?;
        board::ask_hole(board_link).map_err(|_| Start::Board)?;

        // 三、门牌那一枚：本域自己开（`entry` 是服务入口的通用记号）。
        let entry = mail::unseal_hole(bcall::ENTRY_MARK).map_err(|_| Start::Tree)?;
        // **先交给生我者**：装配期要靠它 derive + bind，而那条路不必先上树查自己。
        //
        // **本域自己交、不是装配者转授**：门牌由各域自己交（见 `operator/bridge.rs` 的
        // `COORD` 段）。装配者用这一枚只有**一条**路：往里**推帧**（`derive` / `bind`）；
        // 答话走每一趟自己铸的那枚回信孔（`session::call::lend_out` ＋ `push_to`：铸孔 → 交
        // `STORE` → 把"那一格"编进帧 → 推），读端在装配者这边。⇒ **`STORE` 就是这一格的全部需要**。
        port::ship(
            &HolePie::from_token(entry),
            assembler,
            Access::STORE,
            Policy::NONE,
        )
        .map_err(|_| Start::Tree)?;

        // 四、上树：分 `/sys`、落 `/sys/principal`、再查回来验一遍（同 router / rtc 那一趟）。
        let (tree, host) = operator::open(assembler, Wait::AtMost(MS)).map_err(|_| Start::Tree)?;
        let talk = operator::ask_hole(host).map_err(|_| Start::Tree)?;
        serve_tree(&tree, talk, host, entry);

        // 四之后：**门禁那一枚**——把这一枚门牌**直接交给持树者**（`host` = 持树者的号，
        // `operator::open` 交回来的那一格）。它据此才判得了"这一位此刻代表谁"。
        //
        // 这一枚在手时权限是 `FETCH|STORE|VEST`，故子集 `FETCH|STORE` 不越界。
        port::ship(
            &HolePie::from_token(entry),
            host,
            Access::FETCH | Access::STORE,
            Policy::NONE,
        )
        .map_err(|_| Start::Tree)?;

        // 五、两张表：名册空着，谱系只有根（零号节点）。
        let book = Principal::new(assembler).map_err(|_| Start::Book)?;

        // 六、常驻：**一只组等门牌那一枚**。这是常态，故等待没有期限；那一页缓冲只备一次。
        let pile = Pile::unseal(false).map_err(|_| Start::Desk)?;
        let entry_hole = HolePie::from_token(entry);
        pile.attach(&entry_hole, HoleDir::Pull)
            .map_err(|_| Start::Desk)?;
        let mut buf: alloc::vec::Vec<u8> = alloc::vec::Vec::new();
        if buf.try_reserve_exact(PAGE_SIZE).is_err() {
            return Err(Start::Room);
        }
        buf.resize(PAGE_SIZE, 0);
        Ok::<_, Start>((book, pile, entry_hole, buf))
    })()?;

    loop {
        if pile.await_(Wait::Forever).is_err() {
            return Err(Start::Dead);
        }
        // 门牌是**单槽**：一次醒来的这一批要取干净（可能不止一位客人）。
        while let Ok((len, from)) = entry_hole.pull_timeout_from(&mut buf, Wait::POLL) {
            turn(&mut book, from, &buf[..len]);
        }
    }
}

/// 门上一句话：解帧 → 交给核心 → **从这一趟自带的那枚孔答回去**。
///
/// 认那枚孔靠**帧里那一格** ＋ **一次 [`mail::reserve`] 验**：那一格是
/// "客人借来的那枚回信孔**在我表里**是几号"，而"是谁给的、刻的什么"仍要当场读出来核对——
/// 否则客人能让本域往**别人的孔**里写。
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
    // 答一句：**一格**（[`pcall::Reply`] 那一形）——走这一趟那枚回信孔，装与发都不在这一层
    // 写字节（缓冲是船台自己那只：这一形定长 10）。
    // `.ok()`：装不上那一格按构造到不了（`Buf` 由本族 `Message` 自己给，见 `Slip::load`）；
    // 真到了那里，这一答就发不出去。
    let _ = Slip::<pcall::Reply>::seal(back)
        .load(answer(book, from, ask))
        .ok()
        .map(|s| s.ship());
    let _ = mail::release(back);
}

/// 把一句问交给核心，编出一句答（**一格**：读不懂也答，答 `BAD`）。
///
/// **形状由 [`pcall::Wire`] 说**（收帧那一侧已按动作解好了——两格载荷的意义随之定，不再是一枚
/// 裸码 ＋ 两个裸数）。答案与失败分开放（见 [`pcall`]：`OK` + `flag` 是答案，负码表只装失败）。
fn answer(book: &mut Principal, from: TaskId, ask: Option<pcall::Wire>) -> pcall::Reply {
    // 表外的动作码：这一问有回信的路，只是这一码我不认（与"读不懂"同一格）。
    let Some(ask) = ask else {
        return pcall::Reply::status(pcall::BAD);
    };
    match ask {
        pcall::Wire::Bind(tid, p) => match book.bind(from, tid, p) {
            Ok(()) => pcall::Reply::status(pcall::OK),
            Err(fail) => pcall::Reply::status(pcall::fail_to_code(Some(fail))),
        },
        pcall::Wire::Resolve(tid) => match book.resolve(tid) {
            Some(p) => pcall::reply_present(true, p),
            None => pcall::reply_present(false, PrincipalId::ROOT),
        },
        pcall::Wire::Derive(p) => match book.derive(from, p) {
            Ok(q) => pcall::Reply::value(q),
            Err(fail) => pcall::Reply::status(pcall::fail_to_code(Some(fail))),
        },
        pcall::Wire::Sire(p) => match book.sire(p) {
            Ok(Some(q)) => pcall::reply_present(true, q),
            Ok(None) => pcall::reply_present(false, PrincipalId::ROOT),
            Err(fail) => pcall::Reply::status(pcall::fail_to_code(Some(fail))),
        },
        pcall::Wire::Heir(a, b) => {
            match book.heir(a, b) {
                Ok(yes) => pcall::Reply::yes(yes),
                // "不是祖先"是一句答（`Ok(false)`），"查无此号"才是这一格。
                Err(fail) => pcall::Reply::status(pcall::fail_to_code(Some(fail))),
            }
        }
        // 转换那两条都只答状态那一格（成功 = `OK`）；钥匙是**发送者**，报文里没有"我是谁"。
        pcall::Wire::Adopt(p) => match book.adopt(from, p) {
            Ok(()) => pcall::Reply::status(pcall::OK),
            Err(fail) => pcall::Reply::status(pcall::fail_to_code(Some(fail))),
        },
        pcall::Wire::Waive => match book.waive(from) {
            Ok(()) => pcall::Reply::status(pcall::OK),
            Err(fail) => pcall::Reply::status(pcall::fail_to_code(Some(fail))),
        },
    }
}

/// 上树那一趟：**分目录 → 落门牌 → 查回来验一遍**（同 rtc 那一趟）。
///
/// `got` 只是"认出了那一枚"；它指不指得回原物，由**真客人**（`harness/src/subject.rs`）证——它照同一条路
/// 找上门、问一句、拿回一条号。故本域不自问自答。
fn serve_tree(link: &Quay, talk: PieToken, host: TaskId, entry: PieToken) {
    let (Ok(dir), Ok(me)) = (Name::new(pcall::DIR), Name::new(pcall::NAME)) else {
        debug!("principal: tree: bad name");
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
    // 拿号问名：**号 ↔ 名**这一对对得起来，才算那枚号是真坐标。
    let pname = plate
        .ok()
        .and_then(|id| operator::name(talk, link, id, Wait::AtMost(MS)).ok());
    debug!(
        "principal: tree part={part} dir={dir_id} land={land} find={find} got={got} entry={} plate={pid} pname={}",
        entry.get(),
        pname.as_ref().map(|n| n.as_str()).unwrap_or("-"),
    );
}

