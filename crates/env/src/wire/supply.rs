//! supply — 供给那一族的**词汇**：收方那张 `const` 表里的一格（[`Need`]）与线上那一条
//! （[`Want`]），外加把 `compatible` 串补成定长块的 [`class_block`]。
//!
//! # 为什么它住 `env`
//!
//! 装配单（[`crate::assembly`]）要**两侧读**——内核的 `build.rs`（宿主）与编排域（riscv）——
//! 而 `programs` / `protocol` 都拖着 `runtime`（riscv 内联汇编，宿主上编不过）。故凡是
//! "装配单要摆出来的东西"，定义都得住 `env`。
//!
//! **照实记（这几样是从 `protocol::driver::supply::call` 搬下来的）**：那一处现在是
//! `pub use` 转发，**调用点一行没改**（与 `Access`/`Policy`、`Announce`/`Grant`/`Died` 同一条
//! 先例）。搬的时候把 [`Want`] **连同它的 `impl` 一起带走**——`impl` 是 inherent 的，必须与
//! 类型同住一个 crate，劈开就要改 API；`Need::settle` 正是这么依赖它的。
//!
//! **照实记（`Need` 不是线格式）**：它没有 `repr(C)`、尺寸不参与任何断言，唯一的义务是能被
//! `const` 造出来；线上那一条是 [`Want`]（`repr(C)` + 定长字段，尺寸即线格式）。

use crate::wire::access::{Access, Policy};
use crate::wire::key::Key;
use crate::wire::name::{NAME_LEN, Name};

const KIND_POLE: u8 = Kind::Pole as u8;
const KIND_NOLE: u8 = Kind::Nole as u8;

/// 单子上"哪一类东西"那一格（**判别号即线格式**：`repr(u8)`）。
///
/// 两格对应内核那两种门闩句柄（`PolePie` / `NolePie`）——固件据此挑对那一层。
///
/// **照实记（第三格为什么退了）**：从前还有一格 `Hole`（孔），它是**持树者那笔提示之路**的
/// 格子——那一笔从前也经这条供给路发（引导域先认下来、编排域来要时才发）。树改成**编排域
/// 自己起**的服务之后那一笔整条退了，可它占的格还留着：全仓**没有一处构造 `Kind::Hole`**，
/// 只有发货端那个 match 臂在接一个永远不会来的东西。本笔删掉它：**机制退了，格也退**。
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// 一段内存（设备寄存器页 / 自描述区 / 载荷区）。
    Pole,
    /// 空载荷的信号（中断门铃）。
    Nole,
}

/// 那一格写的是什么：**树里认的类**，还是**已经知道的坐标**。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum At {
    /// **树里认**：`compatible` 串（`ns16550a`、`virtio,mmio`…）——要读树才定得下来。
    Class([u8; NAME_LEN]),
    /// **已经知道**：boot 造的那两件（设备树本体 / 门铃）——它们不在树里，没有"哪一台"可翻。
    Known(Key),
}

/// 收方开的单上那一格：**坐标还没定下来**——写的是"凭什么认它"。
///
/// 它与 [`Want`] 是**两个东西**：`Want` 是线上那一条（坐标已定），`Need` 是收方那张 `const`
/// 表里的一格（坐标未定）。故 `Need` **不是线格式**：没有 `repr(C)`、尺寸不参与任何断言，
/// 唯一的义务是能被 `const` 造出来。
///
/// 坐标怎么定见 [`Need::settle`]：按类要的那几条在**编排域**读树定（设备树只有它读了），
/// 已经知道坐标的那几条原样落下。
#[derive(Clone, Copy)]
pub struct Need {
    at: At,
    kind: Kind,
    access: Access,
    policy: Policy,
}

/// 单子上的一条：**要哪一枚**（坐标已定）、什么种类、多少权、什么形态。
///
/// 它是**线上形**：坐标已经落定（收方那张表里那一格 [`Need`] 经 [`Need::settle`] 定过）。
/// 故这一条**不带 `slot`**——**位置即格**（回单与单子同序同长），收方按位次归位
/// （`protocol::system::grant::each`）。
///
/// `repr(C)` + 定长字段 ⇒ 尺寸即线格式（编译期断言锁死），与 `Pair` 同一条纪律。
///
/// **留白 7 字节是明的**（`kind` 之后）：它把记录填满 32——**不许有隐式尾巴**。`want_bytes`
/// 按整条读，隐式留白就是**未初始化字节上线**（照实记：坐标从 32 字节的名字块换成 16 字节的
/// `Key` 之后，对齐从 4 跳到 8，尾巴那 4 字节是这么冒出来的；补一格明的就没了）。
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Want {
    key: Key,
    access: u32,
    policy: u32,
    kind: u8,
    pad: [u8; 7],
}

