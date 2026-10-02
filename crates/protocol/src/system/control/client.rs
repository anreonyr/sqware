//! 这条路叫什么，以及一条服务的四手。

use crate::wire::message::Message;
use alloc::string::String;
use env::Wait;
use env::{HoleDir, PieToken, TaskId};
use runtime::env::mail;

use crate::communication::session::establish;
use crate::communication::hand::{Receiver, RecvFail};
use crate::communication::session::Berth;

use super::Fail;
use super::frame::{self, BACK, State};
use env::wire::Span as _;

/// **这条路叫什么**：泊位那一格（frame::LINK = `control`）＋ 问话孔那一格
/// （frame::ASK_MARK）
/// 开会话那一手（Session::open）要它——control 那一侧上树 / 装配者转授时用同一格
pub const BERTH: Berth = Berth {
    link: env::Mark::of(frame::LINK),
    ask: frame::ASK_MARK,
};

/// 一面生命周期服务：**树上查回来的门牌** + 它的开者（对端）
/// **它不出编排域**：外面那几枚 `Session` / `Endpoint` / `Receiver` 一个都不露
pub struct Face {
    entry: PieToken,
    host: TaskId,
}

impl Face {
    /// 把一枚门牌收成一面
    /// 对端从**这一枚门闩自己**问出来（establish::opened_by）——门牌是持表那一侧挂的
    /// 不是本端开的
    pub fn of(entry: PieToken) -> Result<Self, Fail> {
        let host = establish::opened_by(entry).ok_or(Fail::Bad)?;
        Ok(Face { entry, host })
    }

    /// 对端是谁（读数用）
    pub fn host(&self) -> TaskId {
        self.host
    }

    /// **造一个 Service**：建域 + 产它的代表线程（恒产未放行）
    /// 镜像由**对端**从清单里取——本端只给名字（见 super 的"`build` 不拷字节"那一节）
    /// **这一步还没有身子**：`Mint` 只把域与线程造出来、还压在对端手里等放行；身子是
    pub fn mint(&self, name: String, wait: Wait) -> Result<Service<'_>, Fail> {
        let said = self.call(frame::Req::Mint(name.clone()), wait)?;
        read(said)?;
        Ok(Service { face: self, name })
    }

    /// **认已有的一条**：不铸、不验——名字只是这一面以后叫它的坐标
    /// 它成不成立由 [四手](Service) 各自的第一趟答出来（表里没有 ⇒ Fail::Unknown）
    pub fn service(&self, name: String) -> Service<'_> {
        Service { face: self, name }
    }

    /// 问一句、取一句答
    /// **传输失败折进 Fail::Bad**：借不出回信孔 / 超时 / 答话长度不对——三件事都答
    /// 在**对面**：语义格是持表那一侧真会答的码，"没走到"是本端自己在码表之外判的
    fn call(&self, act: frame::Req, wait: Wait) -> Result<frame::Said, Fail> {
        fn deny(step: &str) -> Fail {
            crate::debug!("control: call deny={step}");
            Fail::Bad
        }
        // **先铸、先交，再推**（次序是契约的一半，见 communication::session::establish::lend_out）：
        // 那一枚"种在对端表里的号"随帧一起过去 ⇒ 对端一次 `Reserve` 就认得出，不必扫表。
        let (back, seed) = establish::lend_out(self.entry, BACK).map_err(|()| deny("borrow"))?;
        // 编一问：**一张表 ＋ 一处编**（`back` 是运输那一格，随动作一起进帧）。
        let mut frame = [0u8; frame::Ask::LEN];
        let n = act
            .ask(seed)
            .store_at(&mut frame, 0)
            .ok_or_else(|| deny("encode"))?;
        let door = mail::HolePie::from_token(self.entry);
        if door.push(&frame[..n], Wait::Forever).is_err() {
            // "这只手被取走"（Sender::Drop），而它等的这一枚只有我手里这一份。
            let _ = mail::seal(back);
            let _ = mail::release(back);
            return Err(deny("push"));
        }
        let mut buf = frame::Said::EMPTY;
        let got = Receiver::<frame::Said>::from_token(back)
            .recv(buf.as_mut(), wait)
            .map_err(|e| match e {
                RecvFail::Unread(len) => {
                    crate::debug!("control: call deny=recv-unread len={len}");
                    Fail::Bad
                }
                RecvFail::Mail(m) => {
                    crate::debug!("control: call deny=recv:{}", m.code());
                    Fail::Bad
                }
            });
        let _ = door.wait(HoleDir::Push, Wait::Forever);
        // "这只手被取走"（Sender::Drop），而它等的这一枚只有我手里这一份。
        let _ = mail::seal(back);
        let _ = mail::release(back);
        got
    }
}

/// **一条服务**：`mint` 那一下把名字绑进柄 ⇒ 此后那几手不再重复传名字
/// 名字不变、对端不变，故柄里只有这两格；四手各带自己的 `Wait`（**预算不是柄的状态**）
pub struct Service<'a> {
    face: &'a Face,
    name: String,
}

impl Service<'_> {
    /// 这一条叫什么（读数用）
    pub fn name(&self) -> &String {
        &self.name
    }

    /// **放行 + 等就绪**（有通道的那条顺带逐条认领）⇒ 答一枚 Started
    pub fn start(&self, wait: Wait) -> Result<Started<'_>, Fail> {
        let said = self.face.call(frame::Req::Start(self.name.clone()), wait)?;
        Ok(Started {
            face: self.face,
            name: self.name.clone(),
            task: read(said)?.task,
        })
    }

    pub fn stop(&self, wait: Wait) -> Result<(), Fail> {
        let said = self.face.call(frame::Req::Stop(self.name.clone()), wait)?;
        read(said).map(|_| ())
    }

    /// 这一条此刻处于哪个生命阶段
    pub fn state(&self, wait: Wait) -> Result<State, Fail> {
        let said = self.face.call(frame::Req::State(self.name.clone()), wait)?;
        // **先过码表**（`read`），再看第二格；表外的判别值不猜。
        State::of_code(read(said)?.a).ok_or(Fail::Bad)
    }
}

/// 它不重抄 Service 那几手（照树那一族的先例：`Tile` 不抄 `Pane` 的手，只给一条回头的路）
/// 要 `stop` / `state` 就 Started::service 拿回那一柄
pub struct Started<'a> {
    face: &'a Face,
    name: String,
    task: TaskId,
}

impl Started<'_> {
    /// 这一条叫什么（读数用）
    pub fn name(&self) -> &String {
        &self.name
    }

    /// **它此刻是哪一枚线程**（子域的代表线程）
    /// 这是本协议**唯一**交得出域外的那一格身子：`Endpoint` 的孔不行（见 super 的
    /// "通道副本不能跨域"那一节），而 `TaskId` 跨域有意义
    pub fn id(&self) -> TaskId {
        self.task
    }

    /// 回到那一柄（`stop` / `state` 的入口）
    pub fn service(&self) -> Service<'_> {
        Service {
            face: self.face,
            name: self.name.clone(),
        }
    }
}

/// 一句答拆开：**状态先过码表**，`OK` 才把整个答话交出来
/// 状态过码表），不留第二个入口
fn read(said: frame::Said) -> Result<frame::Said, Fail> {
    match frame::code_to_fail(said.status) {
        None if said.status == frame::OK => Ok(said),
        Some(fail) => Err(fail),
        // 表外那一格（连 `OK` 都没读成）⇒ 与"没走到"同一格。
        None => Err(Fail::Bad),
    }
}
