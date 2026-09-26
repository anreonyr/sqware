//! coalition::server — **结盟服务那一台**：一枚线程守着盟册（一条关系 + 一枚计数器）。
//!
//! 载体是 rtc / principal 那一面已经量过的形状——**门牌自带回信孔**：客人替这一趟铸一枚回信
//! 孔借过来、把帧推上门牌，本域从**门牌那一枚**读（发送者由内核在 `Push` 那一刻盖章），办完事
//! 从那枚孔答回去、当场放下。故这里**没有客人账**：一位客人不需要本域记住任何东西。
//!
//! ```text
//!   起手：读自己的 Sire（**只为上板与上树两条会话**——盟无主，之后不落任何字段）
//!         → 上板（板看得见本域的死）→ 铸门牌那一枚
//!           经 LAND 落到树上 `/sys/coalition`，再 FIND 回来验一遍
//!         → FIND `/sys/principal`（**带重试**）拿一份身份服务的门牌
//!   常驻：一只组等门牌那一枚 —— 读一帧（连发送者）→ 先过名册问"你是谁" → 交给核心 → 答回去
//! ```
//!
//! **两条锚为什么都在这儿**：`Sire` 是内核盖的（比任何自报都硬，且是弱引用：装配者一退它
//! 就答 0），树那条路是"按名找人"的现成一步；而身份那一份门牌**只能按名字找**——本域不是
//! 装配者，拿不到它手里那一份副本（正文 K7 的被否项：转授要新装配机制）。

use env::Wait;
use alloc::format;
use core::time::Duration;

use env::{HoleDir, Name, PieToken, TaskId};
use protocol::system::coalition as ccall;
use protocol::system::coalition::core::{Coalition, Fail};
use protocol::system::operator::Where;
use protocol::system::operator as ocall;
use protocol::system::operator::client as operator;
use protocol::system::principal as pcall;
use protocol::system::principal::client::Face;
use protocol::system::principal::core::PrincipalId;
use protocol::session::Quay;
use protocol::session::slip::Slip;
use protocol::system::board as bcall;
use protocol::system::board::client as board;
use runtime::core::port::{self, Access, Policy};
use runtime::core::pile::Pile;
use runtime::env::mail::{self, HolePie};
use runtime::env::room;
use runtime::PAGE_SIZE;


/// 等板 / 等树 / 问名册的总上限（毫秒）。**必须有界**：对面死在头几步时本域不能陪着挂死。
const MS: usize = 1000;

/// 身份服务那份门牌可能落得比本域晚：找不到就再问一次的间隔（毫秒）。
const RETRY_MS: usize = 1;

