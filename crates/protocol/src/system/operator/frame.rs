//! operator 的**帧那一半** —— 帧、码、记号（内核那几只手的别名与适配在 `protocol` 那一侧的 `mod.rs`）。
//!
//! **照实记（这一份为什么拆出来）**：帧形今天只有机器在跑，而机器只走**顺路**——边角
//! （短帧 / 长帧 / 动作码不对 / 那一串号的条数对不上 / 表外的码）一格都走不到。拆开是为了让那些
//! 边角在**宿主靶**上编得动；**那台靶已删**（用户裁定"protocol-case 没必要"）⇒ 这一份照旧只认
//! `env` 与同层 `core`/`judge`，而那些
//! 边角今天**没有判据**；适配那半留在 `protocol` 那一侧的 `mod.rs`（今天的形状：**只有身体、
//! 没有壳**——要哪一手直接叫 [`crate::communication::establish`]），建立那一手的失败域映射
//! 与判据同一份屋顶（**裁决与判据已回实现侧**：见 `programs/src/system/operator/core/`）。
//!
//! 本文件**不做裁决**：树上的规矩（谁能落、谁许改、什么时候剔死）全在实现侧
//! （`programs/src/system/operator/core/`）。
//! 这里只有三件事——**编一帧 / 解一帧**、把"不在我表里"翻成 `None`、把失败域翻成答话码。
//!
//! 判据只有一条可机械检查的纪律——
//!
//! > 本文件里的 `if` / `match` 一处裁决也没有，只有两张对照表（失败域 ↔ 答话码、
//! > 会话失败域 ↔ 板失败域）与编解码。
//!
//! # 帧
//!
//! ```text
//!   Req    Road   [0] op  [1] 段数  [2 .. 2+32k] 路                （k ≤ Path::MAX）
//!          List   [0] op  [1] 记    [2 .. 10]     号              （记：0 = 根 / 1 = 号）
//!          Part   [0] op  [1] 记    [2 .. 10]     号  [10 .. 42] 名
//!          Land   [0] op  [1] 记    [2 .. 10] 号 [10 .. 42] 名 [42 .. 50] 尾格
//!                 [50] 改   [51] 用   [52 .. 60] 号
//!          Find   [0] op  [1 .. 9] 号
//!          Trim   同 Find
//!          Name   同 Find
//!   Union    [0] status                                    —— 一格的答
//!          [0] status   [1] 条数   [2 ..] 号             —— 列
//!          [0] status   [1 ..] 名字                       —— 名（长度即名长）
//!          [0] status   [1 .. 9] 号                       —— 号（`land` / `part` / `seek`，定长 9）
//! ```
//!
//! **答那一侧四种形状在线上分不开**（都以状态那一格起头，而"名"那一条是变长的：**长度即
//! 名长**）⇒ 收进来的那一面是**原样的字节**（[`Said`]），由**问的人**按自己问的那一条读；
//! 编的那一面是 [`Union`]（五种编法：一格状态 / 一串号 / 一枚名字 / 坐标 / 门闩）。
//!
//! **问话一个动作一条形状**（不再是"一帧定长、尾格含义由 op 定"）：荷载收什么，帧里就写什么
//! ——没有一个"报法"字段可以填错，也没有第二个意思可读。最长的仍是 `Road` 那一条
//! （[`REQ_LEN`]，路封顶 [`Path::MAX`] 段），其余都落在十到五十字节。
//!
//! **每一张形状一张字段表**（[`RoadFrame`] / [`List`] / [`Part`] / [`Land`] / [`Entry`]）：
//! 偏移一处都不写。**照实记（表名的口径收窄了一次）**：板那一族的表按**荷载**起名（那一族
//! 有两个动作共用一张）；这一族**一条问一张表**，只有那三条只报号的（`find` / `trim` /
//! `name`）共用——那一张按荷载叫 [`Entry`]（它是唯一一处"两个名字落在同一张表上"）。
//!
//! **变长那一条只有 `Road`**：它那一截由 [`Path`] 自己给（`env::wire::Span` 那两只手：
//! 段数那一格 ＋ 那几段），回表之后 [`RoadFrame`] 就是"动作码 ＋ 路"一张表——**族里没有
//! `2 + i * 32` 这种句子**（用户裁定：尾巴不许手写），**段数那一格也不再住本文件**（它随
//! [`Path`] 走）。
//!
//! **尾格只剩 `land` 用**：入口那一枚经会话交出去（`ship` 换回来的那个号，不是"客人的 Pie
//! 是几号"），报文里走的只是"种在持树者表里的号"。两个编号空间不同源，互相拿错正是旧树
//! `[33..41]` 那一格的病。
//!
//! **答话有四种形状、各有各的上界**，本族那只缓冲按 [`UNION_LEN`] 备（最大那一形）。

use alloc::string::String;
use env::Mark;
use env::{PieToken, TaskId};

use super::path::{Path, PathBuf};
use crate::service::coalition::CoalitionId;
use crate::system::principal::PrincipalId;

// **照实记（宽度别名 `Id` 已退场）**：从前本文件有一条 `pub type Id = u64`，给判据那一侧当
// `PrincipalId` / `CoalitionId` 的**宽度替身**——那时帧不认识那两个号。今天 [`Permit`] 的各格
// 直接带**真类型**（`PrincipalId` / `CoalitionId` / `EntryId`），那条别名一个读者都没有了，故删。
use crate::id::Id as _;
use crate::message::Message;

// ── 两条容量（原先挂在 `Operator` 上）────────────────────────
//
// **照实记**：它们原先是 `Operator` 的关联常量（`Operator::PANE_CAP` / `Operator::ROAD_MAX`；后者今天住 [`Path::MAX`]）。
// 账搬回实现侧之后，**帧长要按它们算**——故容量归协议（线格式的一部分），账去读它。

/// 一枚条目的**号**：机器用的那一个。
///
/// **裸号**：与 [`PrincipalId`](crate::system::principal::PrincipalId) / [`CoalitionId`](crate::service::coalition::CoalitionId)
/// 同形（8 字节小端上线），不同源。线上解码面造得出任何号（[`EntryId::new`]），
/// "这枚号还在不在"由每条读**查一次表**答出来。
///
/// **没有 `ROOT`**（对照另两种号：那两处的 `ROOT` 都在，这里特意没有）：根不是谁条目里的
/// 一条，故**根没有号**——`EntryId(0)` 是第一个**真格子**（`sys`），不是"没有"。
/// "没有这个号"由 [`Fail::Unknown`] 答，别拿 0 当空。根要当坐标时走 [`Where::Root`]。
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct EntryId(usize);

impl EntryId {
    /// 由裸号造一个（线上解码面；已失效的号从这里进来）。
    pub const fn new(raw: usize) -> EntryId {
        EntryId(raw)
    }

    /// 裸号。
    pub const fn get(self) -> usize {
        self.0
    }
}

impl crate::id::Id for EntryId {
    fn new(raw: usize) -> EntryId {
        EntryId::new(raw)
    }

    fn get(self) -> usize {
        EntryId::get(self)
    }
}

/// **号那一格线上是 8 字节小端**——与 [`crate::id::Id`] 给三条号空间定的同一条规则（那一条 trait
/// 的 `to_bytes` / `from_bytes` 就是这一格的正文）。
///
/// **照实记（impl 为什么住这一处，不住 `env::wire`）**：impl 跟着类型走——`env` 不认识
/// [`EntryId`]（依赖是单向的 `protocol → env`），故宽度与字节序只能由定义它的这一处给。
/// 口径与 `env::wire` 里那几个 impl 相同（`Field` 那一族的正文记着）。
///
/// 读的那一侧**不校验"还在不在"**（[`crate::id::Id::from_bytes`] 的注）：解出来的号在不在表里
/// 由核心答（[`Fail::Unknown`]）。
impl env::wire::Field for EntryId {
    const WIDTH: usize = 8;
    fn store(&self, out: &mut [u8]) {
        out.copy_from_slice(&self.to_bytes());
    }
    fn fetch(bytes: &[u8]) -> Option<Self> {
        Some(Self::from_bytes(bytes.get(..8)?.try_into().ok()?))
    }
}

/// 一块 `Pane` 里最多几条。条数是策略，容器要有界。
pub const PANE_CAP: usize = 16;

// **照实记（`ROAD_MAX` 这一格退了）**：那条"最多几段"从前是这一族的一枚自由常量，而它的
// 三个读者（段数那一格、`seek` 的上限判据、提示之路那一形）今天都归 [`Path`]：上限住
// [`Path::MAX`]，段数住 [`Path`] 自己，判据由带它的那张表（[`RoadFrame`] / [`PlateFrame`]）
// 一次说完（超长 ⇒ 读不懂）。
// **树的深度不受这条路的长短约束**（`land` / `part` 收的是号，层层往下立与路无关）。

// ── 号在模型里的宽度 ────────────────────────────────────────
//
// **照实记（`pub type Id = u64` 已删）**：这一格原先是一条宽度别名，理由是"帧与判据只认识这一
// 格、不认识 `PrincipalId` / `CoalitionId`（那两个号是泛型的 `P` / `C`）"。`Permit` 带真类型之后
// 那条理由作废，别名随之一个读者都不剩。宽度本身仍写在 [`crate::id::Id`] 那三条 `to_bytes` /
// `from_bytes` 里（8 字节小端，就是各号自己的 `Field` 那一格）。

