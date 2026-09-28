//! principal::server — **身份服务那一台**：一枚线程守着两张表（名册与谱系）。
//!
//! 载体是 rtc 那一面已经量过的形状——**门牌自带回信孔**：客人替这一趟铸一枚回信孔借过来、
//! 把帧推上门牌，本域从**门牌那一枚**读（发送者由内核在 `Push` 那一刻盖章），办完事从那枚孔
//! 答回去、当场放下。故这里**没有客人账**：一位客人不需要本域记住任何东西，"往哪回"那一格
//! 就在这一趟的孔上。
//!
//! ```text
//!   起手：读自己的 Sire（= 装配者，名册的钥匙）
//!         → 上板（板看得见本域的死）→ 铸门牌**两枚**（两面各一枚）
//!           定面（Set）一份交给生我者（装配期用它 derive + bind，不必上树查自己）
//!           问面（Ask）一份直接交给持树者（门禁只问"这一位代表谁"）
//!           两枚都经 LAND 落到树上 `/sys/principal/{ask,set}`（别的客人按名字找上门）
//!   常驻：一只组等那两枚 —— **从哪一枚读到**就是哪一面 → 交给核心 → 从这一趟的回信孔答回去
//! ```
//!
//! **面为什么长在门牌上**（开面那一刀，与 operator 不同的一格）：本族**没有会话**——门牌自己
//! 就是那条路，所有人往同一枚孔推帧，故服务端原先**分不出面**。开面之后是**两枚门牌、两只孔**：
//! 从哪一枚读到就是哪一面，而**那一问属不属于这一面**由 [`Grant::of_wire`] 当场对一次
//! （对不上答 [`pcall::DENIED`]）。理由与持有者那三行见 [`pcall::grant`] 的文件头。

use crate::system::control::service::Start;
use env::Wait;

use env::{HoleDir, Name, PieToken, TaskId};
use protocol::debug;
use protocol::communication::sender::Sender;
use protocol::communication::session::Session;
use crate::system::board::client as board;
use protocol::system::operator::client as operator;
use protocol::system::operator::client::Mine;
use protocol::system::operator::Permit;
use protocol::system::principal as pcall;
use crate::system::principal::core::Principal;
use crate::system::principal::mount;
use protocol::system::principal::PrincipalId;
use runtime::PAGE_SIZE;
use runtime::core::pile::Pile;
use runtime::core::port::{self, Access, Policy};
use runtime::env::mail::{self, HolePie};

/// 等板 / 等树的总上限（毫秒）。**必须有界**：对面死在头几步时本域不能陪着挂死。
const MS: usize = 1000;

