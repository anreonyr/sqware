//! 配对块——boot 交给 root 的**设备供给账**（一条 = 一台设备）。
//!
//! 这不是 envcall 载荷，而是**启动期借映块**的线格式：内核 boot 扫一次设备树，
//! 把每个 (节点, `reg` 段) 做成一枚门闩，再把「名字 + 我持它的 token」写成定长
//! 记录、只读借映进 root 的空间（与 initrd 清单视图同一套机制）。
//!
//! ```text
//! [0..32)  name   [u8; NAME_LEN]  节点 basename（含 `@unit-address`），NUL 填充
//! [32..40) token  usize LE        该设备门闩**在 root 表里**的句柄
//! ```
//!
//! 为什么格式定义在 `env::wire`（而不是内核与 root 各写一遍）：两个字段都是本模块
//! 已有的类型（[`Name`] 与 [`PieToken`]），两边共用一份定义即无第二份账——
//! 内核只写不读、root 只读不写，**各自都不解释设备语义**（名字是 DTB 原样搬运）。
//!
//! `token = 0` 是无效哨兵（`PieToken` 的约定），故有效记录恒有非零 token。

use super::{NAME_LEN, Name, PieToken};

/// 一条记录的字面字节数（`NAME_LEN` + 8）。
pub const PAIR_LEN: usize = NAME_LEN + size_of::<usize>();

/// 配对块的一条：名字 + 我持它的句柄。
///
/// `repr(C)` + 两个定长字段 ⇒ 尺寸即 [`PAIR_LEN`]（编译期断言锁死），内核可直接把
/// 记录数组写进借映块、root 直接按块读，不需要序列化步骤。
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Pair {
    name: [u8; NAME_LEN],
    token: PieToken,
}

/// 尺寸即线格式（`PAIR_LEN` 是 root 侧的步长，写错即整块错位）。
const _: () = assert!(size_of::<Pair>() == PAIR_LEN);

impl Pair {
    /// 造一条（内核侧）：名字已校验（[`Name`] 是构造期义务）。
    pub fn new(name: Name, token: PieToken) -> Self {
        Self {
            name: *name.bytes(),
            token,
        }
    }

    /// 读一条（root 侧）：名字非法（空 / 超长 / 填充不规范 / 非 UTF-8）→ `None`。
    ///
    /// 不 panic：块来自 boot，而 root 是它的读者——读到坏记录应当**就地判废**，
    /// 由 root 决定是跳过还是拒绝启动（与本仓「非法输入落在返回值上」一致）。
    pub fn name(&self) -> Option<Name> {
        Name::from_bytes(self.name).ok()
    }

    /// 该设备门闩在**本任务**表里的句柄。
    pub const fn token(&self) -> PieToken {
        self.token
    }
}
