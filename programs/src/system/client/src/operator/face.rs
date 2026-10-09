//! Operator会话与命名树的语义操作。

use alloc::string::String;

use super::handoff;
use env::{PieToken, TaskId, Wait};

use crate::operator as ocall;
use crate::operator::Path;
use crate::operator::{EntryId, Fail, Listing, Permit, Where};
use ipc::session::{Berth, Session};
use ipc::time::{deadline, remain};
use system_api::operator::Call;

use super::{Pane, Tile, Watch};
/// **这条路叫什么**：泊位那一格（`LINK` = `operator`）＋ 问话孔那一格（`ASK_MARK`）
/// 开会话那一手（Session::open）要它；本层只把这两格交出去，不替调用方开会话
pub const BERTH: Berth = Berth {
    link: super::marks::LINK_MARK,
    ask: crate::operator::ASK_MARK,
};

/// 门牌那一格声不声明归属（Pane::bind 的最后一格）
#[derive(Clone, Copy)]
pub enum Mine {
    Yes,
    /// 不声明归属
    No,
}

/// 共享调用状态和端点寿命的命名树会话。
pub struct Face {
    session: Session,
}

impl Face {
    /// 把一条装好的会话收成一面（`Session::open(sire, BERTH, wait)` 装它）
    pub fn of(session: Session) -> Self {
        Face { session }
    }

    /// 借用同一条会话；别名共享调用状态，端点仍归原任务。
    pub fn from(session: &Session) -> Self {
        Face {
            session: session.clone(),
        }
    }

    /// 对端是谁（持树者的号；读数用）
    pub fn host(&self) -> TaskId {
        self.session.host()
    }

