//! Principal Protocol — **策略身份**：这个 Task 此刻代表谁，以及这条身份从谁而来。
//! 它不定义权限（PrincipalId 的含义由各具体协议自己解释），不定义 User（User 是它之上的一种
//! 解释），也不定义任何领域语义。它只回答一个问题：**这个请求代表哪个策略主体。**
//! # 三条轴：名册、谱系、转换
//! ```text
//!   名册   TID ──→ 当前 PrincipalId        这个 Task 此刻代表谁        ← 内核盖章的那条管道
//!   谱系   PrincipalId ──父──→ PrincipalId     这条身份从谁而来           ← 只增不改的一棵树
//!   转换   名册那一格上的两句话          领（往下走）／弃（回起点）  ← 主体改自己，不碰树
//! ```
//! **名册**是这份协议与内核之间的那一条边：内核在 `Push` 那一刻给报文盖章，收方拿到的是
//! **TaskId**；名册把它翻成一条策略身份。**谱系**是身份之间的关系：一棵单根的树，`derive`
//! 长出新枝，父不可改、节点不删。**转换**是主体对自己那一格的两种写法，钥匙全在"发送者是谁"

// **判定与接口**（正文、七条原语、帧、客侧那一面、两面各一枚 `Grant`）住在这里；**实现方**（真在
// `prog-principal` 域里跑的那枚线程）住 `programs/src/system/principal/`。
// 装配侧（谁在什么时候 `derive` + `bind`）住 `programs/src/service/principal/bridge.rs`。

pub mod frame;

pub use frame::{Fail, PrincipalId};

pub mod grant;

pub use grant::{Grant, grant_of};

pub mod client;
// 形与据就在本模块树下（`grant` / `frame`）。

pub use frame::{
    BACK, BAD, DENIED, DIR, NAME, OK, Reply, Wire, code_to_fail, fail_to_code, reply_present,
};