/// **容器坐标**：要动的那一块 `Pane` 在哪。
///
/// 两种报法：**根**，或**某一号**。根必须显式占一格——**根没有号**（见 [`EntryId`]），
/// 所以它既不是"0 号"，也不能拿 `Option` 的空位代替：那两样都会被读成"某个真格子"。
///
/// 它的对立面是 [`Operator::find`] / [`Operator::trim`] / [`Operator::name`] 的形参：
/// 那三条要的是**条目**的号，**根根本递不进来**——这是类型义务，不是运行期检查。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Where {
    /// 根那一层：[`Operator::list`] 列的就是它，`land` / `part` 在它下面立一格。
    Root,
    /// 某一号那一块 `Pane` 里。
    At(EntryId),
}

/// 容器坐标那一格的"记"：`0` = 根、`1` = 号（[`Where`] 两种报法在线上的样子）。
///
/// **照实记（它们为什么从 `frame.rs` 搬到这儿）**：这两个数是**这一格自己的编码**
/// （"根"与"某一号"怎么落在字节上），与"哪一帧用得上它"无关——`Field` 那一族的口径是
/// **impl 跟着类型走**，故记也跟着类型走。
const AT_ROOT: u8 = 0;
const AT_ID: u8 = 1;

/// **容器坐标那一格是"记 ＋ 号"**（9 字节）：`0` = 根（后面 8 字节**照写零**）、`1` = 某一号。
///
/// **根为什么占一格、而不是省掉**：字段表要的是"这一格占多宽"（定长），省了就得再想"读到哪儿
/// 算数"；而根**没有号**（见 [`EntryId`]），不能拿零号代替——那会被读成"某个真格子"。
///
/// **表外的记 ⇒ 整帧读不懂**：`0` / `1` 之外的记不是任何一种坐标，`fetch` 答 `None`
/// （与从前那一手 `unpack_at` 同款：不猜、不崩）。
impl env::wire::Field for Where {
    const WIDTH: usize = 1 + <EntryId as env::wire::Field>::WIDTH;

    fn store(&self, out: &mut [u8]) {
        let (tag, id) = match *self {
            Where::Root => (AT_ROOT, EntryId::new(0)),
            Where::At(id) => (AT_ID, id),
        };
        out[0] = tag;
        // 长度恰是 `WIDTH`（`Field::store` 的契约）⇒ 记之后那一段正好是号那一格。
        id.store(&mut out[1..]);
    }

    fn fetch(bytes: &[u8]) -> Option<Self> {
        match *bytes.first()? {
            AT_ROOT => Some(Where::Root),
            AT_ID => Some(Where::At(EntryId::fetch(bytes.get(1..)?)?)),
            _ => None,
        }
    }
}

/// 八条原语会失败在哪一格。**一格对应一个不同的下一步**。
///
/// **没有"名字已被占"那一格**：同名接手一枚 `Tile`、或一块**空的** `Pane`，都是换绑
/// （见 [`Operator::land`] / [`Operator::part`]）；而 owner 归 Principal，Operator 分不出
/// "自己 / 别人"，所以"已占即拒"在这里无处落脚。
///
/// **后两格（[`Fail::Denied`] / [`Fail::Unjudged`]）来自门外那一问**：核心一个字节都不知道
/// 它们（判据住实现侧，见 [`DENIED`] / [`UNJUDGED`]），但它们同样是**客侧要按下一步区分**的
/// 答案 ⇒ 与前面六格同住这一枚类型。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fail {
    /// 那一号/那一格不在树上 ⇒ 换个名字重来，或者先把中间那一层分出来。
    ///
    /// 三条路都走这一格：**没铸过**、`trim` **剪掉了**、[`Operator::find`] **剔死了**
    /// ——表里留着一个墓碑（`None`），但墓碑**不对外答"我这儿死过"**：三条路长得一样。
    /// 空路（根）走 [`Operator::seek`] 时也是这一格。
    Unknown,
    /// 那块 `Pane` 里还有东西，而这一手会**毁掉**里面的 ⇒ 先清空。
    ///
    /// 今天只有两条原语走得到它：[`Operator::land`] 的换绑（要把那块非空 `Pane` 换成砖）与
    /// [`Operator::trim`]（要拿走它）。**`part` 不走这一格**——它要的正是那块 `Pane`，
    /// 已经在就是成了（照实记见 [`Operator::part`] 的注）。
    NonEmpty,
    /// 寻到头是一块 `Pane`，不是一枚 `Tile` ⇒ 改用列，或者往它里面走。
    NotATile,
    /// 那一号不是一块 `Pane`（是一枚 `Tile`）⇒ 走不进去；列的时候则说明"那是枚 `Tile`，没什么可列"。
    NotAPane,
    /// 那一块 `Pane` 已经 [`PANE_CAP`] 条，装不下 ⇒ 拆层 / 扩容量。
    ///
    /// **照实记（"路太长"那一半退了）**：这一格从前还有一个来源——`seek` 收的一条路超过
    /// `ROAD_MAX` 段。今天一条路是 [`Path`]（最多 [`Path::MAX`] 段）⇒ **超长根本表达不出来**
    /// （那一帧当场"读不懂" ⇒ 答 [`BAD`]）⇒ 这一格只剩"装不下"一个读者。
    Full,
    /// 那枚 Pie 后面的人没了（探不到）⇒ 重落 / 重寻。**剔掉那一条的同时**答这一格。
    Dead,
    /// 门外那一问答"不"：[`DENIED`] ——这一位不许动这一格。**终态**：换人 / 换目标，别重试。
    Denied,
    /// 门外那一问答"判不了"：[`UNJUDGED`] ——要问的那条事实问不到。
    ///
    /// **它不承诺"等一会儿会好"**：对面不答 / 超时（会好），与那一号是碑 / 那一格是块窗格 /
    /// 开者那扇门封印了（好不了）都落这一格；分开它们的**是读数，不是第三格码**。
    Unjudged,
}

// ── 一格规则 ────────────────────────────────────────────────

