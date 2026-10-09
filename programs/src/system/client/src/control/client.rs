/// 这条路叫什么，以及一条服务的四手。
use alloc::string::String;
use env::{PieToken, TaskId, Wait};

use ipc::rpc::{Fail as RpcFail, request::Sender};
use ipc::session::establish;
use ipc::time::Deadline;

use super::Fail;
use super::frame::{self, State};
use system_api::control::Call;
use system_api::control::frame::{OK, code_to_fail};

/// 一面生命周期服务：**树上查回来的门牌** + 它的开者（对端）
/// **它不出编排域**：外面那几枚 `Session` / `Endpoint` / `Receiver` 一个都不露
pub struct Face {
    sender: Sender<Call>,
    host: TaskId,
}

impl Face {
    /// 把一枚门牌收成一面
    /// 对端从**这一枚门闩自己**问出来（establish::opened_by）——门牌是持表那一侧挂的
    /// 不是本端开的
    pub fn of(entry: PieToken) -> Result<Self, Fail> {
        let host = establish::opened_by(entry).ok_or(Fail::Bad)?;
        let sender = Sender::<Call>::from_raw(entry, Call::BACK).map_err(|_| Fail::Bad)?;
        if sender.peer() != host {
            return Err(Fail::Bad);
        }
        Ok(Face { sender, host })
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

    pub fn instance(&self, task: TaskId) -> Instance<'_> {
        Instance { face: self, task }
    }

    /// 查询可信命名程序当前存活的任务。
    pub fn task(&self, name: String, wait: Wait) -> Result<TaskId, Fail> {
        let said = read(self.call(frame::Req::Task(name), wait)?)?;
        if said.a != 0 || said.task == TaskId::new(0) {
            return Err(Fail::Bad);
        }
        Ok(said.task)
    }

    /// 问一句、取一句答
    /// **传输失败折进 Fail::Bad**：借不出回信孔 / 超时 / 答话长度不对——三件事都答
    /// 在**对面**：语义格是持表那一侧真会答的码，"没走到"是本端自己在码表之外判的
    fn call(&self, act: frame::Req, wait: Wait) -> Result<frame::Said, Fail> {
        fn deny(step: &str) -> Fail {
            crate::debug::put(&alloc::format!("control: call deny={step}"));
            Fail::Bad
        }
        self.sender
            .call(Deadline::new(wait), |back| frame::Request(act, back))
            .map_err(|fail| {
                let step = match fail {
                    RpcFail::Open(_) | RpcFail::Grant(_) => "borrow",
                    RpcFail::Encode => "encode",
                    RpcFail::Send(_) => "push",
                    RpcFail::Receive(_) | RpcFail::WrongSource => "receive",
                    RpcFail::Decode | RpcFail::Untrusted => "decode",
                };
                deny(step)
            })
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

    /// **放行 + 等就绪**（有通道的那条顺带逐条认领）⇒ 答一枚 Embarked
    pub fn embark(&self, wait: Wait) -> Result<Embarked<'_>, Fail> {
        let said = self
            .face
            .call(frame::Req::Embark(self.name.clone()), wait)?;
        Ok(Embarked {
            face: self.face,
            name: self.name.clone(),
            task: read(said)?.task,
        })
    }

    pub fn debark(&self, wait: Wait) -> Result<(), Fail> {
        let said = self
            .face
            .call(frame::Req::Debark(self.name.clone()), wait)?;
        read(said).map(|_| ())
    }

    pub fn ruin(&self, wait: Wait) -> Result<(), Fail> {
        let said = self.face.call(frame::Req::Ruin(self.name.clone()), wait)?;
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
/// 要 `debark` / `state` 就 Embarked::service 拿回那一柄
pub struct Embarked<'a> {
    face: &'a Face,
    name: String,
    task: TaskId,
}

impl Embarked<'_> {
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

    /// 回到那一柄（`debark` / `state` 的入口）
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
    match code_to_fail(said.status) {
        None if said.status == OK => Ok(said),
        Some(fail) => Err(fail),
        // 表外那一格（连 `OK` 都没读成）⇒ 与"没走到"同一格。
        None => Err(Fail::Bad),
    }
}

pub struct Instance<'a> {
    face: &'a Face,
    task: TaskId,
}
impl Instance<'_> {
    pub fn embark(&self, wait: Wait) -> Result<(), Fail> {
        read(
            self.face
                .call(frame::Req::EmbarkInstance(self.task), wait)?,
        )
        .map(|_| ())
    }
    pub fn debark(&self, wait: Wait) -> Result<(), Fail> {
        read(
            self.face
                .call(frame::Req::DebarkInstance(self.task), wait)?,
        )
        .map(|_| ())
    }
    pub fn ruin(&self, wait: Wait) -> Result<(), Fail> {
        read(self.face.call(frame::Req::RuinInstance(self.task), wait)?).map(|_| ())
    }
    pub fn state(&self, wait: Wait) -> Result<State, Fail> {
        State::of_code(read(self.face.call(frame::Req::StateInstance(self.task), wait)?)?.a)
            .ok_or(Fail::Bad)
    }
}
