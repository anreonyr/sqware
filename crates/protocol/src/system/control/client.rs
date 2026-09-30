//! control::client — **客侧**：这条路叫什么，以及一条服务的四手。
//!
//! ```text
//!   Face::of(门牌)              门牌那一枚是树上查回来的（开者 = 对端）
//!   Face::mint(name)            造一条 ⇒ 答一面 Service（名字绑进柄）
//!   Face::service(name)         认已有的一条（不铸、不验）
//!   Service::start(wait)        放行 ⇒ 答一枚 Started（**身子在这里固定下来**）
//!   Service::stop / state       一问一答：替这一趟铸一枚回信孔借过去，答完丢掉
//! ```
//!
//! **两枚柄，不是一枚带可变状态**：`start` 之前身子还不存在，`start` 之后才有——把"起没起"
//! 做成 `Service` 上的可变状态，会让同一个柄时而有一枚号时而没有。故 `start` 进一枚柄、出
//! 另一枚（[`Started`]），与树那一族的 `Face::pane -> Pane` / `Pane::tile -> Tile` 同形。
//!
//! **问话走门牌、答话走这一趟自带的那一枚孔**：报文里没有"往哪回"这一格——号只在持有它的
//! 那张表里念得动（[`communication`](crate::communication) 事实 8），故每一趟借一枚新的回信孔
//! 过去，收的人按"谁给的 + 记号"两格认出它，答完当场放下（与 principal / coalition / operator
//! 那三面同形）。
//!
//! **没有会话可选装**：这一面不另铸一条路、不定泊位——门牌自己就是那条路（同 rtc / principal
//! 那两面）。**照实记（泊位那一格今天没有消费者）**：从前挂载者要"自己给自己那棵树上树"，
//! 故要一条会话（第一个消费者就是那一版）；今天"挂到 `/svc/sys/control`"由**持树者在自己核里落**
//! （装配者只递入口与两段名字，见 `programs/src/system/mod.rs::Assembly::mount_control`），
//! 这一面**一处会话都
//! 不开**。故 [`BERTH`] 连同它那两格记号（[`frame::LINK`] / [`frame::ASK_MARK`]）今天只剩
//! "这条路若要走，叫什么"这一句声明——留着是因为它们是**这份协议的坐标**（`frame.rs` 那两条
//! 防撞断言读的就是它们），而不是因为有客人。

use alloc::string::String;
use crate::message::Message;
use env::Wait;
use env::{HoleDir, PieToken, TaskId};
use runtime::env::mail;

use crate::communication::establish;
use crate::communication::receiver::{Receiver, RecvFail};
use crate::communication::session::Berth;

use super::Fail;
use super::frame::{self, BACK, State};

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
    ///
    /// **这一步还没有身子**：`Mint` 只把域与线程造出来、还压在对端手里等放行；身子是
    /// [`Service::start`] 那一趟交回来的（[`Started::id`]）。
    pub fn mint(&self, name: String, wait: Wait) -> Result<Service<'_>, Fail> {
        let said = self.call(frame::Req::Mint(name.clone()), wait)?;
        read(said)?;
        Ok(Service { face: self, name })
    }

    /// **认已有的一条**：不铸、不验——名字只是这一面以后叫它的坐标。
    ///
    /// 它成不成立由 [四手](Service) 各自的第一趟答出来（表里没有 ⇒ `Fail::Unknown`）。
    pub fn service(&self, name: String) -> Service<'_> {
        Service { face: self, name }
    }

    /// 问一句、取一句答。
    ///
    /// **传输失败折进 [`Fail::Bad`]**：借不出回信孔 / 超时 / 答话长度不对——三件事都答
    /// [`Fail::Bad`]。压它的理由：**对本端是同一个下一步**（这一趟别指望了）。分得开它们的那一格
    /// 在**对面**：语义格是持表那一侧真会答的码，"没走到"是本端自己在码表之外判的。
    fn call(&self, act: frame::Req, wait: Wait) -> Result<frame::Said, Fail> {
        /// **四步分开报**（照实记，与 `principal` 那一面同一条）：这一格盖着四件事——借回信孔 /
        /// 编帧 / 递出 / 收答——而"装配期折一趟"从前只看得出这一格。故每一步各留一行读数。
        fn deny(step: &str) -> Fail {
            crate::debug!("control: call deny={step}");
            Fail::Bad
        }
        // **先铸、先交，再推**（次序是契约的一半，见 `communication::establish::lend_out`）：
        // 那一枚"种在对端表里的号"随帧一起过去 ⇒ 对端一次 `Reserve` 就认得出，不必扫表。
        let (back, seed) = establish::lend_out(self.entry, BACK).map_err(|()| deny("borrow"))?;
        // 编一问：**一张表 ＋ 一处编**（`back` 是运输那一格，随动作一起进帧）。
        let mut frame = [0u8; frame::Ask::LEN];
        let n = act
            .ask(seed)
            .store_at(&mut frame, 0)
            .ok_or_else(|| deny("encode"))?;
        let door = mail::HolePie::from_token(self.entry);
        // **递出，且等到轮到自己**（照实记见 `HolePie::push`）。**不等自己那只手**——见下 `wait`。
        if door.push(&frame[..n], Wait::Forever).is_err() {
            // 推不出去 ⇒ 这一趟根本没到对端，那一枚收回来。
            // **读者结清（B-a）**：这一枚是本端铸的、只活这一趟 ⇒ **先封印、再放下**——另一头若还在等
            // "这只手被取走"（`Sender::Drop`），而它等的这一枚只有我手里这一份。
            let _ = mail::seal(back);
            let _ = mail::release(back);
            return Err(deny("push"));
        }
        // 收：答话走**这一趟借出去的那一枚孔**（缓冲由调用方给——这一形 2 字节）。
        let mut buf = frame::Said::EMPTY;
        let got = Receiver::<frame::Said>::from_token(back)
            .recv(buf.as_mut(), wait)
            .map_err(|e| match e {
                RecvFail::Unread => deny("recv-unread"),
                RecvFail::Mail(m) => {
                    crate::debug!("control: call deny=recv:{}", m.code());
                    Fail::Bad
                }
            });
        // 答话回来了 ⇒ 对面早取走了；没回来也得把这一手收口（那条报不许悬）：推的人等"孔空"。
        let _ = door.wait(HoleDir::Push, Wait::Forever);
        // 这一趟的回信孔只活到这句话答完：收走就放下（不管成没成）。
        // **读者结清（B-a）**：这一枚是本端铸的、只活这一趟 ⇒ **先封印、再放下**——另一头若还在等
        // "这只手被取走"（`Sender::Drop`），而它等的这一枚只有我手里这一份。
        let _ = mail::seal(back);
        let _ = mail::release(back);
        got
    }
}

