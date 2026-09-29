//! coalition::client — **客侧**：一面结盟服务，以及它的三个结果形状。
//!
//! ```text
//!   Face::of(门牌)           门牌那一枚是树上查回来的（开者 = 对端）
//!   Face::found()            立一枚新盟 ⇒ 答一面 Coalition（号绑进柄）
//!   Face::coalition(c)       认已有的一枚（号是别人给的标签，本手不去问它对不对）
//!   Face::bloc(p, ..)        这一位在哪些盟里（一趟取窗）
//!   Coalition::enter / leave / holds / members   这一枚盟自己那几手
//!   Band / Bloc              一次取窗的结果值（一页 + 游标 + `next`）
//! ```
//!
//! **问话走门牌、答话走这一趟自带的那一枚孔**：报文里没有"往哪回"这一格——号只在持有它的
//! 那张表里念得动（[`communication`](crate::communication) 事实 8），故每一趟借一枚新的回信孔过去，收的人按
//! "谁给的 + 记号"两格认出它，答完当场放下。答有三形（[`frame::Union`]）：一格状态 / 一格答 /
//! 一窗号——**形状由长度分得开**，故这面不用"原样的字节"那一手。
//!
//! **盟籍是身份号之间的事实**：这两轴只互相递 [`PrincipalId`] / [`CoalitionId`] 这两个**值**，
//! 不把 Principal 那一面的柄引进来。
//!
//! **客侧没有"我是谁"这一格**：`enter` / `leave` 不带身份参数——对端拿的是内核盖的那枚印章。
//! 这一格的诚实性是**签名给的**（见正文"已知边界"）。
//!
//! **没有会话可选装**：这一面不另铸一条路、不定泊位——门牌自己就是那条路（同 rtc / principal
//! 那两面）。

use crate::message::Message;
use crate::system::principal::PrincipalId;
use env::Wait;
use env::{HoleDir, PieToken, TaskId};
use runtime::env::mail;

use super::frame::{self, BACK, CoalitionId, Fail, Window};
use crate::communication::establish;
use crate::communication::receiver::Receiver;

/// 一面结盟服务：**树上查回来的门牌** + 它的开者（对端）。
pub struct Face {
    entry: PieToken,
    host: TaskId,
}

impl Face {
    /// 把一枚门牌收成一面。
    ///
    /// 对端从**这一枚门闩自己**问出来（[`establish::opened_by`]）——门牌是 Server 挂的，不是本端开的。
    pub fn of(entry: PieToken) -> Result<Self, Fail> {
        let host = establish::opened_by(entry).ok_or(Fail::Unknown)?;
        Ok(Face { entry, host })
    }

    /// 对端是谁。**这是这一面唯一的读数，不出线**（不在那七条原语里）。
    ///
    /// 全树无生产消费者，留着只为"这一面在跟谁说话"答得出来——它是诊断读数，不是协议面。
    pub fn host(&self) -> TaskId {
        self.host
    }