/// **这一格谁许用**。四格覆盖"就是某一位 / 在某一位那一支里 / 在某枚盟里 /
/// 就是开着某一格的那一位"；[`Permit::Unset`] = 没有许可（那一格没记过）。
///
/// `By`（落牌那一位）**不进这一格**：规则改不改由它说了算（判据在适配层），而"谁能改规则"
/// 与"谁能用这一格"是两个问题——混成一格就会得出"能改的人自然能用"。
///
/// **`Opener` 那一格是"点名那一手"**：前三格只能指到"自己人"（自己的号、自己那一支、自己在的
/// 盟），而 `Opener` 指的是一格**门牌**——客人用 [`seek`](super::Operator::seek) 把一条路
/// 译成号，再把那个号写进规矩，于是「把这一格许给 `/svc/drv/uart` 那位」写得出来。名字由树
/// 提供（**树就是名录**），故规矩里存的是**格号**，不是身份号：判的那一刻才去问"此刻谁占着
/// 那一格"（晚绑定，与 [`Permit::Among`] 同一形状——存一枚盟号，成员现场问）。
///
/// 照实记：**号不重用**（`core.rs` 只增水位）⇒ 那一格被剪/被顶之后，这一条规矩**永久判不了**
/// （重挂是**新号**）。这是"此刻占着这一格的那位"的题中之义，不是缺陷；要"换载体规矩不变"
/// 就得给身份起名字（那是另一条路，今天没有客人要它）。
///
/// ⇒ **这一格的寿命 = 那一格的寿命**：与 [`Permit::Trunk`] / [`Permit::Bough`]（绑在**身份**上、
/// 活到会话结束）不同，它绑在**一次挂载**上。作废之后判出来的是 [`Ruling::Unjudged`]
/// （"好不了"的那一类）——要修的是**写这条规矩的主人**（重 `land` 一次），客人换目标没用。
///
/// **这一格是从四条路里挑的**（要补的那句话是「许给 `/svc/drv/uart` 那位」）：① 名册带名字
/// （仓里从此**两套名字**，要对齐重名 / 改名 / 谁有权命名）；② **树当名录**（本格——客人
/// `seek` 出号、写进规矩，线上仍是 8 字节）；③ 装配表 args（只到装配期，且号是 `derive(ROOT)`
/// 的顺序产物 ⇒ 加一条服务全表错位）；④ 不造机制（那句话仍然说不出来）。**被否的第五条路**
/// 是"规矩里直接写一条路"——线上装不下（`REQ_LEN` 258 减 `land` 用掉的 60 只剩 198，而一条路
/// 最多 8 × 32 = 256），且它违反已定的「号是唯一的直接坐标」。
///
/// # 这一轴**封顶**（用户裁定）
///
/// 上面四格是这一轴的**完备集**：没有组合（合取 / 析取 / 否定），也没有「这一位是什么」
/// 这一族谓词。三条理由，前两条是数不是偏好：
///
/// - **装不下**：线上这一段是 `tag(1) + 号(8)`，而**号那一格只有一格**（[`Permit`] 的 `Field`
///   那一格）。任何"两句合起来"立刻要第二格号，而 51 → 60 是**纯追加**换来的兼容性。
/// - **问次数长在串行的持树者身上**：今天最坏 [`Permit::Opener`] = **三问**（名册 → 树 → 名册），
///   其中两次跨域、各带 1s 期限；而持树者是一枚线程——真机量过：一位客人连打约 1030 手同步
///   往返，别人的三手（`name` / `trim` / `list`）连着 1 秒过期。组合让每一次 `find` 的嵌套
///   问答**随深度增长**。
/// - **组合要的不是新变体，是一套三值代数**：[`judge`] 里 `Ok(false)`（"不是" ⇒ 终态拒）与
///   `Err`（"问不到" ⇒ 判不了）是**两件事**；合取得先定义谁压过谁、要不要短路。那是新维度。
///
/// **要加第五格，得同时有三样**：一位真客人 + 一句它说得出的原话（不是"将来可能"）+
/// 那三问的答案（装在哪一格 / 判一次问几次 / 写完谁读得回）。三样缺一 ⇒ 不加。
///
/// **封顶不等于四格都好判**：[`Permit::Among`] 与 [`Permit::Opener`] 都是"引用 + 现场求解"，而**只有
/// `Opener` 的引用对象会死**（上一段）⇒ 这个封闭集里**存在"永远判不了"的一格**——它与"对面
/// 暂时不答"同落 [`Ruling::Unjudged`]（两类同格是那一格自己的口径：客人那一侧同一步，差别由
/// 持树者各说一行读数分开，见 [`Ruling::Unjudged`] 与 [`Facts::opens`]）。
///
/// # 这一轴今天谁在用（两次真机普查，读数的落点在 `programs/src/system/operator/door.rs`）
///
/// **四格 ＋ [`Permit::Unset`] 在探针上都有真机读数**：`harness/src/probe_rule.rs` 一台客人演两个
/// 身份——`Trunk` / `Bough` / `Among` / `Opener` 各一正一负，第一格（`Unset`，"只判有没有身份"）
/// 也有正证（换了代表那位照样过）；`harness/src/probe_rule_other.rs` 量同四格在**别人**手里那一侧。
///
/// **生产里的选择者：先是零，今天三位**——`/svc/sys/control/{mint,start,stop}` 三格各带一句
/// `Permit::Trunk(PrincipalId::ROOT)`（"许给根"）：`/svc/sys/control` 拆成四面之后，规矩落在**定面**
/// 那三格上，**问面**（`state`）照旧公开（读数见 `harness/src/probe_control.rs`）。
/// 其余各处落格仍递 `Permit::Unset`。两次普查量到：
///
/// - 门禁在生产里**确实生效**（盟册与三台驱动的落格、登记都过了判定，不是只对测具）；
/// - 可**"运行时才去取"的格只有两类**：设备格（每格 2～5 位客人，**没有一位名字写得出来**）与
///   `/svc/sys/control`——后者**已经收上了**（拆成四面：问面公开、`mint` / `start` / `stop` 三面各带
///   `Trunk(ROOT)`）；其余各域的入口都是**在门禁架起之前**由装配者随 `Hatch` 交到手里的
///   （"装配次序即契约"）⇒ 给它们写许可，读数**一条都不会变**（量过：给 `/svc/sys/principal/set`
///   写"谁都不许"，`derive(set,p)` 照旧成）。
///
/// **要它活，缺的是客人，不是格**：格（连它那句规矩）装配期就立好了，而客人**运行时才出生**；
/// 唯一能事后改写的是**格的主人**（`claimable` 只放它），而它手里**没有名录面**——装配者的
/// `Roster` 只有 `bind` / `adopt`，各驱动的 `Context` 只有**入口 ＋ 树会话**。两条出路都是
/// **新能力**：让客人自己那一格带上它的身份，**或**给格的主人一具"谁此刻代表谁"。
///
/// 故这一轴的账是：**判据与读数齐，而生产里今天有三格真规矩**（control 那三面）；"给哪一格写
/// 哪一句"仍要**先有客人**——control 那一句写得出来，是因为"这一面谁也取不回"正是
/// `programs/src/system/control/mod.rs` 头注里记着的那个口子。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Permit {
    /// **这一格没记许可**：判据只到 [`judge`] 的第一格（"你有没有身份"），此后一条边都不问。
    ///
    /// **照实记（它为什么是一格，而不是 `Option` 的 `None`）**：帧里这一格要在**同一枚 9 字节**
    /// 里编出五个状态，而 `env::wire::Field` 只能实现在**本 crate** 的类型上——`Option` 是外来的
    /// （孤儿规则），而给 `Option` 另加一格 tag 会让 `Land` 从 60 变 61 字节、破线上契约。
    /// 故"没有"住在这一格里；它与旧 `Rule::Public` 的差别是**它只说一件事**（没记许可），
    /// 不兼"公开入口 / 陌生标记兜底"那两义。
    Unset,
    /// 就是这一位。
    Trunk(PrincipalId),
    /// 这一位在 `p` 那一支里（`p ≼ 本人`，含相等）——纵向那条轴。
    Bough(PrincipalId),
    /// 这一位在这枚盟里——横向那条轴。
    Among(CoalitionId),
    /// **就是开着第 `e` 格的那一位**（那一格的坐标是 [`EntryId`]，不是身份号）。
    Opener(EntryId),
}

/// 「用那一轴」在帧里的标记。**没有许可**那一档是 `0`。
const PERMIT_NONE: u8 = 0;
const PERMIT_TRUNK: u8 = 1;
const PERMIT_BOUGH: u8 = 2;
const PERMIT_AMONG: u8 = 3;
/// `4` 之后的号装的是**格号**（[`Permit::Opener`]），不是身份号——同一个 8 字节那一格。
const PERMIT_OPENER: u8 = 4;

/// 「**用**」那一轴在线上是"**标记 ＋ 8 字节号**"（9 字节）。
///
/// 口径与 [`EntryId`] 那一处相同：**impl 跟着类型走**——这是 [`Permit`] 自己的编码，
/// `frame.rs` 只管"这一格排在整帧的第几格"。
///
/// **陌生的标记 ⇒ 整帧读不懂**（`fetch` 答 `None`）：它与同一帧里 [`Where`] 那一格
/// （表外的记 ⇒ 整帧读不懂）和 `mine`（非 `0/1` ⇒ 整帧读不懂）同一条口径。
/// **这一格不是裁决面**：许可本身怎么判在实现侧的门外那一问里。
impl env::wire::Field for Permit {
    const WIDTH: usize = 1 + 8;

    fn store(&self, out: &mut [u8]) {
        let (tag, id) = match *self {
            Permit::Unset => (PERMIT_NONE, 0),
            Permit::Trunk(p) => (PERMIT_TRUNK, p.get() as u64),
            Permit::Bough(p) => (PERMIT_BOUGH, p.get() as u64),
            Permit::Among(c) => (PERMIT_AMONG, c.get() as u64),
            // 格号与身份号同宽（都是 8 字节）⇒ 帧长一个字节都不动。
            Permit::Opener(e) => (PERMIT_OPENER, e.get() as u64),
        };
        out[0] = tag;
        out[1..].copy_from_slice(&id.to_le_bytes());
    }

    fn fetch(bytes: &[u8]) -> Option<Self> {
        let tag = *bytes.first()?;
        let raw: [u8; 8] = bytes.get(1..9)?.try_into().ok()?;
        let id = u64::from_le_bytes(raw);
        Some(match tag {
            PERMIT_NONE => Permit::Unset,
            PERMIT_TRUNK => Permit::Trunk(PrincipalId::new(id as usize)),
            PERMIT_BOUGH => Permit::Bough(PrincipalId::new(id as usize)),
            PERMIT_AMONG => Permit::Among(CoalitionId::new(id as usize)),
            PERMIT_OPENER => Permit::Opener(EntryId::new(id as usize)),
            // 表外的标记：这一格读不懂 ⇒ 整条问话读不懂。
            //
            // **照实记（这一格曾经全仓零断言，现在有了一台客人）**：要打到它得造一条
            // 60 字节、`[51] ≥ 5` 的 `LAND` 帧；从前仓里没有这样一台客人（`probe-bound` 那台
            // 推的垃圾帧长度就不对，走 `Message::fetch` 里 `bytes.len() == Land::LEN` 那一闸，
            // `match` 一次都到不了）——故那时它是"旧版 `_ => Permit::Unset`（放行）换口径、
            // 不是补判据"。**`probe-bound` 后来添了第 4.5 条**（`junk_land()`：形状全对、只有
            // `[51] = 9`）：它真机上量到门答 `BAD`、随后那句正经的问照样答得出；**并且并排推了
            // 一条只把 `[51]` 换成表内 `0` 的**（答的不是 `BAD`）——两趟一起才证得住"断的就是
            // 许可这一格"，而不是断在名字 / `mine` / 长度上。
            _ => return None,
        })
    }
}

/// **门外那一问的答案**。三格；`Allow` / `Deny` 各一个不同的下一步，`Unjudged` 是"判不了"。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ruling {
    /// 过。
    Allow,
    /// 不过——**终态**：换人 / 换目标 / 别重试。
    Deny,
    /// **判不了**：这一问要的那条事实问不到——对面不答 / 超时（**会好**），或那一号是碑 /
    /// 那一格是块窗格 / 开者那扇门封印了（**好不了**）。
    ///
    /// 两类在这里**同格**：客人的下一步是同一个（当趟放弃），差别在"为什么"⇒ 那是读数
    /// （[`Facts::opens`] 那一侧的三因分得开）。**重试是客人的策略**，本格不作承诺。
    Unjudged,
}

// ── 码 ──────────────────────────────────────────────────────

// 七个动作在报文里的码——**与核心那七条原语同名**（`land` / `part` / `find` / `trim` /
// `list` / `seek` / `name`）：线上与模型是同一件事的两层，不该各起一套词。
//
// **它们不再是协议面**（照板那一族的先例）：编的那一侧由 [`Req`] 说、解的那一侧由 [`Wire`]
// 说，每一枚码各被读一次（字段表头一格 `op`）。外面认的是类型 ⇒ 降为私有——没有读者的格不
// 留在面上。
const LAND: u8 = 1;
const PART: u8 = 2;
const FIND: u8 = 3;
const TRIM: u8 = 4;
const LIST: u8 = 5;
const NAME: u8 = 6;
// **第七个动作**：把一条路**译成号**——名字只能走到这一格，往下一律按号。
//
// 数字取 7 是白捡的：答话那一列里 `BAD` 也是 7，但**动作码与答话码本来就是两张表**
// （今天 `LAND`..`NAME` 的 1..6 与 `UNKNOWN`..`DEAD` 的 1..6 已经重号），故两边各按各的序列。
const SEEK: u8 = 7;

