//! control::client — **客侧**：这条路叫什么，以及一条服务的四手。
//!
//! ```text
//!   Face::of(门牌)              门牌那一枚是树上查回来的（开者 = 对端）
//!   Face::mint(name)            造一条 ⇒ 答一面 Service（名字绑进柄）
//!   Face::service(name)         认已有的一条（不铸、不验）
//!   Service::start / stop / state   一问一答：替这一趟铸一枚回信孔借过去，答完丢掉
//! ```
//!
//! **问话走门牌、答话走这一趟自带的那一枚孔**：报文里没有"往哪回"这一格——号只在持有它的
//! 那张表里念得动（[`communication`](crate::communication) 事实 8），故每一趟借一枚新的回信孔
//! 过去，收的人按"谁给的 + 记号"两格认出它，答完当场放下（与 principal / coalition / operator
//! 那三面同形）。
//!
//! **没有会话可选装**：这一面不另铸一条路、不定泊位——门牌自己就是那条路（同 rtc / principal
//! 那两面）。**泊位那一格（[`BERTH`]）是给"上树那一侧"用的**：control 把自己的入口挂到
//! `/sys/control` 时，装路那一步要它。

use crate::message::Message;
use env::Wait;
use env::{Name, PieToken, TaskId};
use runtime::env::mail;

use crate::communication::establish;
use crate::communication::receiver::Receiver;
use crate::communication::session::Berth;

use super::frame::{self, State, BACK};
use super::Fail;

/// **这条路叫什么**：泊位那一格（[`frame::LINK`] = `control`）＋ 问话孔那一格
/// （[`frame::ASK_MARK`]）。
///
/// 开会话那一手（`Session::open`）要它——control 那一侧上树 / 装配者转授时用同一格。
pub const BERTH: Berth = Berth {
    link: env::Mark::of(frame::LINK),
    ask: frame::ASK_MARK,
};

/// 一面生命周期服务：**树上查回来的门牌** + 它的开者（对端）。
///
/// **它不出编排域**：外面那几枚 `Session` / `Endpoint` / `Receiver` 一个都不露。
pub struct Face {
    entry: PieToken,
    host: TaskId,
}

impl Face {
    /// 把一枚门牌收成一面。
    ///
    /// 对端从**这一枚门闩自己**问出来（[`establish::opened_by`]）——门牌是持表那一侧挂的，
    /// 不是本端开的。
    pub fn of(entry: PieToken) -> Result<Self, Fail> {
        let host = establish::opened_by(entry).ok_or(Fail::Bad)?;
        Ok(Face { entry, host })
    }

    /// 对端是谁（读数用）。
    pub fn host(&self) -> TaskId {
        self.host
    }

    /// **造一个 Service**：建域 + 产它的代表线程（恒产未放行）。
    ///
    /// 镜像由**对端**从清单里取——本端只给名字（见 [`super`] 的"`build` 不拷字节"那一节）。
    pub fn mint(&self, name: Name, wait: Wait) -> Result<Service<'_>, Fail> {
        let said = self.call(frame::Req::Mint(name), wait)?;
        say(said)?;
        Ok(Service { face: self, name })
    }

    /// **认已有的一条**：不铸、不验——名字只是这一面以后叫它的坐标。
    ///
    /// 它成不成立由 [四手](Service) 各自的第一趟答出来（表里没有 ⇒ `Fail::Unknown`）。
    pub fn service(&self, name: Name) -> Service<'_> {
        Service { face: self, name }
    }

    /// 问一句、取一句答。
    ///
    /// **传输失败折进 [`Fail::Bad`]**：借不出回信孔 / 超时 / 答话长度不对——三件事都答
    /// [`Fail::Bad`]。压它的理由：**对本端是同一个下一步**（这一趟别指望了）。分得开它们的那一格
    /// 在**对面**：语义格是持表那一侧真会答的码，"没走到"是本端自己在码表之外判的。
    fn call(&self, act: frame::Req, wait: Wait) -> Result<frame::Said, Fail> {
        // **先铸、先交，再推**（次序是契约的一半，见 `communication::establish::lend_out`）：
        // 那一枚"种在对端表里的号"随帧一起过去 ⇒ 对端一次 `Reserve` 就认得出，不必扫表。
        let (back, seed) = establish::lend_out(self.entry, BACK).map_err(|()| Fail::Bad)?;
        // 编一问：**一张表 ＋ 一处编**（`back` 是运输那一格，随动作一起进帧）。
        let mut frame = [0u8; frame::Ask::LEN];
        act.ask(seed).store(&mut frame);
        if mail::HolePie::from_token(self.entry).push(&frame).is_err() {
            // 推不出去 ⇒ 这一趟根本没到对端，那一枚收回来。
            let _ = mail::release(back);
            return Err(Fail::Bad);
        }
        // 收：答话走**这一趟借出去的那一枚孔**（缓冲由调用方给——这一形 2 字节）。
        let mut buf = frame::Said::EMPTY;
        let got = Receiver::<frame::Said>::from_token(back)
            .recv(buf.as_mut(), wait)
            .map_err(|_| Fail::Bad);
        // 这一趟的回信孔只活到这句话答完：收走就放下（不管成没成）。
        let _ = mail::release(back);
        got
    }
}

/// **一条服务**：`mint` 那一下把名字绑进柄 ⇒ 此后那几手不再重复传名字。
///
/// 名字不变、对端不变，故柄里只有这两格；四手各带自己的 `Wait`（**预算不是柄的状态**）。
pub struct Service<'a> {
    face: &'a Face,
    name: Name,
}

impl Service<'_> {
    /// 这一条叫什么（读数用）。
    pub fn name(&self) -> &Name {
        &self.name
    }

    /// **放行 + 等就绪**（有通道的那条顺带逐条认领）。
    pub fn start(&self, wait: Wait) -> Result<(), Fail> {
        say(self.face.call(frame::Req::Start(self.name), wait)?)
    }

    /// **下令收掉**（下令即回，不等它收完）。
    pub fn stop(&self, wait: Wait) -> Result<(), Fail> {
        say(self.face.call(frame::Req::Stop(self.name), wait)?)
    }

    /// 这一条此刻处于哪个生命阶段。
    pub fn state(&self, wait: Wait) -> Result<State, Fail> {
        let (_, at) = split(self.face.call(frame::Req::State(self.name), wait)?)?;
        State::of_code(at).ok_or(Fail::Bad)
    }
}

/// 一句答拆两格：状态先过码表，`OK` 才交出答案那一格。
///
/// 它不用 `self`（纯解码）⇒ 自由函数，不是一个为了"看起来属于 Face"而写成方法的手。
fn split(said: frame::Said) -> Result<(u8, u8), Fail> {
    match frame::code_to_fail(said.status) {
        None if said.status == frame::OK => Ok((said.status, said.a)),
        Some(fail) => Err(fail),
        // 表外那一格（连 `OK` 都没读成）⇒ 与"没走到"同一格。
        None => Err(Fail::Bad),
    }
}

/// 三手写只关心"成没成"：拆开、丢掉答案那一格。
fn say(said: frame::Said) -> Result<(), Fail> {
    split(said).map(|_| ())
}