/// **一条服务**：`mint` 那一下把名字绑进柄 ⇒ 此后那几手不再重复传名字。
///
/// 名字不变、对端不变，故柄里只有这两格；四手各带自己的 `Wait`（**预算不是柄的状态**）。
pub struct Service<'a> {
    face: &'a Face,
    name: String,
}

impl Service<'_> {
    /// 这一条叫什么（读数用）。
    pub fn name(&self) -> &String {
        &self.name
    }

    /// **放行 + 等就绪**（有通道的那条顺带逐条认领）⇒ 答一枚 [`Started`]。
    ///
    /// **身子在这一趟里到手**：子域的代表线程在放行之前就已经产好（`service::mint` 的口径），
    /// 故这一答非成即败，不存在"起来了但没有号"这一格。
    pub fn start(&self, wait: Wait) -> Result<Started<'_>, Fail> {
        let said = self.face.call(frame::Req::Start(self.name.clone()), wait)?;
        Ok(Started {
            face: self.face,
            name: self.name.clone(),
            task: read(said)?.task,
        })
    }

    /// **下令收掉**（下令即回，不等它收完）。
    pub fn stop(&self, wait: Wait) -> Result<(), Fail> {
        let said = self.face.call(frame::Req::Stop(self.name.clone()), wait)?;
        read(said).map(|_| ())
    }

    /// 这一条此刻处于哪个生命阶段。
    pub fn state(&self, wait: Wait) -> Result<State, Fail> {
        let said = self.face.call(frame::Req::State(self.name.clone()), wait)?;
        // **先过码表**（`read`），再看第二格；表外的判别值不猜。
        State::of_code(read(said)?.a).ok_or(Fail::Bad)
    }
}

/// **一条起好的服务**：`Service::start` 的产物——**身子（那一枚线程）在这里固定下来**。
///
/// 它不重抄 [`Service`] 那几手（照树那一族的先例：`Tile` 不抄 `Pane` 的手，只给一条回头的路）：
/// 要 `stop` / `state` 就 [`Started::service`] 拿回那一柄。
pub struct Started<'a> {
    face: &'a Face,
    name: String,
    task: TaskId,
}

impl Started<'_> {
    /// 这一条叫什么（读数用）。
    pub fn name(&self) -> &String {
        &self.name
    }

    /// **它此刻是哪一枚线程**（子域的代表线程）。
    ///
    /// 这是本协议**唯一**交得出域外的那一格身子：`Endpoint` 的孔不行（见 [`super`] 的
    /// "通道副本不能跨域"那一节），而 `TaskId` 跨域有意义。
    pub fn id(&self) -> TaskId {
        self.task
    }

    /// 回到那一柄（`stop` / `state` 的入口）。
    pub fn service(&self) -> Service<'_> {
        Service {
            face: self.face,
            name: self.name.clone(),
        }
    }
}

/// 一句答拆开：**状态先过码表**，`OK` 才把整个答话交出来。
///
/// 四手共用这一手——三手只要"成没成"（`read(said).map(|_| ())`），[`Service::state`] 取第二格，
/// [`Service::start`] 取第三格。**答话的解码只有这一条路**（`Said::fetch` 那张表 ＋ 这里这一次
/// 状态过码表），不留第二个入口。
fn read(said: frame::Said) -> Result<frame::Said, Fail> {
    match frame::code_to_fail(said.status) {
        None if said.status == frame::OK => Ok(said),
        Some(fail) => Err(fail),
        // 表外那一格（连 `OK` 都没读成）⇒ 与"没走到"同一格。
        None => Err(Fail::Bad),
    }
}
