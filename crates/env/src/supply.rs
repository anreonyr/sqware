//! supply — 供给那一族的**词汇**：线上那一条（[`Want`]）与它那两格判别号（[`Kind`]）。
//!
//! # 为什么它住 `env`
//!
//! 装配那一侧要**两侧读**——内核的 `build.rs`（宿主）与编排域（riscv）——而 `programs` /
//! `protocol` 都拖着 `runtime`（riscv 内联汇编，宿主上编不过）。故凡是"装配那一侧要摆出来的
//! 东西"，定义都得住 `env`。
//!
//! # 照实记（`Need` / `At` / `class_block` 三样**退场了**）
//!
//! 这一族原先还有**收方那张 `const` 单**：`Need`（坐标未定："凭什么认它"，`At::Class` /
//! `At::Known`）＋ `class_block`（把 `compatible` 串补成定长块）＋ `Need::settle`（读树那一侧
//! 把类翻成坐标）。它们服务的是一条**装配者替所有驱动认设备**的路：装配表摆出每一台要哪几样，
//! 编排域逐条 settle、向引导域领、再按位次递下去。
//!
//! 那一条路整个退了（设备那一轴今天由**设备账**那一台在运行期回答，取法是驱动自己跑一趟）——
//! 于是"坐标未定的单子"没有读者了：**装配者只替一台领全机**（`Setup::Machine`），而它领什么
//! 由**设备树**说了算（`system::machine::Machine::devices`），不由哪张 `const` 表说。
//! **机制退了，格也退**。
//!
//! 留下的是**线上那一形**：`Want`（坐标已定的一条）＋ `Kind`（那一格的判别号）——递单那一趟
//! 仍要走（引导域那一侧一个字没动）。

use crate::key::Key;
use crate::wire::Field;
use crate::{Access, Policy};

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

impl Kind {
    /// 线上那一格解回（**判别号不认识 ⇒ `None`**——同 [`Want::key`] 那条口径：读的人按
    /// "这一帧读不懂"处置，不猜）。
    ///
    /// **它为什么住类型自己身上**（照实记：impl 跟着类型走）：这一格的读者从一条变成两条
    /// （`Want::kind` 与设备账那一族认领那一帧的 `kind`）——两处各写一遍 `match 0/1/…` 就是
    /// 两份判别号表。
    pub const fn of(raw: u8) -> Option<Kind> {
        match raw {
            KIND_POLE => Some(Kind::Pole),
            KIND_NOLE => Some(Kind::Nole),
            _ => None,
        }
    }
}

/// 单子上的一条：**要哪一枚**（坐标已定）、什么种类、多少权、什么形态。
///
/// 它是**线上形**：坐标已经落定。
/// 故这一条**不带 `slot`**——**位置即格**（回单与单子同序同长），收方按位次归位
/// （`protocol::system::grant::each`）。
///
/// `repr(C)` + 定长字段 ⇒ 尺寸即线格式（编译期断言锁死），与 `Pair` 同一条纪律。
///
/// **留白 7 字节是明的**（`kind` 之后）：它把记录填满 32——**不许有隐式尾巴**。那一格是要整条按
/// 字节搬的（见下面的 `Field`），隐式留白就是**未初始化字节上线**（照实记：坐标从 32 字节的名字块
/// 换成 16 字节的 `Key` 之后，对齐从 4 跳到 8，尾巴那 4 字节是这么冒出来的；补一格明的就没了）。
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

    /// 线上那一条：坐标已定（装配者按**设备树**定出来——`system::machine` 那张表，或
    /// boot 那两件按已知坐标）。
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
        Kind::of(self.kind)
    }

    pub fn access(&self) -> Option<Access> {
        Access::from_bits(self.access)
    }

    /// 形态**原样**读出（发货那一侧照它授出：`VEST` / `ONLY` 都是**请求方说**的，固件不替它挑
    /// ——见 `programs/src/root/supply/server.rs` 那条照实记）。
    pub fn policy(&self) -> Option<Policy> {
        Policy::from_bits(self.policy)
    }
}

/// 单子上一条的宽度：**尺寸即线格式**（`repr(C)` ＋ 定长字段 ＋ 一格明的留白）。
pub const WANT_LEN: usize = size_of::<Want>();

/// **各项之和就是它**——若留一格隐式的尾巴，整条按字节搬就会把未初始化字节读上线（见 [`Want`] 的注）。
const _: () = assert!(WANT_LEN == 32);
const _: () = assert!(WANT_LEN == crate::key::KEY_LEN + 4 + 4 + 1 + 7);

/// 线上那一条的那一格（单子那一段尾巴要 `T: Field`，见 `protocol::system::supply::frame`）。
///
/// **照实记（为什么可以整条按字节搬）**：[`Want`] 是 `repr(C)`、尺寸由上面那两条编译期断言钉死，
/// 而且**各字段之和 == 尺寸**（没有隐式留白）⇒ 按字节写满、按字节读回都合法。这一手从前散在
/// `protocol` 那一侧（`want_bytes` 的只读视图 ＋ `Order::want` 里的 `read_unaligned`）——今天收在
/// 类型自己身上（"impl 跟着类型走"）。
impl Field for Want {
    const WIDTH: usize = WANT_LEN;

    fn store(&self, out: &mut [u8]) {
        // SAFETY: 同上（`repr(C)`、无隐式留白、字段全是 POD）。
        let raw = unsafe { &*(self as *const Want).cast::<[u8; WANT_LEN]>() };
        out.copy_from_slice(raw);
    }

    fn fetch(bytes: &[u8]) -> Option<Self> {
        let raw = bytes.get(..WANT_LEN)?;
        let mut want = Want::NONE;
        // SAFETY: 同 `store`；`copy_nonoverlapping` 把那 `WANT_LEN` 字节写满。
        unsafe {
            core::ptr::copy_nonoverlapping(
                raw.as_ptr(),
                (&mut want as *mut Want).cast::<u8>(),
                WANT_LEN,
            );
        }
        Some(want)
    }
}
