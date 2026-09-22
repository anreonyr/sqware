//! key — **坐标**：一条供给记录里"它是哪一件"的那一格（配对块、单子、回单、认线四处同一个）。
//!
//! 三形（**判别号即线格式**，见 [`REGION`] / [`DTB`] / [`IRQ`]）：
//!
//! ```text
//!   region(base)  机器摆的事实：设备寄存器页的 `reg` 段起点、载荷区（`/chosen` 的 start）
//!   dtb()         设备树本体（**引子**：正因为不知道它的地址，才要读树）
//!   irq()         中断门铃（空载荷，没有区）
//! ```
//!
//! **为什么只取基址、不带长度**：区不重叠 ⇒ 基址就是键；而"那一段多长"是**算出来的**
//! （内核把 `initrd` 的 `end` 向上取整到页、还要求 `start` 页对齐）——把长度放进键，就等于
//! 要求两侧把那个算法算得一模一样。基址是直接读到的那一格，不需要任何规则。
//!
//! **为什么后两形按"哪一件"取**：它们在设备树里**没有坐标**——树本体不知道自己的地址，
//! 门铃压根没有区。故它们的坐标就是**它的唯一性**（这一类里只有这一件）。再多一棵树 /
//! 再一枚铃，这一形就得换（那时才有"第几件"的问题）。
//!
//! 字节一份定义在[本模块](Key::bytes)：记录（[`super::pair`]）与各帧都从这里取，不各写一遍。

use core::mem::size_of;

/// 坐标的字节数（判别号 1 + 留白 7 + 那一个数 8）。
pub const KEY_LEN: usize = 16;

/// 判别号：**区**（`at` 是基址）。
pub const REGION: u8 = 0;
/// 判别号：**设备树本体**。
pub const DTB: u8 = 1;
/// 判别号：**中断门铃**。
pub const IRQ: u8 = 2;

/// 坐标：判别号 + 一个数（`repr(C)` + 定长 ⇒ 字节即线格式，编译期断言锁死）。
///
/// 留白那一格是**明的**：三形共用 16 字节，非区支的 `at` 为 0（未用，不是"含义会变"）。
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Key {
    code: u8,
    pad: [u8; 7],
    at: u64,
}

/// 尺寸即线格式（记录与帧的步长都从它推）。
const _: () = assert!(size_of::<Key>() == KEY_LEN);

impl Key {
    /// 空的一格：填数组用（判别号 0xFF 读侧认不出 ⇒ **判废**；与 `PieToken(0)` 同一条约定）。
    ///
    /// **照实记**：这是**构造侧**的哨兵（填定长数组、判废用），不是配对块里的约定——块是
    /// **零填**的，而 [`Key::of`] 认 `region(0)`（判别号 0 + 数 0）。今天读侧按**条数**切片
    /// （每条都是内核写完的），故空槽根本读不到；哪天有空槽进来，`region(0)` 也取不到真东西
    /// ——基址 0 不代表一段区（内核扫树时正是把零址段跳掉的）。
    pub const NONE: Key = Key {
        code: 0xFF,
        pad: [0u8; 7],
        at: 0,
    };

    /// 按区取：那一段的基址（设备 `reg` 段 / 载荷区 `/chosen` 的 start）。
    pub const fn region(base: u64) -> Key {
        Key {
            code: REGION,
            pad: [0u8; 7],
            at: base,
        }
    }

    /// 按"哪一件"取：设备树本体。
    pub const fn dtb() -> Key {
        Key {
            code: DTB,
            pad: [0u8; 7],
            at: 0,
        }
    }

    /// 按"哪一件"取：中断门铃。
    pub const fn irq() -> Key {
        Key {
            code: IRQ,
            pad: [0u8; 7],
            at: 0,
        }
    }

    /// 线格式那一对：判别号 + 那个数（**只有区支的第二个数有意义**，写侧用）。
    pub const fn parts(self) -> (u8, u64) {
        (self.code, self.at)
    }

    /// 从线上的两格读回来：判别号不认识 ⇒ `None`（记录判废，不猜）。
    pub fn of(code: u8, at: u64) -> Option<Key> {
        match code {
            REGION => Some(Key::region(at)),
            DTB => Some(Key::dtb()),
            IRQ => Some(Key::irq()),
            _ => None,
        }
    }

    /// 那一截的**基址**——只有区支有；`dtb` / `irq` 没有区 ⇒ `None`。
    pub fn base(self) -> Option<u64> {
        (self.code == REGION).then_some(self.at)
    }

    /// 本坐标的字节（记录与各帧那一格）。
    pub fn bytes(self) -> [u8; KEY_LEN] {
        // SAFETY: `Key` 是 `repr(C)`、尺寸由编译期断言锁死 = `KEY_LEN`，逐字节读一份副本。
        unsafe { core::ptr::read_unaligned((&self as *const Key).cast::<[u8; KEY_LEN]>()) }
    }

    /// 从字节读回来：**判别号不认识 ⇒ `None`**（判废）。
    pub fn from_bytes(raw: [u8; KEY_LEN]) -> Option<Key> {
        // SAFETY: 尺寸相等（编译期断言），`read_unaligned` 不要求对齐；`code`/`at` 都是
        // 平凡位模式，读出来交给 `of` 校验。
        let k = unsafe { core::ptr::read_unaligned(raw.as_ptr().cast::<Key>()) };
        Key::of(k.parts().0, k.parts().1)
    }
}
