//! operator::door — **门外那一问**（接线那一处）：持树者替树问身份、问谱系、问盟籍，
//! 以及问"第 `n` 格是谁的门牌"。
//!
//! 三件事分居三处，本文件是**接线那一处**：
//!
//! | 处 | 是什么 |
//! |---|---|
//! | [`core::judge`](super::core::judge) | **判据**：`Facts` 那四个问句、[`Ruling`](protocol::system::operator::Ruling) 三格 |
//! | [`core::gate`](super::core::gate) | **裁决**：`verdict`——判据答什么就判成什么，它不做决定 |
//! | 本文件 | **接线**：那两枚门牌（[`Session`]）与"树 → 判据"的那一具（[`Court`]） |
//!
//! # 门牌**问的时候现认**（这一刀）
//!
//! 那两枚门牌是装配者递来**一格号**、由各自那一域**自己** `ship` 进来的
//! （`protocol::system::operator::CoordFrame`）。从前持树者一收到那一帧就**认一次、缓存下来**
//! （`Session` 那一格状态住服务循环那一侧）；今天**认这一手就落在这里**：问到门上才认
//! （[`Session::of`] → [`super::claim::find_face`]）。
//!
//! 换来的形状：**门只有一个调用者**（[`super::answer`]），"门那一侧的状态"不再寄在"路那一侧"
//! 身上。代价（照实）：受门禁的那三条（`find` / `trim` / `land`）**每问一次多两次表扫描**
//! ——与那三问本来就各带的 1s 跨域期限不在一个量级。

use env::{TaskId, Wait};

use protocol::debug;
use protocol::system::coalition::client::Face as CoalitionFace;
use protocol::system::coalition::{CoalitionId, Grant as CoalitionGrant};
use protocol::system::operator::{EntryId, Fail, Permit};
use protocol::system::principal::client::Face as PrincipalFace;
use protocol::system::principal::{Grant as PrincipalGrant, PrincipalId};

use crate::system::operator::core::Operator;
use crate::system::operator::core::gate::{Code, verdict};
use crate::system::operator::core::judge::Facts;

use super::bridge::Coord;
use super::claim::find_face;

/// 问身份那两条边要用的期限（毫秒）。**必须有界**：协调服务不在时不能把树挂死。
const MS: usize = 1000;

/// **两枚门牌**：身份服务那一枚（答"这一位此刻代表谁"与"在不在他那一支里"）与盟册服务那一枚
/// （答"这一位在那枚盟里吗"）。
///
/// 两枚都是装配者**递一格号**、由各自那一域**自己** `ship` 进来的。树**不当自己的客人**：
/// 它不去 `seek("/sys/principal/ask")`，理由同那一笔（自指 ⇒ 环）。
///
/// **盟册那一枚是 `Option`**：它晚到（或压根没配上）时，只有 [`Permit::Among`] 那一格答"判不了"
/// （`Unjudged` 的"会好"那一类——补一帧就好），其余照旧。**降级是诚实的，不是放行**。
struct Session {
    roster: PrincipalFace,
    league: Option<CoalitionFace>,
}

impl Session {
    /// 认出那两枚门牌：**按"谁开的 + 记号"在本表里找**（协调那一帧只带号）。
    ///
    /// 两格都是确定的：那扇门是**各自那一域**开的（副本共享同一事实），记号 = **那一族某一面的
    /// 那枚**（今天要的都是问面）。**不必装配者转授**——各域自己在 `serve_tree` 之后把它直接
    /// 交给持树者。
    ///
    /// 名册那枚是契约：没有它就没有门禁，故它认不出 ⇒ `None`；盟册那枚认不出 ⇒ 只少
    /// [`Permit::Among`] 那一格。
    ///
    /// **两枚记号都要的是"问面"**（开面那两刀）：名册那一族本域要 [`PrincipalGrant::Ask`]
    /// （`Resolve` ＋ `Heir`，两条都是读）——它**做不出** `Adopt`（把一条号领到自己底下）；
    /// 盟册那一族同理要 [`CoalitionGrant::Ask`]（`Amid` ＋ `Band` ＋ `Bloc`）——那枚门牌
    /// **做不出** `Found`（立一枚盟）。两族各交两枚、记号不同，故"要哪一面"必须说清。
    fn of(coord: Coord) -> Option<Session> {
        let roster =
            PrincipalFace::of(find_face(coord.roster?, PrincipalGrant::Ask.mark())?).ok()?;
        let league = coord
            .league
            .and_then(|who| find_face(who, CoalitionGrant::Ask.mark()))
            .and_then(|token| CoalitionFace::of(token).ok());
        Some(Session { roster, league })
    }
}

