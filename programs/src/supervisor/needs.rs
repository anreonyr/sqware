//! needs — **装配契约**：谁要哪几枚门闩、什么权、以什么形态出去。
//!
//! 编排域（`prog-system`）按它开单子，收方（如 `prog-plic`）按它认领；**发货的是引导域**
//! （原件在它手里）。三个 bin 都用它——它住在这里（lib 的 [`crate::supervisor`]）是因为它是
//! **两端必须对上的那件事**：设备语义（寄存器布局 / 线号 / 设备树解析）仍归各自的域
//! （`plic/plic.rs`、`plic/uart.rs`）。
//!
//! # 名字从哪来
//!
//! `name` 是 **boot 在配对块里给的原样**（设备树节点 basename；`devicetree` / `irq` 两条
//! 由内核定，见 `platform/devices.rs` 的 `DTB_NAME` / `IRQ_NAME`）。本模块不发明名字，
//! 只把「谁要的、多少权、怎么出去」与它们对起来。
//!
//! # 形态（`policy`）
//!
//! 传递族里**只有 `VEST` 一位是这里的选择**（对端能不能再授出）；形态由**源枚**定：
//! `ONLY` 是资源事实——内核在造寄存器页门闩时就给了它（"同一时刻只该有一个使用者"），
//! 故那两条写成 `Policy::ONLY`：与源枚一致 ⇒ 这次是**移交**（授出方那份在子枚存活期间
//! 不可用，子枚消亡则源枚自动复原；装配者只是保管人，不是使用者）。
//! **写错会被当场拒**：对独占资源写 `NONE` ⇒ `Accord` 答 `Denied`——这是显式失败，
//! 不是静默复制。自描述（设备树）与中断门铃天然多读者，用 `NONE`（复制，源枚照旧可用）。

use runtime::core::port::{Access, Policy};

use protocol::firmware::call::Want;

/// 门闩的种类住在**协议那一侧**（它是线格式里的一格）：本模块再导出，好让两端的代码
/// 仍旧写 `needs::Kind`。
pub use protocol::firmware::call::Kind;

/// 收方给这枚门闩起的名字——**按它归位，不靠位置约定**。
///
/// 判别号即收方那张表的数组下标；收方只用 `Slot` 取自己的格子，不数第几条。
#[repr(usize)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Slot {
    /// 中断控制器那一页寄存器。
    Plic = 0,
    /// 设备树本体（只读自描述）。
    Dtb = 1,
    /// 中断门铃（空载荷）。
    Bell = 2,
    /// 第一刀的测试源（UART 的中断使能位）。
    Source = 3,
}

/// 一条需求：**谁要的** + **boot 给的名字** + 种类 + 权 + 出去的形态。
#[derive(Clone, Copy, Debug)]
pub struct Need {
    /// 收方那一格的编号。
    pub slot: Slot,
    /// boot 给的名字（配对块里的键）。
    pub name: &'static str,
    /// 门闩的种类（线格式那一格，定义在 `protocol::firmware::call`）。
    pub kind: Kind,
    /// 要多少权。
    pub access: Access,
    /// 以什么形态出去。
    pub policy: Policy,
}

/// 中断面域（`prog-plic`）的需求单。
pub const PLIC: &[Need] = &[
    Need {
        slot: Slot::Plic,
        name: "interrupt-controller@c000000",
        kind: Kind::Pole,
        access: Access::FETCH_STORE,
        policy: Policy::ONLY,
    },
    Need {
        slot: Slot::Dtb,
        name: "devicetree",
        kind: Kind::Pole,
        access: Access::FETCH,
        policy: Policy::NONE,
    },
    Need {
        slot: Slot::Bell,
        name: "irq",
        kind: Kind::Nole,
        access: Access::FETCH,
        policy: Policy::NONE,
    },
    Need {
        slot: Slot::Source,
        name: "serial@10000000",
        kind: Kind::Pole,
        access: Access::FETCH_STORE,
        policy: Policy::ONLY,
    },
];

impl Need {
    /// 需求单的一条 → 单子上的一条（名字装不下 ⇒ `None`，不 panic）。
    ///
    /// 这一格属**这台机器**：`slot` 与设备名都是它的账；线格式那一半在协议里。
    pub fn want(&self) -> Option<Want> {
        Want::new(self.name, self.kind, self.access, self.policy)
    }
}

/// 名字 → 收方那本账里的第几格（配给那一段记录的解码要用它）。
pub fn slot_of(name: &str) -> Option<usize> {
    PLIC.iter()
        .find(|n| n.name == name)
        .map(|n| n.slot as usize)
}