/// 成功那一格：**全协议同一个号**——定义在 `protocol/src/fail_codes.rs`（`fail_codes!` 的第二个参数就是它），
/// 本族只把它转出来。
pub use crate::fail_codes::OK;

/// 答话那一格。**前六格与 [`Fail`] 的前六格一一对应**，第七格不是失败域的：这一问读不懂
/// （帧坏了 ⇒ 不猜、不崩）。**第八、九格来自门外那一问**（判据那一半住
/// `programs/src/system/operator/core/judge.rs`），它们与 [`Fail::Denied`] / [`Fail::Unjudged`]
/// 一一对应。
///
/// 数字是**线上的**，故与动作码同住一处；[`Fail`] 是模型那一侧的名字，两者的对照表只此
/// 一份（持树者那一侧编、客人那一侧读）。
pub const UNKNOWN: u8 = 1;
pub const NONEMPTY: u8 = 2;
pub const NOTATILE: u8 = 3;
pub const NOTAPANE: u8 = 4;
pub const FULL: u8 = 5;
pub const DEAD: u8 = 6;
pub const BAD: u8 = 7;
/// **门外那一问答"不"**：这一位不许动这一格。**终态**——换人 / 换目标，别重试。
///
/// **第八格起不再是 [`Fail`] 的对照表**（[`Fail`] 只有六格）：这两格来自适配层的裁决
/// （判据那一半住实现侧），核心一个字节都不知道它们。分开的理由与
/// [`UNJUDGED`] 同款——"你不许"的下一步与"没铸过 / 剪掉了"不同。
pub const DENIED: u8 = 8;
/// **门外那一问答"判不了"**：这一问要的那条事实问不到——对面不答 / 超时（**会好**），
/// 或那一号是碑 / 那一格是块窗格 / 开者那扇门封印了（**好不了**）。
///
/// 与 [`DENIED`] 分家的理由只有一条，但够硬：**"没资格"与"判不了"是两件事**——混成一格，
/// 就会把"身份服务挂了"读成"我没权限"，整机去查规矩。**它不承诺"等一会儿会好"**：
/// 两类因在客人那一侧是同一个下一步（当趟放弃），把三因分开的是**读数**，不是第三格码。
pub const UNJUDGED: u8 = 9;

/// **落牌的人给这一格声明的条件** —— 两轴，两格。
///
/// ```text
///   [50] 改那一轴   mine: bool        —— 归不归落牌的那一位
///   [51] 用那一轴   标记（0..=4，共五档）
///   [52 .. 60]      号（8 字节 LE）
/// ```
///
/// **两轴是两件事**，故各占各的格：
///
/// - **用**那一轴 = [`Permit`]（四格：就是某一位 / 在某一位那一支里 / 在某枚盟里 /
///   就是开着某一格的那一位；`Unset` = 没有许可）；
/// - **改**那一轴 = 今天原来那一格（"归落牌的那一位"），**它本来就只是 0/1**，故退成一个
///   `bool`——线上值逐字同义（那一格原是 1、没有许可原是 0）。
///
/// 两轴混成一格就会得出"能改的人自然能用"（而反过来才是常见的那一种）。
///
/// # 这一格原来是一个叫 `Rule` 的两格枚举（照实记：撞名）
///
/// 仓里因此有两个同名的 `Rule`（模型那一侧五格、线上这一侧两格），而持树者那一侧同时
/// `use` 了两个——再加一轴就会写出"这个 `Rule` 不是那个 `Rule`"的代码。那一刀把它们拆开：
/// 线上一侧只剩模型那一侧那一个名字（**再出口**），"改"退成 `bool`。
///
/// **照实记（模型那一侧改叫 `Permit`）**：两格枚举退场之后，"规矩"这个词在这一轴上已经不贴了
/// ——它装的是"**许给谁**"（一位 / 一支 / 一盟 / 一格的开者），故模型那一侧叫 [`Permit`]；
/// 答话那一侧仍叫 [`Ruling`]（它答的是"许不许"）。**`Rule` ↔ `Ruling` 那一对由此拆开**。
///
/// **方向也是挑过的**：本文件反向依赖判据那一半（`judge`，住实现侧），而后者从不
/// 依赖本文件——故 `gate` 那条"不与 `protocol` 那一侧沾边"的纪律一字不破（那一侧
/// 拖着 `runtime`，`judge.rs` 不拖）。

/// 问话那一侧的上界：**最长那一条**（`Road`：`op` ＋ [`Path::LEN`]）。
///
/// 服务端按它备一只缓冲（收下来的帧不会超过它），各条问话的**实际**长度由形状说——定长那几条
/// 是字段表求和（`LEN`），`Road` 那一格是 [`RoadFrame::store_at`] 交回的游标。
pub const REQ_LEN: usize = RoadFrame::LEN;

/// 一答的**上限**：四种答形里最大的那一形（`[status][条数][号…]`）。一条 `Pane` 本来就不超过
/// [`PANE_CAP`] 枚 ⇒ **一趟答得完，没有"未完"那一格**（对照 `coalition` 那一侧：盟籍
/// 没有上限，故那里必须带一格"未完"）。
///
/// 本族那只缓冲就是它（[`Message::Buf`]）；另两形都短于它——编译期钉住（`名` 那一形最长是
/// 状态 ＋ 名字那一格的上界（31 字节），`号` 那一形是状态 ＋ 8）。
pub const UNION_LEN: usize = 2 + PANE_CAP * 8;

// 31 = 名字那一格在**这一族**里的上界（长度那一字节不在这一形里：长度即内容）。
const _: () = assert!(Status::LEN + 31 <= UNION_LEN);
const _: () = assert!(Status::LEN + <[u8; 8] as env::wire::Field>::WIDTH <= UNION_LEN);

// 那几枚偏移常量（`AT_ROOT` / `AT_ID` / `NAME_AT` / `TAIL_AT` / `LAND_FRAME`）随字段表一起退场：
// "记"归 [`Where`] 自己的 `Field`（它住 `core.rs`——impl 跟着类型走），其余几个数由各张表求和
// 得出（`Land::LEN` = 60、`Part::LEN` = 42 …），而 `LAND_FRAME` 那个名字没有读者了。
//
// **照实记（`land` 的长度契约收紧了）**：从前 50 字节起就收——最后那两轴读不到就按
// 没有许可走（那是给"还没写这两轴的调用方"留的兜底）。字段表把长度变成**契约**：
// `Land` 就是 60 字节，短一字节整帧读不懂。仓里没有第二种长度（编那一侧一律写全）。

// ── 问话：一个动作一条形状，一张形状一张字段表 ──────────────

// **照实记（`RoadHead` 那一张表退了；今天它以一条表的形状回来了）**：它从前是"动作码 ＋ 段数"
// 两格，而第二格从今天起住 [`Path`]（那一格的第一字节就是它）⇒ 那一版的这一形头只剩动作码，
// 由 `Req::store` 亲手写。`Path` 实现 [`env::wire::Span`] 之后这一形**回表**：[`RoadFrame`]
// 的第一格是动作码、第二格是路（`[op][段数][段…]`，逐字节同形）。
/// `Road` 那一问：动作码 ＋ 一条**绝对坐标**的路。
///
/// **照实记（本族唯一变长的形状）**：[`Path`] 是"要游标那一格"（[`env::wire::Span`]），
/// 故它与动作码并排住同一张表；`LEN` 是**最长那一形**（满路），实际长度由游标说。
/// 那格"段数可以大于实际带的项数"的旧声明照旧收窄：`Path` 里段数**恒等于**带的段数
/// （超上限根本造不出来 ⇒ 读不懂）。
#[derive(env::Frame, Clone, PartialEq, Eq, Debug)]
pub struct RoadFrame {
    pub op: u8,
    pub road: PathBuf,
}

const _: () = assert!(RoadFrame::LEN == 1 + Path::LEN);

/// `List` 那一问：动作码 ＋ 容器坐标。
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct List {
    pub op: u8,
    pub at: Where,
}

/// `Part` 那一问：动作码 ＋ 容器坐标 ＋ 新名。
#[derive(env::Frame, Clone, PartialEq, Eq, Debug)]
#[frame(len = 42)]
pub struct Part {
    pub op: u8,
    pub at: Where,
    pub name: String,
}

/// `Land` 那一问：动作码 ＋ 容器坐标 ＋ 新名 ＋ 入口那一枚 ＋ **这一格的两轴条件**
/// （改那一轴 `mine` / 用那一轴 `permit`）。
#[derive(env::Frame, Clone, PartialEq, Eq, Debug)]
#[frame(len = 60)]
pub struct Land {
    pub op: u8,
    pub at: Where,
    pub name: String,
    pub entry: PieToken,
    pub mine: bool,
    pub permit: Permit,
}

/// `Find` / `Trim` / `Name` 那三问**共用**的形状：动作码 ＋ 一枚号。
///
/// **照实记（这三条为什么共用一张表）**：三者的荷载逐字同形（一枚 [`EntryId`]），差别只在
/// 动作码那一格——故解出来仍是三格（[`Wire::Find`] / [`Wire::Trim`] / [`Wire::Name`]），
/// 而"这一格占多宽"只有一处。
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Entry {
    pub op: u8,
    pub id: EntryId,
}