    /// 盟 · 写：立一枚新号（空盟）——**自己是哪一位**由内核盖的印章说。
    pub fn found(&self, wait: Wait) -> Result<Coalition<'_>, Fail> {
        let said = self.call(frame::Req::Found, wait)?;
        Ok(self.coalition(CoalitionId::new(payload(said)? as usize)))
    }

    /// 把一枚盟号收成 [`Coalition`]（读面：号是别人给的标签，本手不去问它对不对）。
    pub fn coalition(&self, c: CoalitionId) -> Coalition<'_> {
        Coalition { face: self, at: c }
    }

    /// 盟 · 读：`p` 此刻在哪些盟里（**一趟取窗**，序 = 号序升序；游标是阈值，见 [`Bloc::next`]）。
    pub fn bloc(
        &self,
        p: PrincipalId,
        after: Option<CoalitionId>,
        wait: Wait,
    ) -> Result<Bloc<'_>, Fail> {
        let page = self.window(frame::Req::Bloc(p, after), wait)?;
        Ok(Bloc {
            face: self,
            p,
            page,
        })
    }

    /// 问一句、取一答（**形状由长度分**：这一族三形在线上分得开，见 [`frame::Union`]）。
    ///
    /// **传输失败折进 [`Fail::Unknown`]**：借不出回信孔 / 超时 / 收不下一答（空帧、长度不落在
    /// 三形里）——三件事都答 [`Fail::Unknown`]，与"**这枚盟没铸过**"同一格。压它的理由与
    /// principal 那一面同：**对本端是同一个下一步**（这一趟别指望了）。**照实记（补一句）**：
    /// 这只是说**传输**那三件事落在哪一格；答话那一形里的码照读——开面那一刀添的
    /// [`Fail::Denied`] 就走它（见 `payload` / `flag` / [`Face::window`] 那三处同一口径）。
    fn call(&self, act: frame::Req, wait: Wait) -> Result<frame::Union, Fail> {
        // **先铸、先交，再推**（次序是契约的一半，见 `communication::establish::lend_out`）。
        let (back, seed) = establish::lend_out(self.entry, BACK).map_err(|()| Fail::Unknown)?;
        // 编一问：**一张表 ＋ 一处编**（`back` 是运输那一格，随动作一起进帧）。
        let mut frame = [0u8; frame::Query::LEN];
        let n = act.query(seed).store_at(&mut frame, 0).ok_or(Fail::Unknown)?;
        let door = mail::HolePie::from_token(self.entry);
        // **递出，且等到轮到自己**（照实记见 `HolePie::push`）：不丢那一等，单槽门面上的两位
        // 客人就只会**排队**，不会把后到的那一趟当场折成失败。**不等自己那只手**——见下 `wait`。
        if door.push(&frame[..n], Wait::Forever).is_err() {
            // **读者结清（B-a）**：这一枚是本端铸的、只活这一趟 ⇒ **先封印、再放下**——另一头若还在等
            // "这只手被取走"（`Sender::Drop`），而它等的这一枚只有我手里这一份。
            let _ = mail::seal(back);
            let _ = mail::release(back);
            return Err(Fail::Unknown);
        }
        // 收：答话走**这一趟借出去的那一枚孔**（缓冲由调用方给＝本族最大那一形）。
        // 两格失败（没收到 / 解不动）在这一侧落同一格：`Unknown`。
        let mut buf = frame::Union::EMPTY;
        let got = Receiver::<frame::Union>::from_token(back)
            .recv(buf.as_mut(), wait)
            .map_err(|_| Fail::Unknown);
        // 答话回来了 ⇒ 对面早取走了；没回来也得把这一手收口（那条报不许悬）：推的人等"孔空"。
        let _ = door.wait(HoleDir::Push, Wait::Forever);
        // 这一趟的回信孔只活到这句话答完：收走就放下（不管成没成）。
        // **读者结清（B-a）**：这一枚是本端铸的、只活这一趟 ⇒ **先封印、再放下**——另一头若还在等
        // "这只手被取走"（`Sender::Drop`），而它等的这一枚只有我手里这一份。
        let _ = mail::seal(back);
        let _ = mail::release(back);
        got
    }

    /// 取一窗：**先看状态那一格**（失败域 + 读不懂），再认窗那一形。
    ///
    /// 一格答那一形不是窗，它那一格码照样交出来（`band` / `bloc` 那一问的失败走它）。
    fn window<T: crate::id::Id>(&self, act: frame::Req, wait: Wait) -> Result<Window<T>, Fail> {
        match self.call(act, wait)? {
            frame::Union::Seq(seq) => Ok(seq.window()),
            frame::Union::Status(code) | frame::Union::One(frame::Reply { status: code, .. }) => {
                Err(code_to_fail(code))
            }
        }
    }
}

/// **一枚盟**：`CoalitionId` 是固定下来的宾语，那几手不再重复传它。
///
/// 它没有"主人"这一格（K2）：盟无主，故这几手只有"集合运算"那几只钥匙。
pub struct Coalition<'a> {
    face: &'a Face,
    at: CoalitionId,
}

