//! operator::door — **门外那一问**（接线那一处）：持树者替树问身份、问谱系、问盟籍，
//! 以及问"第 `n` 格是谁的门牌"。
//! 三件事分居三处，本文件是**接线那一处**：
//! | 处 | 是什么 |
//! |---|---|
//! | [`core::judge`](crate::service::operator::core::judge) | **判据**：`Facts` 那四个问句、[`Ruling`](protocol::service::operator::Ruling) 三格 |
//! | [`core::gate`](crate::service::operator::core::gate) | **裁决**：`verdict`——判据答什么就判成什么，它不做决定 |
//! | 本文件 | **接线**：那两枚门牌（[`Session`]）与"树 → 判据"的那一具（[`Court`]） |

use env::{TaskId, Wait};

use protocol::debug;
use protocol::service::coalition::client::Face as CoalitionFace;
use protocol::service::coalition::{CoalitionId, Grant as CoalitionGrant};
use protocol::service::operator::{EntryId, Fail, Permit};
use protocol::service::principal::client::Face as PrincipalFace;
use protocol::service::principal::{Grant as PrincipalGrant, PrincipalId};

use crate::service::operator::core::Operator;
use crate::service::operator::core::gate::{Code, verdict};
use crate::service::operator::core::judge::Facts;

use crate::service::operator::claim::face_of_mark;

/// 问身份那两条边要用的期限（毫秒）。**必须有界**：协调服务不在时不能把树挂死。
const MS: usize = 1000;

/// **两枚门牌**：身份服务那一枚（答"这一位此刻代表谁"与"在不在他那一支里"）与盟册服务那一枚
/// （答"这一位在那枚盟里吗"）。
/// 两枚都是装配者**递一格号**、由各自那一域**自己** `ship` 进来的。树**不当自己的客人**：
/// 它不去 `seek("/svc/sys/principal/ask")`，理由同那一笔（自指 ⇒ 环）。
/// **盟册那一枚是 `Option`**：它晚到（或压根没配上）时，只有 [`Permit::Among`] 那一格答"判不了"
/// （`Unjudged` 的"会好"那一类——补一帧就好），其余照旧。**降级是诚实的，不是放行**。
struct Session {
    roster: PrincipalFace,
    league: Option<CoalitionFace>,
}

impl Session {
    /// 认出那两枚门牌：**按记号在本表里找**（那两枚由各自那一域自己交进来）。
    /// 记号 = **那一族某一面那一枚**（今天要的都是问面）；那一族**只有一家生产者**，故记号
    fn of() -> Option<Session> {
        let roster = PrincipalFace::of(face_of_mark(PrincipalGrant::Ask.mark())?).ok()?;
        let league = face_of_mark(CoalitionGrant::Ask.mark())
            .and_then(|token| CoalitionFace::of(token).ok());
        Some(Session { roster, league })
    }
}

/// 门禁要的那几条边都从这一份出：问身份（`resolve`）、谱系（`heir`）、盟籍（`amid`），
/// 外加**树自己**那一问（第 `n` 格是谁的门牌）。
struct Court<'a> {
    session: &'a Session,
    tree: &'a Operator,
}

