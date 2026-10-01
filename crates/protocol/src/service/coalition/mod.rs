//! 一组身份共同参与一件事。
//! 它是身份之间的**横向**关系（system::principal 那一条是**纵向**的：从谁而来）。
//! 它不定义权限、不产生 PrincipalId、不发 Pie、不解释成员资格的含义，也不负责**发现**——
//! 它只回答一个问题：**这一组身份里有没有它。**
//! # 一条关系、两个方向、三格写三格读
//! **只有一张表**：反着念就是第二个方向。principal 要三个轴名（名册 / 谱系 / 转换），是因
//! **号**（`CoalitionId`）是服务发的一枚裸号，铸过就一直在："这枚盟存在"= 号落在那枚计数器
//! 之下。故**空盟合法**：铸一枚空盟什么也不产生，也没有"散掉"这回事。
//! # 七条原语
//! | 原语 | 字 | 干什么 | 钥匙 | 上线 |
//! |---|---|---|---|---|
//! | Coalition::found | 立 | 铸一枚新号（空盟），**并记下立它那一位** | 发送者**已绑定**（适配层） | 是 |
//! | Coalition::enter | 入 | 把**发送者此刻代表的那条身份**放进 `c` | 发送者那一格 | 是 |
//! | Coalition::admit | 代报名 | 把**另一位**放进 `c` | **你是这一枚的盟主** | 是 |
//! | Coalition::leave | 出 | 把它从 `c` 拿出来（撞空也成） | 发送者那一格 | 是 |
//! | Coalition::amid | 在 | `p` 在不在 `c` 里 | 无（读是公开的） | 是 |
//! | Coalition::band | 员 | `c` 的成员（**一趟取窗**：号序 + 阈值游标） | 无 | 是 |
//! | Coalition::bloc | 籍 | `p` 的盟籍（同上） | 无 | 是 |
//! **写只有四条，读全是公开的**：答案不是秘密，本协议不授予任何东西——故读那三条里
//! **没有"对端说了不"这一档**，唯一会答的失败是"这枚号没铸过"（Fail::Unknown）；

// `prog-coalition` 域里跑的那枚线程）住 `programs/src/system/coalition/`。
// 装配侧（谁在什么时候 `derive` + `bind`）住 `programs/src/service.rs`。

pub mod frame;

pub use frame::{CoalitionId, Fail, WINDOW_CAP, Window};

pub mod grant;

pub use grant::{Grant, grant_of};

pub mod client;

pub use frame::{BACK, BAD, DENIED, DIR, NAME, OK, Reply, Union, Wire, code_to_fail, fail_to_code};