/// 起服务：**读锚 → 上板 → 铸门牌上树 → 找身份那一份门牌 → 一枚线程招待所有客人**。
///
/// **起手那几步收在一个闭包**（照实记）：与 [`principal`](super::super::principal) 那一台同一
/// 形状——它们清一色是"不成 ⇒ 这一域起不来"的早退步，从前每步一段 `let Ok(..) = .. else { return
/// Err(..) }`，主脉络被岔口切碎。收进闭包之后全走 `?`、失败域在末尾**折一次**（上板与铸门牌
/// 那两桩收尾同理：成败相同）。
pub fn serve() -> Result<(), super::fail::Fail> {
    // 一～五：起手（读锚 → 上板 → 铸门牌 → 上树 → 找身份那一份 → 空册 → 常驻那只组）。
    //
    // 三样东西跟着交出来：**常驻那一问要的两格**（`pile` 是那一组，`entry_hole` 是组下那一枚
    // 门牌）与**那本账**（`book`，`turn` 收它）＋**名册那一份门牌**（`face`，每条写原语都过
    // 它）。门牌本身不必出来——它在闭包里已经交出去过了。
    let (mut book, face, pile, entry_hole) = (|| {
        // 一、锚：`Sire` = 装配者。**只为上板与上树两条会话**——盟无主，核心不需要它
        //     （对照 principal：那边把它当名册钥匙，注入核心那一格）。
        // **起我那一枚线程**（不是 `sire()`：那一手答的是**域级**的生我者，对住本域的
        // 这一枚指的不是编排者。见 `service::Role::args` 的照实记）。
        let assembler = crate::service::assembler().ok_or(super::fail::Fail::Sire)?;

        // 二、上板：只为让板看得见本域的死（它常驻，编排域据此记账）。
        let (_link, board_link) =
            board::open(assembler, Wait::AtMost(MS)).map_err(|_| super::fail::Fail::Board)?;
        board::ask_hole(board_link).map_err(|_| super::fail::Fail::Board)?;

        // 三、门牌那一枚：本域自己开（`entry` 是服务入口的通用记号）。
        let entry = mail::unseal_hole(bcall::ENTRY_MARK).map_err(|_| super::fail::Fail::Tree)?;

        // 四、上树：分 `/sys`、落 `/sys/coalition`、再查回来验一遍（同 router / rtc / principal）。
        let (tree, host) =
            operator::open(assembler, Wait::AtMost(MS)).map_err(|_| super::fail::Fail::Tree)?;
        let talk = operator::ask_hole(host).map_err(|_| super::fail::Fail::Tree)?;
        serve_tree(&tree, talk, host, entry);

        // 四之后：**门禁那一枚**——把这一枚门牌**直接交给持树者**（`host` = 持树者的号，
        // `operator::open` 交回来的那一格）。它据此才判得了"这一位在那枚盟里吗"（`Rule::In`）。
        //
        // 与 principal 那一格同一形状（见 `programs/src/system/operator/bridge.rs` 的 `COORD`
        // 照实记：装配者转授那一版真机报 `operator:coord-ship`，内核 `-1`）。这一枚在手时权限是
        // `FETCH|STORE|VEST`，故子集 `FETCH|STORE` 不越界。装配者那一侧按装配单上那一格
        // （`Eyes::League`）递——两枚门牌**分两帧、次序不定**，持树者收到哪一枚补哪一枚。
        port::ship(
            &HolePie::from_token(entry),
            host,
            Access::FETCH | Access::STORE,
            Policy::NONE,
        )
        .map_err(|_| super::fail::Fail::Tree)?;

        // 五、**身份那一份门牌**：本域是它的客人（K7）。带重试——它可能落得比本域晚。
        let face_entry = find_face(&tree, talk).ok_or(super::fail::Fail::Face)?;
        let face = Face::of(face_entry).map_err(|_| super::fail::Fail::Face)?;

        // 六、一本空册：一枚号都还没铸（**起手不失败**——空册不分配）。
        let book = Coalition::new();

        // 七、常驻：**一只组等门牌那一枚**。这是常态，故等待没有期限。组与名下那一枚孔
        //     在闭包里立好，交给下面那个循环。
        let pile = Pile::unseal(false).map_err(|_| super::fail::Fail::Desk)?;
        let entry_hole = HolePie::from_token(entry);
        pile
            .attach(&entry_hole, HoleDir::Pull)
            .map_err(|_| super::fail::Fail::Desk)?;
        Ok::<_, super::fail::Fail>((book, face, pile, entry_hole))
    })()?;

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
            turn(&mut book, &face, from, &buf[..len]);
        }
    }
}

/// 门上一句话：解帧 → 先过名册 → 交给核心 → **从这一趟自带的那枚孔答回去**。
///
/// 认那枚孔靠**帧里那一格** ＋ **一次 [`mail::reserve`] 验**（同 `principal` 那一面）；
/// `from` 是**内核盖的发送者**。
fn turn(book: &mut Coalition, face: &Face, from: TaskId, frame: &[u8]) {
    let Some((ask, back)) = ccall::Wire::take(frame) else {
        // 不是那个形状（长度不对）：不猜、不动账、也不回话——没有可信的"往哪回"。
        return;
    };
    if !matches!(
        mail::reserve(back),
        Ok((_vestor, owner, mark)) if owner == from && mark == ccall::BACK
    ) {
        // 这一趟没把回信孔交进来、或那一格指的是别人的孔：没有可回的路，账一动不动。
        return;
    }
    // 答一句：**形由 [`ccall::Union`] 说**（三种答形合一：格状态 / 一格答 / 一窗号）——装与发
    // 都不在这一层写字节（缓冲是船台自己那只＝本族最大那一形）。
    // `.ok()`：装不上那一格按构造到不了（`Buf` 由本族 `Message` 自己给，见 `Slip::load`
    // 的照实记）；真到了那里，这一答就发不出去。
    let _ = Slip::<ccall::Union>::seal(back)
        .load(answer(book, face, from, ask))
        .ok()
        .map(|s| s.ship());
    let _ = mail::release(back);
}