    /// 这棵树的**根**（`/`）
    /// **它线上是独立那一形**（Where::Root：`tag = 0`），不是"零号那一格"——根没有号
    /// 而零号是真格子（`/svc`）。故柄里存的是 Where，不是一枚 EntryId
    pub fn root(&self) -> Pane<'_> {
        Pane {
            face: self,
            at: Where::Root,
        }
    }

    /// 一条**从根出发**的路 → 一块窗格（带退避重试：路那几格可能由别的域落下）
    /// 那一格是一枚砖 ⇒ Fail::NotAPane（`list` 自己那一问就判了）
    pub fn pane(&self, road: &Path, wait: Wait) -> Result<Pane<'_>, Fail> {
        let id = road_to_id(&self.session, road, wait)?;
        Pane::at(self, id, wait)
    }

    /// 一条**从根出发**的路 → 一格（带退避重试：路那几格可能由别的域落下）
    /// **它只译号，不顺手取那一枚门闩**：要门闩的是 Tile::token，那是另一问
    /// （`find` 会动树、还会把那一枚授进来）。多问一趟既费一次往返、又会让"路译得出"这一件
    /// 已经成的事背着另一个失败
    /// （对一块 `Pane` 答 Fail::NotATile）。`Tile` 因此是"**可以当砖用**的一格"，不是
    /// "已被验明是砖"——要一个当场就验的入口，用 Face::pane（它那一问 `list` 会答
    /// **四面入口的分工**（`Face` 两格带重试，`Pane` 两格不带）
    /// "我手里已经有一条好路"那一形。要哪一形由调用点的处境说，不由默认值兜
    pub fn tile(&self, road: &Path, wait: Wait) -> Result<Tile<'_>, Fail> {
        let id = road_to_id(&self.session, road, wait)?;
        Ok(Tile { face: self, id })
    }

    /// 问一句、收一句（本面那枚问话孔 ＋ 本端这条树路）。
    pub(super) fn call(&self, ask: ocall::Req, wait: Wait) -> Result<ocall::Said, Fail> {
        self.session
            .call::<Call>(ask, wait)
            .map_err(|_| Fail::Unknown)
    }

    /// **分**：在 `at` 那一块 `Pane` 里给 `name` 放一块空窗格；答那一格自己的号
    /// 判据与 Pane::open 同一句（幂等 / 是一枚砖 ⇒ Fail::NotAPane / 装不下 ⇒
    pub fn part(&self, at: Where, name: String, wait: Wait) -> Result<EntryId, Fail> {
        let said = self.call(ocall::Req::Part { at, name }, wait)?;
        said.entry().map_err(map_code)
    }

    /// 在容器中发布能力；明确拒绝时撤回本次转授。
    pub fn land(
        &self,
        at: Where,
        name: String,
        entry: PieToken,
        permit: Permit,
        mine: Mine,
        wait: Wait,
    ) -> Result<EntryId, Fail> {
        let said = handoff::offer(&entry, self.session.host(), |shipped| {
            self.session.call::<Call>(
                ocall::Req::Land {
                    at,
                    name,
                    entry: shipped,
                    permit,
                    mine: matches!(mine, Mine::Yes),
                },
                wait,
            )
        })
        .map_err(|_| Fail::Unknown)?;
        said.entry().map_err(map_code)
    }

    /// **寻**：把 `id` 那一号后面那一枚 Pie 要过来（经会话授进本端表，号随答话回来）
    /// **它是最敏感的一格**：这一步会**转移能力**（`find` 那一枚带 `VEST`，见
    /// operator 的"交出去的权柄收不回来"）。故 `find` 自成一位，不与
    pub fn find(&self, id: EntryId, wait: Wait) -> Result<PieToken, Fail> {
        let said = self.call(ocall::Req::Find(id), wait)?;
        said.seed().map_err(map_code)
    }

    /// **剪**：把 `id` 那一号剪掉
    pub fn trim(&self, id: EntryId, wait: Wait) -> Result<(), Fail> {
        let said = self.call(ocall::Req::Trim(id), wait)?;
        match said.code() {
            ocall::OK => Ok(()),
            code => Err(map_code(code)),
        }
    }

    /// **列**：`at` 那一块里有哪些号
    pub fn list(&self, at: Where, wait: Wait) -> Result<Listing, Fail> {
        let said = self.call(ocall::Req::List(at), wait)?;
        said.list().map_err(map_code)
    }

    /// **译**：一条路（**从根写起**）译成号。与 Pane::tile 同一条腿，只是面不同
    pub fn seek(&self, road: &Path, wait: Wait) -> Result<EntryId, Fail> {
        let said = self.call(ocall::Req::Road(road.to_path_buf()), wait)?;
        said.entry().map_err(map_code)
    }

    /// **名**：`id` 那一号此刻叫什么
    pub fn name(&self, id: EntryId, wait: Wait) -> Result<String, Fail> {
        let said = self.call(ocall::Req::Name(id), wait)?;
        said.name().map_err(map_code)
    }

    /// **看**：订 `road` 这条子树，此后树上真变了就往本端那一页里记一条。
    ///
    pub fn watch(&self, road: &Path, wait: Wait) -> Result<Watch<'_>, Fail> {
        Watch::of(self, road, wait)
    }
}

fn road_to_id(session: &Session, road: &Path, wait: Wait) -> Result<EntryId, Fail> {
    let until = deadline(wait);
    super::lookup::until(|| route(session, road, remain(until)), |backoff| {
        if remain(until) == Wait::POLL { return false; }
        let _ = execution::room::park(core::time::Duration::from_millis(backoff as u64));
        remain(until) != Wait::POLL
    })
}
fn route(session: &Session, road: &Path, wait: Wait) -> Result<EntryId, super::lookup::Failure> {
    use super::lookup::Failure;
    let said = session.call::<Call>(ocall::Req::Road(road.to_path_buf()), wait)
        .map_err(|_| Failure::Stop(Fail::Unknown))?;
    if said.failure_status() == Some(ocall::UNKNOWN) { return Err(Failure::Missing); }
    said.entry().map_err(|code| Failure::Stop(map_code(code)))
}

/// 线上那一格码 → 失败域；表外（含 `BAD`）折 Fail::Unknown
/// 这是**唯一**一处 `u8 → Fail`：`Said` 那几个读法答的是 wire 那一层的码，本族在客侧把它们
/// 收进语义那一格
pub(super) fn map_code(code: u8) -> Fail {
    ocall::code_to_fail(code).unwrap_or(Fail::Unknown)
}
