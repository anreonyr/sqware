//! needs — **装配契约**：谁要哪几枚门闩、什么权、以什么形态出去。
//!
//! 装配者（`prog-root`）按它发货，收方（如 `prog-plic`）按它认领。它住在 supervisor 的
//! 目录里、由两个 bin 各声明一次（见 `root/main.rs` 与 `plic/main.rs` 头部的 `#[path]`），
//! 是因为它是**两端必须对上的那件事**——设备语义（寄存器布局 / 线号 / 设备树解析）仍归
//! 各自的域（`plic/plic.rs`、`plic/uart.rs`）。
//!
//! # 名字从哪来
//!
//! `name` 是 **boot 在配对块里给的原样**（设备树节点 basename；`devicetree` / `irq` 两条
//! 由内核定，见 `platform/devices.rs` 的 `DTB_NAME` / `IRQ_NAME`）。本模块不发明名字，
//! 只把「谁要的、多少权、怎么出去」与它们对起来。
//!
//! # 形态（`policy`）
//!
//! `CAGE` = **交出**：授出方那份在交出期间不可用，子枚消亡则源枚自动复原——故它声明的
//! 是"装配者只是保管人，不是使用者"。寄存器一类的门闩同一时刻只该有一个使用者，故用
//! `CAGE`；自描述（设备树）天然多读者，用 `NONE`（转授，源枚照旧可用）。

use runtime::core::port::{Access, Policy};

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

/// 门闩的种类——授出时要挑对那一层句柄（`PolePie` / `NolePie`）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// 一段内存（设备寄存器页 / 自描述区）。
    Pole,
    /// 空载荷的信号（中断门铃）。
    Nole,
}

/// 一条需求：**谁要的** + **boot 给的名字** + 种类 + 权 + 出去的形态。
// 两个 bin 共享本文件：发货方（root）读全部字段，收货方（plic）只读 `slot` / `name`
// ——各自只用到一半是正常的，不是死代码。
#[allow(dead_code)]
#[derive(Clone, Copy, Debug)]
pub struct Need {
    /// 收方那一格的编号。
    pub slot: Slot,
    /// boot 给的名字（配对块里的键）。
    pub name: &'static str,
    /// 门闩的种类。
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
        access: Access::READ_WRITE,
        policy: Policy::CAGE,
    },
    Need {
        slot: Slot::Dtb,
        name: "devicetree",
        kind: Kind::Pole,
        access: Access::READ,
        policy: Policy::NONE,
    },
    Need {
        slot: Slot::Bell,
        name: "irq",
        kind: Kind::Nole,
        access: Access::READ,
        policy: Policy::NONE,
    },
    Need {
        slot: Slot::Source,
        name: "serial@10000000",
        kind: Kind::Pole,
        access: Access::READ_WRITE,
        policy: Policy::CAGE,
    },
];