/// 把一句问交给核心，编出一句答（**三种答形**：格状态 / 一格答 / 一窗号）。
///
/// **形状由 [`ccall::Wire`] 说**（收帧那一侧已按动作解好：两格载荷的意义随之定，不再是一枚裸码
/// ＋ 两个裸数）。三条**写**原语同一个起手：**先拿发送者过名册**（[`who`]）。三条读不过名册
/// ——`amid` 的 `p` 与两条取窗的键都是问的人给的标签（K6）。
fn answer(book: &mut Coalition, face: &Face, from: TaskId, ask: Option<ccall::Wire>) -> ccall::Union {
    // 表外的动作码：这一问有回信的路，只是这一码我不认（与"读不懂"同一格）。
    let Some(ask) = ask else {
        return ccall::Union::Status(ccall::BAD);
    };
    match ask {
        // `found` 的钥匙是"你得是个已绑定的身份"（K3），**解析出来的那条号只当门卫**：
        // 盟无主（K2），不记铸造者——全族唯一一处。
        ccall::Wire::Found => match who(face, from) {
            Ok(_) => ccall::Union::One(ccall::Reply::value(book.found())),
            Err(fail) => ccall::Union::Status(ccall::fail_to_code(Some(fail))),
        },
        ccall::Wire::Enter(c) => match who(face, from) {
            Ok(w) => match book.enter(w, c) {
                Ok(()) => ccall::Union::One(ccall::Reply::status(ccall::OK)),
                Err(fail) => ccall::Union::Status(ccall::fail_to_code(Some(fail))),
            },
            Err(fail) => ccall::Union::Status(ccall::fail_to_code(Some(fail))),
        },
        ccall::Wire::Leave(c) => match who(face, from) {
            Ok(w) => match book.leave(w, c) {
                Ok(()) => ccall::Union::One(ccall::Reply::status(ccall::OK)),
                Err(fail) => ccall::Union::Status(ccall::fail_to_code(Some(fail))),
            },
            Err(fail) => ccall::Union::Status(ccall::fail_to_code(Some(fail))),
        },
        ccall::Wire::Amid(p, c) => {
            match book.amid(p, c) {
                // "不在"是一句答（`Ok(false)`），"查无此盟"才是这一格。
                Ok(yes) => ccall::Union::One(ccall::Reply::yes(yes)),
                Err(fail) => ccall::Union::Status(ccall::fail_to_code(Some(fail))),
            }
        }
        // 两条取窗：`a` 是键，`b` 是**游标 + 1**（`0` = 没有游标，见 [`ccall`] 的帧那一节）——
        // 游标那一手已经在 `Wire` 里解好了。
        ccall::Wire::Band(c, after) => match book.band(c, after) {
            Ok(window) => ccall::Union::seq(&window),
            Err(fail) => ccall::Union::Status(ccall::fail_to_code(Some(fail))),
        },
        // `bloc` 没有失败域（`p` 是标签，不在任何盟里就是空窗）。
        ccall::Wire::Bloc(p, after) => ccall::Union::seq(&book.bloc(p, after)),
    }
}