/// 起服务：**读锚 → 上板 → 铸门牌两枚 → 一枚线程招待所有客人**。
///
/// **起手那几步收在一个闭包**（与持树者 / 盟册那两台同形）：它们清一色是"不成 ⇒ 这域
/// 起不来"的早退步，从前每步一段 `let Ok(..) = .. else { return Err(..) }`——报的是同一个死法、
/// 写的是七段岔口，主脉络因此被岔口切碎。收进闭包之后全走 `?`、失败域在末尾**折一次**。
/// `serve` 的主干于是只剩两步：**起手 → 常驻**。
///
/// 起手要交出去的五样：常驻那一问的三格（`pile` 是那一组、`ask` / `set` 是组下那两枚门牌的号
/// ——**从哪一枚读到就是哪一面**）、`book`（`turn` 收它）、`buf`（收帧那一页，循环里也用）。
pub fn serve() -> Result<(), Start> {
    // 一～六：起手（读锚 → 上板 → 铸两枚门牌 → 上树 → 两张表 → 常驻那只组）。
    let (mut book, pile, ask, set, mut buf) = (|| {
        // 一、锚：**生我者就是装配者**。名册只认这一枚——`Sire` 是内核盖的，比任何自报都硬；
        //    它还是弱引用，装配者一退这一格就答 0（那之后没人能写名册，也不该有）。
        // **起我那一枚线程**：本域是装配者建的，故 `Sire` 答的就是它——只有这一条来源。
        let assembler = runtime::env::unit::sire();

        // 二、上板：只为让板看得见本域的死（它常驻，编排域据此记账）。
        let _board = Session::open(assembler, board::BERTH, Wait::AtMost(MS))
            .map_err(|_| Start::Board)?;

        // 三、门牌**两枚**：**一面一枚**（[`mount::entry`] 按面给记号与末段名）。
        //
        // **两面各一枚、不是一枚两面**：本族没有会话，服务端只能从**它自己表里哪一枚孔**收到来
        // 认面（见文件头）。两枚都长命——铸它们的是本线程自己（同 `operator::mount` 那条照实记）。
        let (ask, ask_name) = mount::entry(pcall::Grant::Ask).map_err(|_| Start::Tree)?;
        let (set, set_name) = mount::entry(pcall::Grant::Set).map_err(|_| Start::Tree)?;

        // **定面先交给生我者**：装配期要靠它 derive + bind，而那条路不必先上树查自己。
        //
        // **本域自己交、不是装配者转授**：门牌由各域自己交（见 `operator/bridge.rs` 的
        // `COORD` 段）。装配者用这一枚只有**一条**路：往里**推帧**（`derive` / `bind`）；
        // 答话走每一趟自己铸的那枚回信孔（`communication::establish::lend_out` ＋ `HolePie::push`：铸孔 → 交
        // `STORE` → 把"那一格"编进帧 → 推），读端在装配者这边。⇒ **`STORE` 就是这一格的全部需要**。
        //
        // **它拿不到问面**（照实记：从前这一份就是唯一那一枚，两面都在里面）：装配期不需要读身份
        // ——`derive` / `bind` 两条都在定面上。
        port::ship(
            &HolePie::from_token(set),
            assembler,
            Access::STORE,
            Policy::NONE,
        )
        .map_err(|_| Start::Tree)?;

        // 四、上树：分 `/sys`、分 `/sys/principal`、落那两格，再**逐面**查回来验一遍。
        let session = Session::open(assembler, operator::BERTH, Wait::AtMost(MS))
            .map_err(|_| Start::Tree)?;
        serve_tree(
            &session,
            [
                (pcall::Grant::Ask, ask, ask_name),
                (pcall::Grant::Set, set, set_name),
            ],
        );

        // 四之后：**问面那一枚交给持树者**（`host` = 持树者的号，`Session::open` 收下的那一格）。
        // 它据此才判得了"这一位此刻代表谁"；而它**做不出** `Adopt`——那条在定面上。
        //
        // 这一枚在手时权限是 `FETCH|STORE|VEST`，故子集 `FETCH|STORE` 不越界。
        port::ship(
            &HolePie::from_token(ask),
            session.host,
            Access::FETCH | Access::STORE,
            Policy::NONE,
        )
        .map_err(|_| Start::Tree)?;

        // 五、两张表：名册空着，谱系只有根（零号节点）。
        let book = Principal::new(assembler).map_err(|_| Start::Book)?;

        // 六、常驻：**一只组等那两枚门牌**。这是常态，故等待没有期限；那一页缓冲只备一次。
        let pile = Pile::unseal(false).map_err(|_| Start::Desk)?;
        let ask_hole = HolePie::from_token(ask);
        let set_hole = HolePie::from_token(set);
        for hole in [&ask_hole, &set_hole] {
            pile.attach(hole, HoleDir::Pull).map_err(|_| Start::Desk)?;
        }
        let mut buf: alloc::vec::Vec<u8> = alloc::vec::Vec::new();
        if buf.try_reserve_exact(PAGE_SIZE).is_err() {
            return Err(Start::Room);
        }
        buf.resize(PAGE_SIZE, 0);
        Ok::<_, Start>((book, pile, ask, set, buf))
    })()?;

    loop {
        // **`Ok(None)` 不是终局**（[`Pile::await_`] 自己的照实记：挂起过、或期限到，都会给
        // `None`——继续等就再叫一次）；只有 `Err` 才是这一组死了。
        //
        // **照实记（这一格栽过）**：开面这一刀把它写成了 `let Ok(Some(hit)) = … else { 死 }`
        // ——那是从持树者那一处抄来的形状，而那一处 `else` 里是 `continue`。实机读数：名册
        // 答完**第一帧**（装配者的 `derive`）就当"组死了"退场，整机装配随之塌
        // （`exit tid=5 note: inner: group dead`）。`None` 与 `Err` 是两件事，那一处折叠
        // 把"这一轮没事"读成了"这一组死了"。
        let (tok, _dir) = match pile.await_(Wait::Forever) {
            Ok(Some(hit)) => hit,
            Ok(None) => continue,
            Err(_) => return Err(Start::Dead),
        };
        // **从哪一枚读到就是哪一面**——本族的"面"就是它（没有会话可读记号）。
        //
        // 余下的号（构造上到不了：组里只挂了这两枚）⇒ 不猜，回去再等。
        let face = if tok == ask {
            pcall::Grant::Ask
        } else if tok == set {
            pcall::Grant::Set
        } else {
            continue;
        };
        // 门牌是**单槽**：一次醒来的这一批要取干净（可能不止一位客人）。
        let hole = HolePie::from_token(tok);
        while let Ok((len, from)) = hole.pull_timeout_from(&mut buf, Wait::POLL) {
            turn(&mut book, from, face, &buf[..len]);
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
/// `face` 是**这一帧从哪一枚门牌进来的**（[`serve`] 那只组说的事实）。
fn turn(book: &mut Principal, from: TaskId, face: pcall::Grant, frame: &[u8]) {
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
    // 写字节（缓冲在这一帧的栈上：这一形定长 10）。
    // `.ok()`：装不上那一格按构造到不了（`Buf` 由本族 `Message` 自己给，见 `Sender::send`）；
    // 真到了那里，这一答就发不出去。
    let _ = Sender::<pcall::Reply>::from_token(back)
        .send(answer(book, from, ask, face), Wait::Forever)
        .ok();
    let _ = mail::release(back);
}

/// 把一句问交给核心，编出一句答（**一格**：读不懂也答，答 `BAD`）。
///
/// **形状由 [`pcall::Wire`] 说**（收帧那一侧已按动作解好了——两格载荷的意义随之定，不再是一枚
/// 裸码 ＋ 两个裸数）。答案与失败分开放（见 [`pcall`]：`OK` + `flag` 是答案，负码表只装失败）。
///
/// **面这一道在交给核心之前**（[`Grant::of_wire`] 对那一条线上一码）：这一帧是从 Face 那枚门牌
/// 进来的，而它问的那件事属不属于那一面，只有这一句说得清。对不上答 [`pcall::DENIED`]
/// ——**终态**（换一枚门牌 / 别重试），与 kernel 那几格失败分开。
///
/// **照实记（这一道在 core 之外）**：核心那三条 `Fail`（`Denied` / `Unknown` / `Full`）讲的都是
/// **名册与谱系本身**的事（你是谁、那条号在不在、备不备得下）；"你手里那一枚门牌给不给这一条"
/// 是**载体**的事——故它在这一层，且它先于核心（连账都不必看一眼）。
fn answer(
    book: &mut Principal,
    from: TaskId,
    ask: Option<pcall::Wire>,
    face: pcall::Grant,
) -> pcall::Reply {
    // 表外的动作码：这一问有回信的路，只是这一码我不认（与"读不懂"同一格）。
    let Some(ask) = ask else {
        return pcall::Reply::status(pcall::BAD);
    };
    if pcall::Grant::of_wire(&ask) != face.at() {
        // 这一条不在它手里那一面上：不开那一面的门牌做不出这件事（照实记：从前两面是一面，
        // 持树者手里那枚门牌因此**做得出** `Adopt`）。
        //
        // **这一句读数不是装饰**：核那一条拒（"你不是写名册的那一枚"）也答 `Fail::Denied`
        // ——两个因同码（客人的下一步一样），分得开它们的只有这一行。
        debug!(
            "principal: face={} asked={} denied",
            face.name(),
            pcall::Grant::of_wire(&ask)
        );
        return pcall::Reply::status(pcall::DENIED);
    }
    match ask {
        // 照实记（这一格有两种因，同码）：发送者不是装配者、或那一条号不在树里——两者都答
        // `Fail::Denied` / `Unknown`，分开它们的是持册者那一侧要看得见的读数。
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

/// 上树那一趟：**分目录 → 逐面落门牌 → 逐面查回来验一遍**（同 rtc 那一趟）。
///
/// **目录那两段各自只走一次**（`/sys` 与 `/sys/principal`）：两面共用它们，而 `open` 是**幂等**
/// 的——那块窗格已经在就答它那个号（里面有没有东西不管）。**`/sys/principal` 自己不是一格**：
/// 它是第一条路的前缀走出来的那块 `Pane`（同 `/sys/operator` 那一格，见 [`mount`] 的照实记）。
///
/// **两面各自报一行读数**："哪一面没挂上"要看得见，且一面挂不上不拦另一面（同
/// [`Assembly::mount_grants`](crate::system::Assembly::mount_grants) 那条立场）。
///
/// `got` 只是"认出了那一枚"；它指不指得回原物，由**真客人**（`harness/src/subject.rs`）证——它照同一条路
/// 找上门、问一句、拿回一条号。故本域不自问自答。
fn serve_tree(session: &Session, faces: [(pcall::Grant, PieToken, Name); 2]) {
    let tree = operator::Face::from(session);
    let (Ok(dir), Ok(segment)) = (Name::new(pcall::DIR), Name::new(mount::SEGMENT)) else {
        debug!("principal: tree: bad name");
        return;
    };
    let root = tree.root();
    // **分目录**：`/sys`（两族共用那一格坐标）。
    let Ok(sys) = root.open(dir, Wait::AtMost(MS)) else {
        debug!("principal: tree part /sys failed");
        return;
    };
    // **再分一段**：`/sys/principal`——两面共用那段前缀，**它自己不落任何叶子**。
    let Ok(seg) = sys.open(segment, Wait::AtMost(MS)) else {
        debug!("principal: tree part /sys/principal failed");
        return;
    };
    for (grant, entry, name) in faces {
        // **落门牌**：答的是门牌自己那一格的号。
        let landed = seg
            .bind(name, entry, Permit::Unset, Mine::No, Wait::AtMost(MS))
            .map(|plate| plate.id());
        let (land, pid) = match &landed {
            Ok(id) => (Ok(()), id.get()),
            Err(fail) => (Err(*fail), 0),
        };
        // 查回来验一遍：**按号**（名字只在上面那两格用过，此后一律按号）。
        let (find, got) = match &landed {
            Ok(_) => match tree.tile(&[dir, segment, name], Wait::AtMost(MS)) {
                Ok(e) => match e.token(Wait::AtMost(MS)) {
                    Ok(_) => (Ok(()), true),
                    Err(fail) => (Err(fail), false),
                },
                Err(fail) => (Err(fail), false),
            },
            Err(fail) => (Err(*fail), false),
        };
        // 拿号问名：**号 ↔ 名**这一对对得起来，才算那枚号是真坐标。
        let pname = match landed {
            Ok(id) => root.name(id, Wait::AtMost(MS)).ok(),
            Err(_) => None,
        };
        debug!(
            "principal: tree face={} land={land:?} find={find:?} got={got} entry={} plate={pid} pname={}",
            grant.name(),
            entry.get(),
            pname.as_ref().map(|n| n.as_str()).unwrap_or("-"),
        );
    }
}