/// **一问的荷载**——一个动作一条形状，没有"报法"那一格可以填错。
///
/// 号那一侧全按 [`EntryId`] 走；名字只出现在两条路上：[`Req::Road`]（`seek` 收的那条路）
/// 与 `part` / `land` 的**新名**（那是"这一格叫什么"，不是"往哪儿走"）。
///
/// **照实记（名字）**：这一族从前叫 `Ask`（收的那一面叫 `AskIn`）。用户裁定 `Ask` / `Reply`
/// 那一套不要，用 **`Req` / `Wire` / `Union`**——故这里是新生的名字，不是改名。
///
/// **它没有生命周期**（照实记）：从前那是为 [`Req::Road`] 借来的那一段 `&[旧 Name]` 而设，今天那
/// 一格拿的是**自有**的 [`Path`]（`Copy`，自带段数）⇒ 整个枚举里再没有一处借用。
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Req {
    /// `seek`：把一条路译成号（**路只出现在这一格**）。
    Road(PathBuf),
    /// `list`：列那一块 `Pane` 里的号。
    List(Where),
    /// `part`：在那一块 `Pane` 下，给这个新名分一格。
    Part { at: Where, name: String },
    /// `land`：在那一块 `Pane` 下，给这个新名落一枚。
    ///
    /// `entry` 是**经会话交出去之后**、种在持树者表里的那一个号（`ship` 换回来的），
    /// 不是"客人的 Pie 是几号"——两个编号空间不同源。
    ///
    /// `permit` 是**落牌的人给这一格声明的"用"那一轴**（[`Permit`]），`mine` 是**"改"那一轴**
    /// （声明归自己之后，别人接手这一格会被拒）。
    Land {
        at: Where,
        name: String,
        entry: PieToken,
        permit: Permit,
        mine: bool,
    },
    /// `find`：那一号后面那一枚 Pie。
    Find(EntryId),
    /// `trim`：把那一号剪掉。
    Trim(EntryId),
    /// `name`：那一号此刻叫什么。
    Name(EntryId),
}

/// **解开的一问**（名字已经是 [`String`]，故不是借用）。
///
/// 与 [`Req`] 是一对：编的时候按动作分形状，解的时候也按动作分形状——`op` 与荷载不配
/// （比如 `LAND` 那一码配上一枚号）解不出来，持树者据此答 [`BAD`]。
///
/// **照实记（它为什么与 [`Req`] 仍是两个类型）**：`Req` 那一侧是**动作分派**（一个动作一条
/// 形状，编的时候按动作挑），`Wire` 是**解开之后**的那一句——两者的**正文**各写一遍，是因为
/// 两侧要的东西不同（编那一侧要"往哪写"，解那一侧要"读到了什么"），不是为了形状不同。
/// **照实记（路那一格从前确实不一样）**：那时编的借一条、解的收一份（`[Name; ROAD_MAX]` ＋
/// 真实段数）；今天两处都是 [`Path`]，那一处差别随之消失。
/// **表外的动作码不另立一格**（与板那一族不同）：树这一侧对它答 [`BAD`]，故解不出来就是
/// `None`（见 [`Message::fetch`] 那一段）。
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Wire {
    /// `seek`：一条路（段数与段都在 [`Path`] 里；超上限根本造不出来，故 [`FULL`] 不再来自它）。
    Road(PathBuf),
    /// `list`：容器坐标。
    List(Where),
    /// `part`：容器坐标 + 新名。
    Part {
        at: Where,
        name: String,
    },
    /// `land`：容器坐标 + 新名 + 入口那一枚 + **这一格的两轴条件**
    /// （用那一轴 [`Permit`] / 改那一轴 `mine`）。
    Land {
        at: Where,
        name: String,
        entry: PieToken,
        permit: Permit,
        mine: bool,
    },
    /// `find` / `trim` / `name`：一枚号（三者的形状一样，故解出来仍是三格）。
    Find(EntryId),
    Trim(EntryId),
    Name(EntryId),
}

impl Message for Req {
    type In = Wire;
    /// 这一族的缓冲：**最长那一条**（[`REQ_LEN`]）。
    type Buf = [u8; REQ_LEN];
    const EMPTY: Self::Buf = [0u8; REQ_LEN];

    /// 编进 `out`：**动作码由形状给**（不在别处再写一遍），偏移与长度由字段表求和。
    ///
    /// `Road` 那一格的正文（路）也回表了（[`RoadFrame`]：动作码 ＋ 路）——偏移一处都不写。
    fn store(&self, out: &mut [u8]) -> Option<usize> {
        match self {
            Req::Road(road) => RoadFrame {
                op: SEEK,
                road: road.clone(),
            }
            .store_at(out, 0),
            Req::List(at) => List { op: LIST, at: *at }.store_at(out, 0),
            Req::Part { at, name } => Part {
                op: PART,
                at: *at,
                name: name.clone(),
            }
            .store_at(out, 0),
            Req::Land {
                at,
                name,
                entry,
                permit,
                mine,
            } => Land {
                op: LAND,
                at: *at,
                name: name.clone(),
                entry: *entry,
                mine: *mine,
                permit: *permit,
            }
            .store_at(out, 0),
            Req::Find(id) => Entry { op: FIND, id: *id }.store_at(out, 0),
            Req::Trim(id) => Entry { op: TRIM, id: *id }.store_at(out, 0),
            Req::Name(id) => Entry { op: NAME, id: *id }.store_at(out, 0),
        }
    }

    /// 解开一问：**`op` 决定形状**（见文件头那张表）。**读不懂返 `None`**（持树者据此答
    /// [`BAD`]）。
    ///
    /// **长度为该形状该有的长度是帧的契约**（各张表的 `LEN`，`store` 产出的就是那个长度），
    /// 故短一字节、长一字节都读不懂。
    ///
    /// **照实记（"段数原样报出去"与"只解前 `ROAD_MAX` 段"那两句都退了）**：从前段数可以写得
    /// 比带回来的多（"路太长"由持树者答 `FULL`），故解的那一侧得自己裁、还得躲开零填充之间的
    /// 空段（真机实测：四格全答 `BAD` 就栽在这里）。今天段数与段由 [`Path`] 一口说清：
    /// 谁造的路谁带几段，**长度即形状**。
    ///
    /// **表外的动作码 ⇒ `None`**：树这一族不另立"表外的码"那一格（对它的答话与"读不懂"
    /// 同一句，见 [`Wire`] 的照实记）。
    fn fetch(bytes: &[u8]) -> Option<Wire> {
        let op = *bytes.first()?;
        Some(match op {
            // **长度即形状**：`1 ＋ 1 ＋ 段数 × 32`（段数那一格在 [`Path`] 里；条数与长度对不对
            // 由下面那一句判——短一字节、长一字节都答"读不懂"）。
            SEEK => {
                let (frame, at) = RoadFrame::fetch_at(bytes, 0)?;
                if at != bytes.len() {
                    return None;
                }
                Wire::Road(frame.road)
            }
            LIST if bytes.len() == List::LEN => Wire::List(List::fetch(bytes)?.at),
            // 这两形含一枚变长名字 ⇒ **"恰好"按游标判**（帧长不再等于那张表的 `LEN`）。
            PART => {
                let (frame, end) = Part::fetch_at(bytes, 0)?;
                if end != bytes.len() {
                    return None;
                }
                Wire::Part {
                    at: frame.at,
                    name: frame.name,
                }
            }
            LAND => {
                let (frame, end) = Land::fetch_at(bytes, 0)?;
                if end != bytes.len() {
                    return None;
                }
                Wire::Land {
                    at: frame.at,
                    name: frame.name,
                    entry: frame.entry,
                    permit: frame.permit,
                    mine: frame.mine,
                }
            }
            FIND | TRIM | NAME if bytes.len() == Entry::LEN => {
                let id = Entry::fetch(bytes)?.id;
                match op {
                    FIND => Wire::Find(id),
                    TRIM => Wire::Trim(id),
                    _ => Wire::Name(id),
                }
            }
            // 没见过的动作码、或长度不是这张形状该有的那个 ⇒ 读不懂（不另立一格）。
            _ => return None,
        })
    }
}

// 手写的那六手（`op_of` / `unpack_ask` / `unpack_at` / `unpack_id` / `unpack_name` / `tail`）与
// `pack_ask` 一起退场：编与解各由"一张字段表 ＋ 一条 `match`"说（见上面那一段）。
//
// **照实记（那六手都是"同一件事的第二处"）**：`unpack_at` 与 `pack_at` 各写一遍"记 ＋ 号"、
// `unpack_name` 与 `pack_name_in` 各写一遍"名字那一格怎么切"、`pack_rule` 与 `unpack_rule` 各
// 写一遍那九个字节——写者与读者分居文件两头，**错一处编得过**，症状要等那一帧被读成"读不懂"
// 才显形。今天这三件事各只有一处：`Where` / `Permit` 各自的 `Field`，名字那一格归 `String` 的 `Span`。

// ── 答：一格状态 / 一串号 / 一枚名字 / 一枚号 ─────────────────

/// 一帧「列」的读数：号最多 [`PANE_CAP`] 枚。
///
/// **照实记（为什么不与 `coalition` 的 [`Window`](crate::service::coalition::Window) 并成一个容器）**：
/// 两者都在搬"一串号"，差的正是**"未完"那一格**——盟籍**没有上限**（一格盟可以很多人）⇒ 那边
/// 必须带 `more`，并因此把格子存成 `[Option<T>; CAP]`（泛型 + `const new` 造不出 `T` 的占位，
/// 而零号是**真格子**，不能拿它当空）；**一条 pane 本来就有顶**（[`PANE_CAP`]）⇒
/// "还没完"这件事在这一族**不存在**，带 `more` 就是一格**恒假**的字段。故两处各留一个，
/// **帧形也跟着**（[`Tally`] 无"未完"、coalition 的 `SeqHead` 有）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Listing {
    ids: [EntryId; PANE_CAP],
    n: usize,
}

