//! System Protocol — **服务编排**：整个系统中由哪些 Service 构成，以及它们如何被组织、
//! 依赖、启动、停止、替换与监督。
//! 它不创造 Task。Service 的**执行载体**是 `Task`（一个域 = `Team`），载体由内核的
//! Unit ABI 产生；本协议把那些载体**组织成系统**。故两层要彻底分开：
//! ```text
//!   内核 ABI（机制）     Unit                 Task 怎么创建、运行、结束
//!        │
//!   本协议（编排）       Service              系统由哪些 Service 组成、怎么运行
//!        │                                     （判定与账在本文这一侧）
//!   实现方               programs/src/system/ 起一条、看/判、放下、收场
//! ```
//! **它容纳那四套协议**（[`control`] / [`operator`] / [`principal`] / [`coalition`]——用户裁定）：
//! 判据是"**谁住编排域**"。iii 之后这四套的落地都是**编排域里的线程**（control 那一枚线程、
//! 持树者 / 名册 / 盟册），而"**要找服务得先有目录**——今天那本目录就是树（`operator`）"这句
//! 也写在本正文里。故协议树与实现树（`programs/src/system/`）**同形**：本层与那四套同一份屋顶。
//! ```text
//!   System Protocol = Control + Principal + Coalition + Operator     （四轴，平级）
//!     Control     系统里有什么 Service，它们处于什么生命状态
//!     Principal   一个 Task / 请求代表谁
//!     Coalition   哪些身份形成横向关系
//!     Operator    名字如何指向资源
//!   System 本身没有第五种对象：不是 kernel object、不是统一 Client、不负责通信机制。
//! ```

/// **服务那一层那一段路**（一条路：`/svc`）——挂在树上的服务都从它起。
pub const SVC: &Path = Path::new("svc");

/// **平台自己那几枚在容器底下那一段**（`sys`）：持树者（`operator`）与三枚内件
/// （名册 / 盟册 / 控制面）都从它起 —— `/svc/sys/{operator,principal,coalition,control}`。
pub const SYS: &str = "sys";

/// **那四族共用那段前缀**（`/svc/sys`）：`operator` / `principal` / `coalition` / `control`
/// 各自那一段路（各族自己的 `DIR`）都从它起 —— **只此一处**。
pub const DIR: &Path = Path::new("svc/sys");

use crate::service::operator::path::Path;

pub mod control;
pub mod faces;
pub mod supply;

/// **面之外那几枚记号**（不归某族"面"那一族、却被当记号用的）。
const LOOSE: &[env::Mark] = &[
    crate::driver::ENTRY_MARK,
    crate::service::operator::TIP_MARK,
    control::ASK_MARK,
    control::BACK,
    crate::service::principal::BACK,
    crate::service::coalition::BACK,
    crate::service::operator::ASK_MARK,
];

/// **全协议任两枚记号不许撞**：四族的面 × 别族的面 × 上面那几枚散记号，逐对判一次。
const _: () = {
    let fams: [&[env::Mark]; 4] = [
        &crate::service::coalition::Grant::MARKS,
        &control::Grant::MARKS,
        &crate::service::operator::Grant::MARKS,
        &crate::service::principal::Grant::MARKS,
    ];
    let mut f = 0;
    while f < fams.len() {
        let a = fams[f];
        let mut i = 0;
        while i < a.len() {
            // 一、与**后面**各族的面（本族内部那一条由 `faces!` 自己判）。
            let mut g = f + 1;
            while g < fams.len() {
                let b = fams[g];
                let mut j = 0;
                while j < b.len() {
                    assert!(a[i].get() != b[j].get(), "system: two faces share one mark");
                    j += 1;
                }
                g += 1;
            }
            // 二、与面之外那几枚。
            let mut j = 0;
            while j < LOOSE.len() {
                assert!(
                    a[i].get() != LOOSE[j].get(),
                    "system: a face and a loose mark share one mark"
                );
                j += 1;
            }
            i += 1;
        }
        f += 1;
    }
    // 三、面之外那几枚彼此。
    let mut i = 0;
    while i < LOOSE.len() {
        let mut j = i + 1;
        while j < LOOSE.len() {
            assert!(
                LOOSE[i].get() != LOOSE[j].get(),
                "system: two loose marks share one mark"
            );
            j += 1;
        }
        i += 1;
    }
};