/// 发送者此刻代表谁——**"self"的全部护栏就是这一句**（正文"已知边界"）。
///
/// **照实记：两条失败压成一格**——"这条 TID 没绑"与"身份服务答不上来（超时 / 对面没了）"。
/// 压它的理由同 rtc 那一格：**调用方的下一步在两种情况下相同**（别指望这条路）；principal
/// 那枚 `Denied` 翻不过来，因为本族的 `Denied` 是空的（盟无主）。
fn who(face: &Face, from: TaskId) -> Result<PrincipalId, Fail> {
    face.resolve(from, Wait::AtMost(MS))
        .map_err(|_| Fail::Unknown)?
        .ok_or(Fail::Unknown)
}

/// 找**身份服务**那份门牌：`FIND "/sys/principal"`，**找不到就再问**（有界）。
///
/// 门牌是 principal 自己跑完它那一段才落下的（它比本域先起来，但"就绪"与"上树"不是同一步）
/// ——故这一趟**必须有重试**：撞 `UNKNOWN` 就睡 `RETRY_MS` 再来，总预算 [`MS`]。
///
/// **照实记（"总预算"曾经不是预算）**：每一趟 `seek` 的期限原来是写死的 [`MS`]——而那一趟
/// **自己就能花掉 `MS`**，`left` 却只减 `RETRY_MS` ⇒ 真实墙钟上界是"重试次数 × MS"，
/// 与这一行字面差三个数量级。现在**把剩下的预算当这一趟的期限**递下去：总账 ≤ `MS` + 一趟。
///
/// 取回的那一枚**随答话回来**（`operator::find` 的第二格）：从前要按"谁给的"扫本端表、取
/// 满足条件的**最后**一枚——本域表里此刻还有刚验完的那一枚自己的门牌副本，靠次序才分得开。
/// 甲′ 之后号在答话里，次序那条契约随之退场（照实记见 `echo.rs` 那一份）。
fn find_face(link: &Quay, talk: PieToken) -> Option<PieToken> {
    let (Ok(dir), Ok(name)) = (Name::new(pcall::DIR), Name::new(pcall::NAME)) else {
        return None;
    };
    let road = [dir, name];
    // **间接寻址那一手**：名字先经 `seek` 译成号（"还没挂上"那一格也在这里重试），此后按号。
    let mut left = MS;
    let id = loop {
        match operator::seek(talk, link, &road, Wait::AtMost(left)) {
            Ok(id) => break id,
            Err(ocall::UNKNOWN) if left > 0 => {
                let _ = room::sleep(Duration::from_millis(RETRY_MS as u64));
                left = left.saturating_sub(RETRY_MS);
            }
            Err(_) => return None,
        }
    };
    match operator::find(talk, link, id, Wait::AtMost(MS)) {
        Ok((ocall::OK, Some(entry))) => Some(entry),
        _ => None,
    }
}

/// 上树那一趟：**分目录 → 落门牌 → 查回来验一遍**（同 rtc / principal 那一趟）。
///
/// `got` 只是"认出了那一枚"；它指不指得回原物，由**真客人**（`harness/src/member.rs`）证——它照同一条路
/// 找上门、立一枚盟、进进出出。故本域不自问自答。
fn serve_tree(link: &Quay, talk: PieToken, host: TaskId, entry: PieToken) {
    let (Ok(dir), Ok(me)) = (Name::new(ccall::DIR), Name::new(ccall::NAME)) else {
        say("coalition: tree: bad name");
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
    // **`got` 换了来路**（乙′）：见 `ocall::Union::Seed` 的照实记。
    // 拿号问名：**号 ↔ 名**这一对对得起来，才算那枚号是真坐标。
    let pname = plate
        .ok()
        .and_then(|id| operator::name(talk, link, id, Wait::AtMost(MS)).ok());
    say(&format!(
        "coalition: tree part={part} dir={dir_id} land={land} find={find} got={got} entry={} plate={pid} pname={}",
        entry.get(),
        pname.as_ref().map(|n| n.as_str()).unwrap_or("-"),
    ));
}

/// 打一行。调试面是"服务还没起来的嘴"：本域没有控制台，只有它。
fn say(msg: &str) {
    let _ = runtime::env::debug::put(msg);
}
