//! system/operator — **正文已搬进「约」**（`crates/contract/src/system/operator/mod.rs`）。
//!
//! 这里只剩**碰内核的那几件**（客侧那几手、十件手的身体、绑真手的构造与两张对照表）——
//! 判据见 `crates/protocol/src/lib.rs` 与 `crates/contract/src/lib.rs`。


// ── 载体：三侧分别住在哪 ───────────────────────────────────
//
// **使用侧** [`client`]（客侧三手）住这里——那是"别的任务怎么找上树"。**今天的客人**：六台域
// （`echo`（自问自答一趟：分 → 落 → 寻 → 收 → 剪）、`router` / `rtc` / `uart`（各把门牌挂上
// 树）、`principal` / `coalition`（上树那条 `/sys` 路））与测具一串（`harness` 的 `subject` /
// `member` / `guest` / `lodger` / `sleeper` / `probe_*`）。**实现侧**（持树者）
// 与**装配侧**（把持树者接上客人 / 认下提示之路）住 `programs/src/system/operator/{server,bridge}.rs`。
// 下面这段是那一台的说明——它讲的是"怎么跑"。
//
//!  同一对动作（`seat` + `claim`），靠**孔上的记号**对位。
//!
//!  ```text
//!    装配者（编排域 system）                    客人（某个服务域）        持树者（operator 域，一枚线程）
//!    quay.seat(LINK) + quay.claim(客人, LINK) ▶ open: seat(LINK) + claim(生我者, LINK)
//!    转授：把客人那一枚 Ship 给持树者 ──────────────────────────────────▶  按"谁转授的 + 记号"认答话写端
//!    LINK 上先递一格：持树者的号 ────────────▶  open 收下 ⇒ 此后叫得出它
//!    提示：往提示之路推一个客人号 ─────────────────────────────────────▶  收一位客人（admit）
//!                                             铸问话孔(ask)、Ship 给持树者 ▶  按"谁开的 + 记号"认出 ⇒ arm + attach
//!                                             推(Req) ───────────────────▶  组唤醒 ⇒ pull ⇒ 交给树
//!                                             读(Reply) ◀────────────────  推(Reply)
//!  ```
//!
//!  # 为什么持树者就一枚线程
//!
//!  树上那几枚是**一枚只在持它的那张表里有意义的句柄**（`PieToken` = "我这张表里的第几个"）。
//!  "查到了要把 Pie 授出去"必须由**持有那一枚的那张表**来做——故所有条目只能住同一张表，
//!  也就是同一枚线程。板那一台栽过这条（每位客人一枚待客线程 ⇒ 甲的条目在甲的表里，乙来查
//!  时判它"已死"、也授不出去，症状是"刚挂上的名字，别人一查就是 `Unknown`"）。
//!
//!  # 与板那一台的差别（照实记）
//!
//!  1. **持树者住在自己的域里**（板线程住编排域）：故它是被 `service::spawn` 产出来的，
//!     装配者从 `task` 就知道它是谁，不必再起线程；它那一枚提示孔由**它自己**铸、Ship 给
//!     生我者（= 装配者 = 那一枚 `Quay::claim` 的对端）。
//!  2. **没有"客人说走了"那一档**：本正文里没有客人生命周期（谁问都答）。故没有退场的
//!     那一格——只有"看出来"那一档（那一枚答不出 ⇒ 剔格子）。
//!  3. **死亡那一档不在本协议里**：它由**板**那条路看——树这一侧也**上板**、与别的服务同形
//!     （`scenario.rs` 的 `INNER` 里那一格 `board: true`；道是装配者铸的，名字也由它随提示
//!     那一格递过去，见 `protocol::system::board::call::Tip::LEN`）。本协议不推任何东西给
//!     装配者。
//!  4. **帧里带一整条路**（段列表），不是单个名字；入口那一枚仍然经会话交出去，报文里只有
//!     一格状态。
//!
//!  # 入口那一枚不经装配者转授
//!
//!  客人要跟持树者说话，只需两样：**它是谁**（号）与**一条答话路**。号由装配者递一格告诉它
//!  （[`attach`] 的 `tell`），答话路由装配者转授给持树者。**客人递 Pie 给持树者不必先拿到
//!  什么凭证**——`Accord` 的目的地就是一个 `TaskId`，故"把 Pie 交出去"这一步是客人直接对
//!  持树者做的（板那一台也是这么交入口的）。
//!

pub mod call;
pub mod client;
// 形、据、账已搬进「约」——转出。
pub use contract::system::operator::{core, frame, gate, judge, ledger};

pub use call::{ASK_MARK, LINK, Listing, TIP_MARK, TIP_NAME};
pub use crate::system::operator::core::{EntryId, Fail, OpenedBy, Operator, Stamps, Unship, VestedBy, Where};
pub use crate::system::operator::gate::{Blind, Code, Control, verdict};
pub use crate::system::operator::judge::{Branch, Door, Id, League, Rule, Ruling, Who, judge};
pub use crate::system::operator::ledger::{Key, Ledger, Line, Owner};

/// **同步义务**：`gate.rs` 自己留了那三格线上码（它要在宿主靶里编，而 `call.rs` 拖着 `runtime`
/// 与 `session` ⇒ 编不动）。这里在编译期把两份钉在一起——真正的对照表只有 [`call`] 那一份，
/// `gate` 那一份一漂就编不过。宿主靶那一侧没有这条断言（它编不到 `call.rs`），所以它**必须**
/// 住在这里。
const _: () = {
    assert!(gate::WIRE_OK == call::OK);
    assert!(gate::WIRE_DENIED == call::DENIED);
    assert!(gate::WIRE_UNJUDGED == call::UNJUDGED);
};