impl Coalition<'_> {
    /// 这一枚是几（读数用；**跨协议交接的只有这个值**）。
    pub fn id(&self) -> CoalitionId {
        self.at
    }

    /// 盟 · 写：**我**进这一枚。
    ///
    /// **没有"谁"这一格**（照实记：这一手原先收一个 `_p: PrincipalId`，那个形参不参与任何判定）：
    /// 线上只有盟号那一格（`Req::Enter(c)`），收的人认的是**内核盖的那枚印章** ⇒ 这一手能表达的
    /// 只有"**我**进这枚盟"，而"我" = 本端此刻代表的那一位（`Face::task` / `Task::principal` 那条路）。
    /// 传一个不是自己的号进来，既不该改变行为、也不该被静默接受——故干脆不给这一格。
    ///
    /// 要**替别人**入盟 ⇒ [`Coalition::admit`]（K2 翻案那一刀给的那一条带名字的路：盟主点名，
    /// 服务端过名册把名点实）——注意它**不是**把这一手放宽：这一手照旧只做得了"我进"。
    pub fn enter(&self, wait: Wait) -> Result<(), Fail> {
        let said = self.face.call(frame::Req::Enter(self.at), wait)?;
        payload(said).map(|_who| ())
    }

    /// 盟 · 写：把**另一位**（`target` = 它那一枚 TID）放进这一枚——**只有盟主叫得动**。
    ///
    /// **这就是"驱动自己入不了别人的名"那条路的另一半**（设备账那一族的 `bond`）：`enter`
    /// 的钥匙是发送者那一格，故"许某一位进这一类"只能由**立盟那位**替它说；本手就是那一位
    /// 代报名者手里的那一手。
    ///
    /// **`target` 是 TID 不是身份号**（照实记）：解名在服务端（它本就有名册问面），因为运行期
    /// 唯一说得出口的"哪一位"就是内核盖的那枚印章。**这不是放弃护栏**：本端仍要过名册
    /// （"你得是个已绑身份"），而"这一枚盟归不归你代报名"由服务端那个**盟主**那一格答
    /// （对不上一律 [`Fail::NotChief`]）。
    pub fn admit(&self, target: TaskId, wait: Wait) -> Result<(), Fail> {
        let said = self.face.call(frame::Req::Admit(self.at, target), wait)?;
        payload(said).map(|_who| ())
    }

    /// 盟 · 写：**我**出这一枚。撞空也成（集合运算没有"第二次"）。
    ///
    /// 同 [`Coalition::enter`]：主体由印章说，故这一手不收身份那一格。
    pub fn leave(&self, wait: Wait) -> Result<(), Fail> {
        let said = self.face.call(frame::Req::Leave(self.at), wait)?;
        payload(said).map(|_who| ())
    }

    /// 盟 · 读：`p` 在不在这一枚里。**两件事两个落点**——`Ok(false)` 是不在，
    /// `Err(Unknown)` 是这枚盟不存在。
    pub fn holds(&self, p: PrincipalId, wait: Wait) -> Result<bool, Fail> {
        let said = self.face.call(frame::Req::Amid(p, self.at), wait)?;
        // `AMID` 的答案在**有没有**那一格（在 / 不在），8 字节那一格留空。
        flag(said)
    }

    /// 盟 · 读：这一枚里此刻有谁（**一趟取窗**；`after` 是阈值，`None` = 从头取）。
    pub fn members(&self, after: Option<PrincipalId>, wait: Wait) -> Result<Band<'_>, Fail> {
        let page = self.face.window(frame::Req::Band(self.at, after), wait)?;
        Ok(Band {
            face: self.face,
            at: self.at,
            page,
        })
    }
}

/// **一次取窗的结果值**：一页成员 ＋ 游标规则（[`Band::next`]）。
///
/// 它不是长期存在的资源对象：只带这一页与"这一位在问什么"，故只有读数、续取、遍历三手。
pub struct Band<'a> {
    face: &'a Face,
    at: CoalitionId,
    page: Window<PrincipalId>,
}