/// 门禁要的那几条边都从这一份出：问身份（`resolve`）、谱系（`heir`）、盟籍（`amid`），
/// 外加**树自己**那一问（第 `n` 格是谁的门牌）。
///
/// `Session` 只拿两枚门牌，而树不住它里面（`&mut` 那一条借用过不去），故这一格把**两半**
/// 凑在一起——名册/盟册（[`Session`]）＋ 树（[`Operator`]）。
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
        // （`a ≼ b`，就是这一条判据的语义）得把 `b` 当柄、`a` 当参数；
        // `principal(a).contains(b)` 问的是 `b ≼ a`——那是另一条判据，不是这一格。
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
            // 余下三格**到不了**（`core::opens` 的判据表只有上面三条）。一格一格列出来，是为了
            // 将来 `Fail` 多一格时**编不过**，而不是悄悄落进一个 `_`。
            Err(Fail::NonEmpty | Fail::NotAPane | Fail::Full) => Ok(None),
            // **门外那一问答"不"**（终态）：这一位不许。它与上面那三条一样**到不了**
            // （`core::opens` 不过门禁），也**不去** `Unjudged` 那一格：那句话是**确定**的，
            // 而 `Err(())` 是"连有没有都问不到"。故落在 `Ok(None)`（"没有那一位"那一句确定的话）。
            Err(Fail::Denied) => Ok(None),
            // **问不到**：树自己答不出这一问 ⇒ `Err(())`——正是 `Facts::opens` 契约里
            // "树自己问不到"那一格（判据那一侧由它得"判不了"）。同样到不了；两格分开列，
            // 是为了这句话（"不许"与"问不到"不是同一件事）在形状上就分得开。
            Err(Fail::Unjudged) => Err(()),
        }
    }
}