impl Listing {
    /// 空的那一串。
    pub const fn new() -> Listing {
        Listing {
            ids: [EntryId::new(0); PANE_CAP],
            n: 0,
        }
    }

    /// 收一串（**收够 [`PANE_CAP`] 枚就停**：一条 pane 本来就不超过它）。
    ///
    /// **照实记（它替掉了 `pack_list` 那一手）**：从前编那一侧直接往缓冲里写（`2 + n * 8`
    /// 那几个偏移）；现在编的是**这一枚容器**，落字节归 [`Tally`] 与 [`env::wire::store_tail`]。
    pub fn of(ids: impl Iterator<Item = EntryId>) -> Listing {
        let mut listing = Listing::new();
        for id in ids.take(PANE_CAP) {
            listing.push(id);
        }
        listing
    }

    /// 按号序（就是帧里的次序）走一遍。
    ///
    /// **照实记（这一份只剩这一个读面）**：原先还有 `len` / `is_empty` / `get` 三格——
    /// `is_empty` / `get` **全仓零用家**，`len` 只被宿主靶用过（`back.len()`），而生产路径
    /// （`echo` 的读数）只走 `iter()` ⇒ 三格都删掉，那处改写成 `iter().count()`。
    /// 要"几枚"就问这一句。
    pub fn iter(&self) -> impl Iterator<Item = EntryId> + '_ {
        self.ids[..self.n].iter().copied()
    }

    /// 那一段号——**编那一侧要它**（`store_tail` 走的是一条切片，不是一个迭代器）。
    pub fn as_slice(&self) -> &[EntryId] {
        &self.ids[..self.n]
    }

    /// 收一枚。**满了就丢**：一条 pane 本来就不超过 [`PANE_CAP`] 枚。
    fn push(&mut self, id: EntryId) {
        if let Some(slot) = self.ids.get_mut(self.n) {
            *slot = id;
            self.n += 1;
        }
    }
}

// ── 答那一侧的三张字段表（四种答形共用它们）──────────────────

/// **头一格**：状态。它自己就是"一格状态"那一形（六格失败与"门外那两格"都走它），也是另外
/// 三形的起头。
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Status {
    pub status: u8,
}

/// 「列」那一形的**头两格**：状态 ＋ **条数**（后面跟着那么多个号——那是尾巴，走
/// [`env::wire::store_tail`]）。
///
/// **这一格的条数与帧长绑死**（读的人两边对不上就判读不懂），故它**不是** `Road` 那一格
/// 的条数（那里的条数是**声明**，允许大于实际带的）——两句不同的话，故各说各的。
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Tally {
    pub status: u8,
    pub count: u8,
}

/// 「号」那一形：`[status][8 字节]`——**定长 9**（`part` / `seek` 答坐标、`find` 答门闩，
/// 线上逐字同形）。
///
/// **照实记（这一格的类型为什么是裸 8 字节）**：两个号空间（[`EntryId`] / [`PieToken`]）
/// 在这一格上分不开，故字段表不假装它是哪一枚——读面见 [`Said::entry`] / [`Said::seed`]。
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Word {
    pub status: u8,
    pub word: [u8; 8],
}

/// **一答的形状**——答有四种：一格状态 / 一串号 / 一枚名字 / 一枚号。
///
/// **照实记（名字）**：这一族从前是 `pack_list` / `pack_name` / `pack_id` / `pack_seed` 四枚
/// 自由函数（外加 `read_list` / `read_name` / `read_id` 三枚）。用户裁定这一族用
/// `Req` / `Wire` / `Union`，而答的**读**那一面叫 [`Said`]——故这里是新生的名字，不是改名。
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Union {
    /// 一格状态（成功 / 六格失败 / 门外那两格）——**没有任何荷载**。
    Status(u8),
    /// `list` 的下场：一串号。
    List(Listing),
    /// `name` 的下场：一枚名字（**长度即名长**）。
    Name(String),
    /// `part` / `seek` 的下场：那一格**坐标**。
    Entry(EntryId),
    /// `find` 的下场：那一格是"我给你的那一枚**在你表里**是几号"（[`PieToken`]）。
    ///
    /// **与 [`Union::Entry`] 同形不同物**（都是 `[OK][8 字节]`）而**另起一格、不复用**：两枚号
    /// 类型不同，混用就是把"树的坐标"与"你表里的门闩"当成一件事。
    ///
    /// **照实记（这一格为什么在帧里）**：从前 `find` 只答一格状态，客人拿到 `OK` 之后还得**扫
    /// 自己的表**按"谁给的"把那一枚认回来（`operator::take`）。而号本来就在持树者手上
    /// ——`port::ship` 的 `to.seed()`，原先被 `.map(|_| ())` 扔掉——故随答话一起过来，客人拿它
    /// 一次 `Reserve` 就验得完。代价照实记：**答话丢了一趟，那一枚号也跟着丢**（今天还能靠扫表
    /// 侥幸认回来）——与 rtc / principal / coalition 那三面同一个取舍。
    Seed(PieToken),
}

/// **收进来的一答**：**原样的字节** ＋ 四个读法。
///
/// **照实记（答这一侧为什么不像问那一侧那样"一个类型说形状"）**：四种答形**在线上分不开**
/// ——`[OK][条数][号…]`、`[OK][名字]`、`[OK][8 字节]` 都以状态那一格起头，而"名"那一条是变长的
/// （**长度即名长**：没有终止符、也没有条数）。分得开它们的是**问的人**——他问的是哪一条自己
/// 知道。故这一枚把字节原样收下，四个读法各按一形解；**形状不对 ⇒ [`BAD`]**（与从前那三枚
/// `read_*` 同一个判据，只是收在了一处）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Said {
    buf: [u8; UNION_LEN],
    len: usize,
}

impl Said {
    /// 这一条答的字节。
    fn bytes(&self) -> &[u8] {
        self.buf.get(..self.len).unwrap_or(&[])
    }

    /// 那一格状态（四种答形的头一格都是它）。
    ///
    /// 空帧（连状态都没有）⇒ [`BAD`]——核里空不是消息，故这一格只防"读法被用错"。
    pub fn code(&self) -> u8 {
        self.bytes().first().copied().unwrap_or(BAD)
    }

    /// 按「号」那一形读（`land` / `part` / `seek` 的下场）：`[status][8 字节]` → **坐标**。
    ///
    /// 状态不是 [`OK`] ⇒ `Err(那一格码)`；不是那一形（长度不对）⇒ `Err(BAD)`。
    pub fn entry(&self) -> Result<EntryId, u8> {
        Ok(EntryId::from_bytes(self.word()?))
    }

    /// 按「门闩」那一形读（`find` 的下场）：同一形状 → **你表里的那一枚号**。
    pub fn seed(&self) -> Result<PieToken, u8> {
        PieToken::from_bytes(&self.word()?).ok_or(BAD)
    }

    /// 「号」那一形里的那 8 字节（上面两个读法共用的那一格）。
    fn word(&self) -> Result<[u8; 8], u8> {
        let code = self.code();
        if code != OK {
            return Err(code);
        }
        let bytes = self.bytes();
        if bytes.len() != Word::LEN {
            return Err(BAD);
        }
        Ok(Word::fetch(bytes).ok_or(BAD)?.word)
    }

    /// 按「名」那一形读（`name` 的下场）：`[status][名字]` → 一枚名字。
    ///
    /// 名字读不懂（空 / 太长 / 含 NUL / 不是 UTF-8）⇒ `Err(BAD)`：那一侧旧日的四格失败域
    /// 在这里**归一格**——问的人能做的补救是同一件（这一帧坏了，重问）。
    pub fn name(&self) -> Result<String, u8> {
        let code = self.code();
        if code != OK {
            return Err(code);
        }
        let bytes = self.bytes();
        let text = env::wire::fetch_bytes(bytes, Status::LEN).ok_or(BAD)?;
        // 长度即内容 ⇒ 那些字节自己就是一枚名（UTF-8 是这一格的义务；空也是合法的名）。
        Ok(String::from(
            core::str::from_utf8(text).map_err(|_| BAD)?,
        ))
    }

    /// 按「列」那一形读（`list` 的下场）：`[status][条数][号…]` → 一串号。
    ///
    /// **帧长即条数**：条数与剩下那些字节对不上（或条数超过 [`PANE_CAP`]）⇒
    /// `Err(BAD)`——短一字节也是它。
    pub fn list(&self) -> Result<Listing, u8> {
        let code = self.code();
        if code != OK {
            return Err(code);
        }
        let bytes = self.bytes();
        let head = Tally::fetch(bytes).ok_or(BAD)?;
        let count = head.count as usize;
        if count > PANE_CAP {
            return Err(BAD);
        }
        let body = bytes.get(Tally::LEN..).ok_or(BAD)?;
        let mut ids = [EntryId::new(0); PANE_CAP];
        let end = env::wire::fetch_tail(body, 0, &mut ids[..count]).ok_or(BAD)?;
        if end != body.len() {
            return Err(BAD);
        }
        Ok(Listing::of(ids[..count].iter().copied()))
    }
}