impl Band<'_> {
    /// 窗外**还有**（接着取还会答出东西）。
    pub fn more(&self) -> bool {
        self.page.more()
    }

    /// 这一页的号（序 = 号序升序）。
    pub fn iter(&self) -> impl Iterator<Item = PrincipalId> + '_ {
        self.page.iter()
    }

    /// 接着取下一窗：**拿末一枚当阈值**（空页 ⇒ 再取也只有空）。
    pub fn next(&self, wait: Wait) -> Result<Band<'_>, Fail> {
        let after = self.page.last();
        let page = self.face.window(frame::Req::Band(self.at, after), wait)?;
        Ok(Band {
            face: self.face,
            at: self.at,
            page,
        })
    }
}

/// **一次取窗的结果值**（反向：这一位在哪些盟里）。
pub struct Bloc<'a> {
    face: &'a Face,
    p: PrincipalId,
    page: Window<CoalitionId>,
}

impl Bloc<'_> {
    /// 窗外**还有**。
    pub fn more(&self) -> bool {
        self.page.more()
    }

    /// 这一页的号（序 = 号序升序）。
    pub fn iter(&self) -> impl Iterator<Item = CoalitionId> + '_ {
        self.page.iter()
    }

    /// 接着取下一窗：**拿末一枚当阈值**。
    pub fn next(&self, wait: Wait) -> Result<Bloc<'_>, Fail> {
        let after = self.page.last();
        let page = self.face.window(frame::Req::Bloc(self.p, after), wait)?;
        Ok(Bloc {
            face: self.face,
            p: self.p,
            page,
        })
    }
}

/// 读一格答：**不是那一形 ⇒ 读不懂**，是那一形再看状态那一格。
///
/// 一格答那一形在与窗那两问上是"失败"（成功的那两问答的是窗）。
///
/// **三处读者同一个口径**（照实记：这一处原先只认 `One` 那一形，把裸 `Status` 整个折成
/// `Unknown`——那条捷径在"本族只有一格失败"时对得上，因为那时 `found` 那几条唯一收得到的裸
/// 状态就是 `UNKNOWN`。开面那一刀添了 [`Fail::Denied`] 之后它就不对了：**面不对**答的是裸
/// `Status(DENIED)`，被折成 `Unknown` 会把"换一枚门牌"读成"这枚盟不存在"。今天与
/// [`Face::window`] 那一处逐字同形：裸状态照读它的码，**形状不对**才是"读不懂"。
fn payload(said: frame::Union) -> Result<u64, Fail> {
    match said {
        frame::Union::One(reply) => match frame::code_to_fail(reply.status) {
            None if reply.status == frame::OK => Ok(reply.a),
            Some(fail) => Err(fail),
            None => Err(Fail::Unknown),
        },
        // 失败那一形（裸状态）：码照读。
        frame::Union::Status(code) => Err(code_to_fail(code)),
        // 形状不对（问"入盟"却答了一窗号之类）⇒ 读不懂。
        frame::Union::Seq(_) => Err(Fail::Unknown),
    }
}

/// 读一格答里那一格"是 / 不是"。
fn flag(said: frame::Union) -> Result<bool, Fail> {
    match said {
        frame::Union::One(reply) => match frame::code_to_fail(reply.status) {
            None if reply.status == frame::OK => Ok(reply.flag),
            Some(fail) => Err(fail),
            None => Err(Fail::Unknown),
        },
        // 同上：裸状态照读它的码。
        frame::Union::Status(code) => Err(code_to_fail(code)),
        frame::Union::Seq(_) => Err(Fail::Unknown),
    }
}

/// 线上那一格码 → 失败域；表外（含 `BAD`）折 [`Fail::Unknown`]。
///
/// **它现在读得出 `Denied`**（开面那一刀把那一格添进了双射表）：[`Face::window`] / `payload` /
/// `flag` 三处都走它。"表外"仍是 `BAD` 那一类——`None` 与"没走到"同格（传输失败那一节）。
fn code_to_fail(code: u8) -> Fail {
    frame::code_to_fail(code).unwrap_or(Fail::Unknown)
}