/// **门禁的入口**：那两格还没到（或认不出）⇒ **放行**；否则按那一格自己的许可判
/// （[`Operator::permit`](super::core::Operator::permit) 答出来的那一句）。
///
/// **装配期根本不在门禁这条轴上**：principal 挂自己那两枚门牌那一趟（`part /sys` ＋
/// `part /sys/principal` ＋ 两处 `land`）发生在它的 `serve_tree` 里，而本域**认下它的门牌**与
/// 它**拿到身份**（`derive(ROOT)` + `bind`）都在**那之后** ⇒ 那一刻它**既没有门牌、又还没有
/// 身份**。门禁若在
/// 那一刻生效，它连自己的门牌都挂不上，整机起不来。运行期那些客人则都在 `Hatch` 放行之前拿到
/// 了身份——**挡门的是它们，不是装配这一步**。
///
/// **认不出要报一句，别静默**：号到了而门牌认不出（那扇门不是它开的 / 记号不对）时，门禁会
/// 一直放行，而"为什么"要看得见。
///
/// # 照实记（这一轴在生产里零客人：两次真机普查 ＋ 一张"谁生得出那个号"的表）
///
/// 门禁**在生产里是生效的**（不是只对测具）——临时在那两问上加了读数，把"谁在哪一格上被
/// 门禁看见"整份量了一遍（**读数用完即撤**）：
///
/// ```text
///   /device/router   被 5 位客人取（三台驱动 ＋ guest ＋ lodger）   ⇒ 每格都是"谁都能取"
///   /device/uart/rx  被 2 位（uart 自己那一趟自证 ＋ 控制台那位）
///   /device/rtc      被 2 位（rtc 自己那一趟自证 ＋ sleeper）
///   /sys/control     被 **1 位**：probe-control
///   其余都是 probe_rule 自己那几格（四条变体的正负证）
/// ```
///
/// **第二次普查（许可写进生产，读数一条都不变）**：给 `/sys/principal/set` 临时写上"谁都不许"
/// （`Permit::Trunk(PrincipalId::new(usize::MAX))`）⇒ `derive(set,p)=19` 与 `found(set)=3` 照旧
/// `Ok`、6/6 boot 照旧过。原因：门禁护的是**从树上取这一格**（`find` / `land` / `trim`），而各域
/// 那几枚入口是**在门禁架起之前**由装配者随 `Hatch` 交到手里的（"装配次序即契约"的题中之义）。
///
/// 于是"运行时才取"的格只剩两类，两类都写不出一句真话：
///
/// | 那一格 | 谁在取 | 为什么写不了 |
/// |---|---|---|
/// | 设备格（`/device/*`） | 每格 2～5 位（三台驱动 ＋ 控制台 / 房客 / 客） | **没有一位名字可写**——见下表 |
/// | `/sys/control` | 唯一一位：`probe-control` | 它恰恰是"这一格**取得到**"那条正证本身：把这一格许给装配者，就是改掉那台探针存在的理由，而"control 挂进树、真客人是它"是**裁定过**的一条路 |
///
/// **四种规矩各自要什么号、今天谁生得出**（这一张是全仓读出来的，不是猜的）：
///
/// | 规矩那一格 | 要的号 | 今天谁生得出 |
/// |---|---|---|
/// | [`Permit::Trunk`] / [`Permit::Bough`] | 名册上的身份号 | **只有持树者**（它手里有名册问面，[`Facts::who`] 就是它）——可它**只判不落**；**落格的那几位都没有名册面**：装配者的 [`Roster`](crate::system::principal::bridge::Roster) 只有 `bind` / `adopt`，各驱动的 `Context` 只有**入口 ＋ 树会话** |
/// | [`Permit::Among`] | 盟号 | **生产里没有**：立盟那条 `Found` 零调用者（盟册那一族开两面的普查） |
/// | [`Permit::Opener`] | 某一格的号 | 落格那一位**能**（`bridge::land` 把号答给它）——可"许给开着某格的那位"要那一格的**开者**正好就是那位客人，而生产里没有一格是"只有开者来取"的 |
///
/// **两头对不上，缺的不是格，是"客人"**：格（连它那句规矩）在**装配期**就立好了，而客人**运行时
/// 才出生**；唯一能事后改写的是**格的主人**（`claimable` 只放它），可它手里**没有名册面** ⇒ 它
/// 写不出那位客人的号。⇒ 要这一轴活，得先有一样今天没有的东西：**让客人自己那一格带上它的身份**
/// （把身份挂在一格上），**或**给格的主人一具"谁此刻代表谁"。两条都是新能力，不是收格。
///
/// 故这一轴今天**在探针上四条正负证齐全**（`harness/src/probe_rule.rs` 与
/// `probe_rule_other.rs`：`Trunk` / `Bough` / `Among` / `Opener` 各一正一负），而在**生产里一位
/// 客人也没有**——不是欠账，是"还没有客人说得出一句真话"。
///
/// **照实记（`part` 不过这一关）**：`part` 收的是**坐标**，那一格可能刚存在、可能是一块 `Pane`
/// （[`Permit::Opener`] 对 `Pane` 答"判不了"）——它该过的是「改」那一轴，那一轴已经过了。
pub(super) fn may(
    tree: &Operator,
    coord: Coord,
    who: TaskId,
    permit: Permit,
) -> Code {
    if coord.roster.is_none() {
        return Code::Ok;
    }
    let Some(session) = Session::of(coord) else {
        debug!("operator: door has no face");
        return Code::Ok;
    };
    verdict(&Court { session: &session, tree }, who, permit)
}