impl Want {
    /// 空的一条：填数组用（坐标是 [`Key::NONE`] = 判废的判别号，读侧一律答 `None`）。
    pub const NONE: Want = Want {
        key: Key::NONE,
        access: 0,
        policy: 0,
        kind: KIND_POLE,
        pad: [0u8; 7],
    };

    /// 线上那一条：坐标已定（由 [`Need::settle`] 定出来，或编排域自己按坐标要）。
    pub fn new(key: Key, kind: Kind, access: Access, policy: Policy) -> Want {
        Want {
            key,
            access: access.bits().bits(),
            policy: policy.bits().bits(),
            kind: kind as u8,
            pad: [0u8; 7],
        }
    }

    /// 坐标（**判别号不认识 ⇒ `None`**——帧读不懂那一格，调用方按 `Bad` 处置）。
    pub fn key(&self) -> Option<Key> {
        Key::of(self.key.parts().0, self.key.parts().1)
    }

    pub fn kind(&self) -> Option<Kind> {
        match self.kind {
            KIND_POLE => Some(Kind::Pole),
            KIND_NOLE => Some(Kind::Nole),
            _ => None,
        }
    }

    pub fn access(&self) -> Option<Access> {
        Access::from_bits(self.access)
    }

    /// 形态**原样**读出；"剔掉 `VEST`"是发货那一侧（`protocol::driver::supply`）的事。
    pub fn policy(&self) -> Option<Policy> {
        Policy::from_bits(self.policy)
    }
}

/// 编译期把 `compatible` 串补零成定长块——与 [`Name::new`] 运行期做的是同一件事。
///
/// **不做** `Name::new` 那套校验（非空 / UTF-8 / ≤ `NAME_LEN` - 1）：它是给 `const` 表用的，
/// 字面量写错会在 [`Name::from_bytes`] 读出时当场暴露（表是编译期常量，读的人就在旁边）。
pub const fn class_block(s: &str) -> [u8; NAME_LEN] {
    let src = s.as_bytes();
    let mut out = [0u8; NAME_LEN];
    let mut i = 0;
    while i < src.len() && i < NAME_LEN - 1 {
        out[i] = src[i];
        i += 1;
    }
    out
}

impl Need {
    /// 按类要一条（用 [`class_block`] 写）。
    pub const fn class(class: [u8; NAME_LEN], kind: Kind, access: Access, policy: Policy) -> Need {
        Need {
            at: At::Class(class),
            kind,
            access,
            policy,
        }
    }

    /// 按**已知坐标**要一条（boot 造的那两件：`Key::dtb()` / `Key::irq()`）。
    pub const fn known(key: Key, kind: Kind, access: Access, policy: Policy) -> Need {
        Need {
            at: At::Known(key),
            kind,
            access,
            policy,
        }
    }

    /// 这一格写的是哪一类（**只有类支有**；读数"类 → 坐标"那一行要它当左半）。
    pub fn class_name(&self) -> Option<Name> {
        match self.at {
            At::Class(class) => Name::from_bytes(class).ok(),
            At::Known(_) => None,
        }
    }

    /// 定坐标：`Class` 交给 `of`（读树的那一侧），已经知道的原样落进 [`Want`]。
    ///
    /// 返 `None` = **这台机器上没有这一类**（本层的失败域，不是引导域的答话——那一格是
    /// `Fail::Unknown`）。
    pub fn settle(self, of: impl Fn(&str) -> Option<Key>) -> Option<Want> {
        let key = match self.at {
            At::Class(class) => {
                let class = Name::from_bytes(class).ok()?;
                of(class.as_str())?
            }
            At::Known(key) => key,
        };
        Some(Want::new(key, self.kind, self.access, self.policy))
    }
}