impl Message for Union {
    /// 收的那一面是 [`Said`]（**原样的字节**——形状由问的人认，见它的照实记）。
    type In = Said;
    /// 这一族的缓冲：**最大那一形**（[`UNION_LEN`]）。
    type Buf = [u8; UNION_LEN];
    const EMPTY: Self::Buf = [0u8; UNION_LEN];

    /// 编进 `out`：状态由形状给（不在别处再写一遍），变长那两段交给 `env::wire` 的两个尾巴。
    fn store(&self, out: &mut [u8]) -> Option<usize> {
        match self {
            Union::Status(code) => Status { status: *code }.store_at(out, 0),
            Union::List(list) => {
                let ids = list.as_slice();
                let head = Tally {
                    status: OK,
                    count: ids.len() as u8,
                };
                head.store_at(out, 0)?;
                env::wire::store_tail(out, Tally::LEN, ids)
            }
            Union::Name(name) => {
                let at = Status { status: OK }.store_at(out, 0)?;
                // 名长即这一帧剩下的那些字节（**长度即内容**那一形）。
                env::wire::store_bytes(out, at, name.as_bytes())
            }
            Union::Entry(id) => Word {
                status: OK,
                word: id.to_bytes(),
            }
            .store_at(out, 0),
            Union::Seed(seed) => Word {
                status: OK,
                word: seed.to_bytes(),
            }
            .store_at(out, 0),
        }
    }

    /// 收一条：**原样收下**（空帧、或长过这一族的缓冲 ⇒ `None`）。形状不在这里判——
    /// 见 [`Said`] 的照实记。
    fn fetch(bytes: &[u8]) -> Option<Said> {
        if bytes.is_empty() {
            return None;
        }
        let mut buf = [0u8; UNION_LEN];
        buf.get_mut(..bytes.len())?.copy_from_slice(bytes);
        Some(Said {
            buf,
            len: bytes.len(),
        })
    }
}

// ── 提示之路（**装配者 → 持树者**，不是门外那一问）：两形 ──────
//
// 它不在上面那张图里：上面那几帧是**客人 ↔ 持树者**的一问一答，这几条是**装配者递过来
// 的东西**（立一条路 / 一位客人）。两族同住本文件，因为"帧形只有一处"这一条不分装配期与
// 运行期——它是同一棵树的两半。两形的总说明与 `Tip` / `TipIn` 在下面。（"一格号"那一形
// 随 `Eyes` 退场，见下面那条照实记。）

// ── 提示之路那两形：一条路 / 一位客人 ────────────────────────
//
// 两形走**同一个洞、同一个读者**（提示之路 = 装配侧 → 持树者）：既不经过会话、也没有客人
// ——"往树上立一路"由持树者在自己核里做（`programs/src/system/operator/plate.rs::plate`）。
//
// **首格 `kind` 说这一帧是哪一形**——与客人那一族的动作码同一条纪律：一个动作一条形状，
// 一张形状一张表。**照实记（从前靠长度分派）**：那三形原先按长度认（`n == 73` / `16` / 其余），
// 于是"三形不许等长"得靠两条 `const _: () = assert!` 兜着（那三形里的"一格号"今天已退场）；
// 而**一条路有几段会让长度变**
// ——今天认出哪一形只看第一格，长度回到它本来的意思（这一帧有多长）。

/// 提示之路上的两个 `kind`（首格；表外 ⇒ 这一帧读不懂）。
///
/// **照实记（"一双眼睛"那一形退场）**：那一形从前是"哪一位域把门牌交过来了、它是哪一双眼睛"
/// （`Coord` 帧 ＋ `env::wire::Eyes` 那一格）——而那一格号（`who`）持树者**自己用不着**：它只拿它
/// 去本表里按「谁开的 ＋ 记号」找那枚门牌，而两个记号是**协议里各族自己的常量**、各自只有一家
/// 生产者 ⇒ **按记号单独就找得到**（`operator::claim::face_of_mark`）。于是整形退场：帧那一形、
/// `Eyes` 那一格、协调状态那一本、装配声明上那一格，一处不留。`kind` 值不留空位（两形重排）。
const TIP_PLATE: u8 = 1;
const TIP_GUEST: u8 = 2;
const TIP_WIRED: u8 = 3;

// **照实记（`PlateHead` 这一张表退了；今天它以 [`PlateFrame`] 回表）**：它从前是"`kind` ＋
// 段数"两格。段数那一格今天住 [`Path`]，故那一版的这一形首格就是 `kind`——由 `Tip::store` 亲手写。
// 从前它与客人那一族 `RoadHead` 的差别（"这条路没有答话那一格 ⇒ 超长当场读不懂"）今天同归
// 带它的那张表（[`RoadFrame`] / [`PlateFrame`]：**恰好**那一句）。
/// 「一位客人」那一形：`kind` ＋ 它的号。
///
/// 持树者按"它开的 ＋ 记号"在本表里认它那条答话路——"这一位域"那一形已随 `Eyes` 退场
/// （见上面那条照实记）。
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct GuestFrame {
    pub kind: u8,
    pub who: TaskId,
}

/// 「门禁接线」那一形：**只有一个字节**（`kind`）——一句话，不带号。
///
/// **它说的是什么**：装配者已经把**名册**认下来了（补绑它自己与树），从那以后持树者那道门
/// **问得动身份**（`operator::door::may`）。
///
/// **为什么这一句非有不可**（照实记：量出来的，它替代了原来那两格号）：门一旦接线就按"谁在问"
/// 判身份，而在装配者补绑名册**之前**，名册自己那一趟上树（`land`）就会被自己那道门拒掉
/// ——实测 `principal: start failed`（名册起手走不完 ⇒ 它那道"我答得了"的孔永远不铸）。
/// 故"什么时候算接线完成"是**装配者手里的事实**（只有它做完那一次补绑），由它推这一句空话。
///
/// **为什么不带号**：原来那一形带"哪一位域 ＋ 它是哪一双眼睛"两格，而持树者拿那两格只做一件事
/// ——按「谁开的 ＋ 记号」找门牌；两个记号各只有一家生产者，**记号单独就够**（见 `claim::face_of_mark`）。
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct WiredFrame {
    pub kind: u8,
}

/// **这一趟落格要带的那句规矩**（"立一条路"那一形上的第二轴）。
///
/// # 为什么只有两格，且没有"填一枚 `Permit`"这一路
///
/// 这条路上装的是**装配者**（它请持树者替它落格）。装配者**报不出任何号**——它没有名录面
/// （`Roster` 只有 `bind` / `adopt`），也没有读格的那几手（`Tree` 只有"递上去"）⇒ 一枚
/// `Permit` 里的号它一个都填不了。故这一轴说的是**要不要带规矩**，而不是"带哪一条"；
/// 而"带哪一条"由 [`Rule::Root`] 自己钉死——那是这一族今天**说得出口**的唯一一句真话
/// （理由与实测见它自己的照实记）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Rule {
    /// **不记许可**：与 `/svc/sys/operator/{…}` 那七格同一条口径——任何已绑身份都取得回。
    None,
    /// **许给根**（`Permit::Trunk(PrincipalId::ROOT)`）：这一格只有根能取。
    ///
    /// **照实记（原先写的是"许给开着这一格的那位"，真机上是一条假规矩）**：装配者铸的那几枚入口
    /// 其开者**答不出来**——根那批孔是"引导期那批设备门闩"（`opened_by` 的 `owner` 格是 0），
    /// 故 `Permit::Opener(这一格)` 判出来是 [`Ruling::Unjudged`] 里"**永久**判不了"那一类
    /// （实测读数：`operator: opens sealed n=33`），而这一族的正文写着那条规矩"要修的是写它的人"。
    /// ⇒ 这条路改用"**许给根**"：它是这一族今天**说得出口**的那一句真话。
    Root,
}

impl Rule {
    /// 这一格在帧里有多宽（与 [`Rule::store`] / [`Rule::fetch`] 那一对同源）。
    pub const WIDTH: usize = 1;
}

impl env::wire::Field for Rule {
    const WIDTH: usize = Rule::WIDTH;

    fn store(&self, out: &mut [u8]) {
        out[0] = match *self {
            Rule::None => 0,
            Rule::Root => 1,
        };
    }

    fn fetch(bytes: &[u8]) -> Option<Self> {
        match *bytes.first()? {
            0 => Some(Rule::None),
            1 => Some(Rule::Root),
            // 表外的记 ⇒ 整帧读不懂（同 `Permit` / `Where` 那一格的口径）。
            _ => None,
        }
    }
}

/// `Plate` 那一句：首格 `kind` ＋ 一条路 ＋ 末段那一枚 ＋ 规矩一格。
///
/// **照实记（本族的第二种变长形状）**：中间那一格是 [`Path`]（"要游标那一格"），而它**之后还有
/// 两格定长**——从前这一形的两手全是手写偏移（`+ 1` / `+ PieToken::WIDTH` / `+ Rule::WIDTH`），
/// 回表之后偏移一处都不写（`LEN` 是最长那一形）。
#[derive(env::Frame, Clone, PartialEq, Eq, Debug)]
pub struct PlateFrame {
    pub kind: u8,
    pub road: PathBuf,
    pub leaf: PieToken,
    pub rule: Rule,
}

const _: () = assert!(PlateFrame::LEN == 1 + Path::LEN + PieToken::WIDTH + Rule::WIDTH);

/// 提示之路上**最长那一形**的宽度（立一条路：[`PlateFrame`]）——两侧各备一只这么大的缓冲，
/// 收的那一侧按它拉。
pub const TIP_LEN: usize = PlateFrame::LEN;