impl Facts for Court<'_> {
    fn who(&self, tid: TaskId) -> Result<Option<PrincipalId>, ()> {
        match self.session.roster.task(tid).principal(Wait::AtMost(MS)) {
            Ok(found) => Ok(found.map(|p| p.id())),
            Err(_) => Err(()),
        }
    }

    fn heir(&self, a: PrincipalId, b: PrincipalId) -> Result<bool, ()> {
        // **柄与参数的方向**：`contains` 发的是 `heir(参数, self)`，故要问 `heir(a, b)`
        // 得把 `b` 当柄、`a` 当参数；
        self.session
            .roster
            .principal(b)
            .contains(a, Wait::AtMost(MS))
            .map_err(|_| ())
    }

    fn amid(&self, me: PrincipalId, at: CoalitionId) -> Result<bool, ()> {
        // 盟册那一枚没在手里 ⇒ 答"问不到"，而不是答"否"——**判不了**与"不在那枚盟里"是
        // 两件事，后者会让客人当场放弃。
        let Some(league) = self.session.league.as_ref() else {
            return Err(());
        };
        league
            .coalition(at)
            .holds(me, Wait::AtMost(MS))
            .map_err(|_| ())
    }

    fn opens(&self, at: EntryId) -> Result<Option<TaskId>, ()> {
        // **判据要的只有"有没有那一位"**（见 `Facts::opens`），故三种"没有"在裁决那一侧同落
        // `Ok(None)`。这一条**不动树**：剔死是 `find` 的活儿。
        match self.tree.opens(at) {
            Ok(tid) => Ok(Some(tid)),
            // 碑 / 从没铸过：**永久**。
            Err(Fail::Unknown) => {
                debug!("operator: opens gone n={}", at.get());
                Ok(None)
            }
            // 那一号是块窗格：**永久**（结构事实）。
            Err(Fail::NotATile) => {
                debug!("operator: opens pane n={}", at.get());
                Ok(None)
            }
            // 开者答不出——**永久**。`Dead` 自己仍是**三因一码**（不是孔 / 不在我表里 / 已封印）：
            // 这一行读数是"没有开者"，不声称分得开那三因。
            Err(Fail::Dead) => {
                debug!("operator: opens sealed n={}", at.get());
                Ok(None)
            }
            // 余下三格**到不了**。一格一格列出来，是为了
            // 将来 `Fail` 多一格时**编不过**，而不是悄悄落进一个 `_`。
            Err(Fail::NonEmpty | Fail::NotAPane | Fail::Full) => Ok(None),
            // **门外那一问答"不"**（终态）：这一位不许。它与上面那三条一样**到不了**
            // （`core::opens` 不过门禁），也**不去** `Unjudged` 那一格：那句话是**确定**的，
            // 而 `Err(())` 是"连有没有都问不到"。故落在 `Ok(None)`（"没有那一位"那一句确定的话）。
            Err(Fail::Denied) => Ok(None),
            // **问不到**：树自己答不出这一问 ⇒ `Err(())`——正是 `Facts::opens` 契约里
            // "树自己问不到"那一格。同样到不了；两格分开列，
            // 是为了这句话（"不许"与"问不到"不是同一件事）在形状上就分得开。
            Err(Fail::Unjudged) => Err(()),
        }
    }
}

/// **门禁的入口**：那两格还没到（或认不出）⇒ **放行**；否则按那一格自己的许可判
/// （[`Operator::permit`](crate::service::operator::core::Operator::permit) 答出来的那一句）。
/// **装配期根本不在门禁这条轴上**：principal 挂自己那两枚门牌那一趟（`part /svc` ＋
/// `part /svc/sys/principal` ＋ 两处 `land`）发生在它自己的 `serve()` 里，而本域**认下它的门牌**与
/// 它**拿到身份**（`derive(ROOT)` + `bind`）都在**那之后** ⇒ 那一刻它**既没有门牌、又还没有
/// 身份**。门禁若在
pub(super) fn may(tree: &Operator, wired: bool, who: TaskId, permit: Permit) -> Code {
    // **门禁先决两格**：
    // 一、**装配者有没有说"接线完成"**——它认下名册那一刻才推那一句（[`crate::service::operator::bridge::Tree::wire`]
    if !wired {
        return Code::Ok;
    }
    let Some(session) = Session::of() else {
        debug!("operator: door has no face");
        return Code::Ok;
    };
    // **如实记（量过：这一问不是那一秒的病根）**：它几问句各带 `Wait::AtMost(MS = 1000)`，故一度
    // 是"装配期那位客人等了 1.1~1.2 s"的头号嫌疑。debug 档 11 跑里挂了 `operator: door ms=` 一
    // 行去量它——**一行都没落**（五跑出现 stall 的那些跑里，这一问每一趟都在 200 ms 门槛之下）
    verdict(
        &Court {
            session: &session,
            tree,
        },
        who,
        permit,
    )
}