/// **装配者推给持树者的一句话**（提示之路那一帧）。
///
/// 三形，各自的正文在变体上；共用的两句话：
///
/// - **树不能当自己的客人**：把一格挂上树在别处都是**客人**那一趟（`part` ＋ `land` 两问走一条
///   会话），而树没有那条会话——它的生我者（编排域）是**替每一位客人转授**的那一侧，替不了
///   自己（自指 ⇒ 环）。树手里本来就握着**核**（`Operator::land` / `part`）
///   ⇒ "装配者递东西、持树者自己落"。
/// - **名字随帧来**：持树者不认识任何一族的名字（`control::frame::DIR` / `NAME` 都是递帧那一侧
///   的事实），它只答"把这一条路立出来"。
///
/// **编与解是两个类型**（同 [`Req`] / [`Wire`]）；**它没有生命周期**：从前的借用只为
/// [`Tip::Plate`] 那一条借来的路，今天那一格拿的是**自有**的 [`Path`]（照实记同 [`Req`]）。
pub enum Tip {
    /// **在树上立一条路**：前缀逐段立成窗格（缺的就地造），末段按 `leaf` 落叶子或立窗格。
    ///
    /// 路是**绝对坐标**（从根起数），故 `/svc/sys/control`、`/svc/sys/operator`、`/svc/sys/operator/part`
    /// 三种落法**同一个形状**说得出来；再深一层、或"父底下立一块窗格"也不需要新格
    /// ——**照实记（从前说不出第四种）**：那一版是"两段名字 ＋ 一格 `layer`"（`Sys` / `Segment` /
    /// `Under`），而"父是 `/svc/{dir}`、末段却是窗格"这一格**说不出来**；今天它就是
    /// `Plate { road: [.., dir, name], leaf: NONE }`。
    ///
    /// `leaf` 填 [`PieToken::NONE`] 说的是"**末段是窗格**"（没有可落的叶子）——它在整帧里只有
    /// 这一个意思，不按别的格改读法（目录不是叶子：没有入口、没有 Pie）。
    ///
    /// **照实记（`NONE` 那一格原先没有生产者；开面那一刀之后有了）**：那两条挂载路（`/sys/control`
    /// 与七位）从前都是**落叶子**，而目录那一格由前缀走出来（不单独占一帧）——故 `NONE` 一次也
    /// 发不出去。**`/svc/sys/control` 拆面之后**：它自己变成一段前缀，而"前缀立成一块窗格"正是
    /// `leaf = NONE` 那一形（见 `programs/src/system/mod.rs::mount_control`）⇒
    /// **这一格的第一位生产者就是它**。
    /// 它留着是因为它是这一形的**第二轴**：把 `leaf` 收成必填，"**立一段空窗格**"这句话就再也
    /// 说不出来（将来别的模块搬上树时，第一句常是它）。这与 `Req::Land` 那一格"每一格都还要有
    /// 意思"同一条：宁可多一格**说得出口**的话，也不让一个动作只许一种理解。
    Plate {
        road: PathBuf,
        leaf: PieToken,
        rule: Rule,
    },
    /// **这一位是客人**。
    Guest(TaskId),
    /// **门禁接线**（装配者已认下名册）：一句话，不带号。
    Wired,
}

impl Tip {
    /// 编进 `out`，返写完的游标；装不下 / **路空** ⇒ `None`（路本身合法由 [`Path`] 保证）。
    pub fn store(&self, out: &mut [u8]) -> Option<usize> {
        match self {
            Tip::Plate { road, leaf, rule } => {
                if road.is_empty() {
                    return None;
                }
                PlateFrame {
                    kind: TIP_PLATE,
                    road: road.clone(),
                    leaf: *leaf,
                    rule: *rule,
                }
                .store_at(out, 0)
            }
            Tip::Guest(who) => GuestFrame {
                kind: TIP_GUEST,
                who: *who,
            }
            .store_at(out, 0),
            Tip::Wired => WiredFrame { kind: TIP_WIRED }.store_at(out, 0),
        }
    }
}

/// **解开的一句**：路已经收进自己那一份（[`Path`]）。
pub enum TipIn {
    /// 立一条路（前缀逐段立窗格，末段按 `leaf`），并按 `rule` 决定要不要带一句规矩。
    Plate {
        road: PathBuf,
        leaf: PieToken,
        rule: Rule,
    },
    /// 这一位是客人。
    Guest(TaskId),
    /// 门禁接线。
    Wired,
}

impl TipIn {
    /// 解开一句：**首格 `kind` 决定形状**，长度必须是那一形该有的长度。
    ///
    /// 读不懂（表外的 `kind` / 路空 / 段数越界 / 长度不对）⇒ `None`：持树者据此报一行读数
    /// ——这条路上没有答话那一格，**别静默丢**。
    pub fn fetch(bytes: &[u8]) -> Option<TipIn> {
        match *bytes.first()? {
            TIP_PLATE => {
                let (frame, at) = PlateFrame::fetch_at(bytes, 0)?;
                // **路空**是这一族的规矩（"末段"必须有）；**长度**也是形状的一部分
                // （`1 ＋ 1 ＋ 段数 × 32 ＋ 8 ＋ 1`：长短都不认）。两句都是本族的，derive 不替它判。
                if frame.road.is_empty() || at != bytes.len() {
                    return None;
                }
                Some(TipIn::Plate {
                    road: frame.road,
                    leaf: frame.leaf,
                    rule: frame.rule,
                })
            }
            TIP_GUEST if bytes.len() == GuestFrame::LEN => {
                Some(TipIn::Guest(GuestFrame::fetch(bytes)?.who))
            }
            TIP_WIRED if bytes.len() == WiredFrame::LEN => Some(TipIn::Wired),
            _ => None,
        }
    }
}

// ── 失败域 ↔ 答话码 ─────────────────────────────────────────

crate::fail_codes! {
    /// 失败域 → 答话那一格（`None` = 一个失败都不是）。
    ///
    /// **九格成一枚完整双射**：前六格是核心自己的失败，后两格是门外那一问（判据层）的两格
    /// ——`DENIED` / `UNJUDGED` 本来就在线上答得出来，故客侧读得回来。`BAD` 在表外。
    bijective Fail; OK;
    Fail::Unknown => UNKNOWN,
    Fail::NonEmpty => NONEMPTY,
    Fail::NotATile => NOTATILE,
    Fail::NotAPane => NOTAPANE,
    Fail::Full => FULL,
    Fail::Dead => DEAD,
    Fail::Denied => DENIED,
    Fail::Unjudged => UNJUDGED,
}

// ── 载体两侧共用的坐标 ─────────────────────────────────────
//
// 这几格是**记号与名字**：两侧都要按它认领/铸孔，故只能有一份（规则 5）。

/// 树那条通道的名字：**两侧同一个**（泊位自己的坐标，不进报文）。
pub const LINK: &str = "operator";

/// 问话孔那一枚上的记号（两侧同一个：客人铸它时刻上去的，持树者按它认领那枚孔）。
///
/// **带面名**（同 `board` 那面的 `board-ask`、以及两面的 `*-tip`）：问话孔的认领键是
/// "**谁开的 + 记号**"，而**同一枚任务可能同时是两面的客人**（`canonical` / `guest` / `principal`
/// …都是：一边问板、一边问树）——两枚孔都铸在**它自己那张表**里，记号再一样就分不开了。
///
/// 照实记（这一格是**量出来的**，不是想出来的）：把记号统一成 `ask` 之后，客侧"先找后铸"
/// 的那一手当场把**板那一枚**当成了树那一枚交回来 ⇒ 树那条路永远没有问话孔 ⇒ 装机就塌
/// （实测 `principal: tree … got=false` + `system: service failed`，`examine` **0/3**）。
/// 今天两面的孔落进**两张不同的表**（板线程 / 持树者），故这件事从来没露过头。
pub const ASK_MARK: Mark = Mark::of("operator-ask");

/// 提示孔那一枚上的记号（持树者铸它时刻上去的；装配者按它认领那一枚）。
///
/// **照实记（这一格原先有两个记号）**：装配者那一侧从前还 `seat` 过一次——本端另铸一枚、刻的是
/// 另一个记号（`TIP_NAME = "operator-tip"`）——而那一枚**两头都不用**：那条路上只走"往持树者
/// 那一枚里推一位新客人 / 一帧协调"。两个记号并成一个之后，装配者那一侧只剩
/// [`establish::claim`](crate::communication::establish::claim) 一手，另一个名字随之退场
/// （`programs/src/system/operator/bridge.rs::host_of`）。它那条注里"引导域按名来要"的说法
/// **没有下家**：引导期那一枚提示孔今天由 `Tree` 自己拿着逐次传下去。
pub const TIP_MARK: Mark = Mark::of("tip");

// ── 面不相撞（**编译期**钉住——用户裁定"常量交给编译器"）────────────────────
//
// 原先这是宿主台的一条运行时用例（`the_operator_marks_do_not_collide_with_the_other_doors`，
// 搬出运行时源时随用例一起改到这里）：这几枚值各是一枚 FNV 散列（`env::Mark::of`），
// **撞了就是那次装机塌掉**（见上面 `ASK_MARK` 的照实记）。挪到编译期之后，riscv 那一档也一样
// 钉着——"一漂就编不过"，且不再占一条用例。
//
// 比的是 `.get()` 那个裸值：`Mark` 的 `PartialEq` 不是 `const`，而 `get` 是 `const fn`。
const _: () = assert!(ASK_MARK.get() != Mark::of("board-ask").get());
const _: () = assert!(ASK_MARK.get() != Mark::of("ask").get());
const _: () = assert!(ASK_MARK.get() != TIP_MARK.get());
