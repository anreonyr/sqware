//! program::source — **这一台的身子从哪来**（镜像里的一台 / 住本域的一枚内件）。
//!
//! `Program` 只声明来源，把它变成真的那一手归 [`Control`](crate::system::control::Control)。

use env::TaskId;

/// **身子那一格**：两条来路，与旧装配机器里那唯一的分叉逐字对应
/// （旧的 `match role { Some(role) => spawn_here, None => mint }`）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source {
    /// **镜像里的一台**：按清单名去清单里挑镜像（`Catalog::find`），建域 + 产线程。
    Catalog(&'static str),
    /// **住本域**的一枚内件**（iii：编排域的四枚线程里，除编排者自己以外那三枚）：
    /// 不建域，就在编排域自己的域里产一枚线程——四角色共用同一份 ELF，靠 `args` 分派。
    Here(Role),
}

/// **一枚内件是谁**——一枚 ELF 里四个角色，靠 `Spawn` 的那一格 `args` 分派。
///
/// 写它的是装配者，读它的是被产出的那枚线程（`system/main.rs` 的 `main`）——两侧共读
/// **同一处定义**（照 `Eyes` 的先例：`p.name == "principal"` 那种字符串比对，改个名字就
/// 静默失灵）。空 `args` ⇒ [`Role::System`]：引导域起编排者那一趟照旧传 `&[]`。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    /// 编排域自己那一枚（整个域的宿主线程）。
    System = 0,
    /// 持树者。
    Tree = 1,
    /// 名册（身份服务）。
    Roster = 2,
    /// 盟册（结盟服务）。
    League = 3,
}

impl Role {
    /// 递出去的那两格：角色 ＋ **起我那一枚线程**（编排者自己）。
    ///
    /// **照实记（第二格为什么要递，而不是问内核——这一格是实测逼出来的）**：内核的 `Sire`
    /// 答的是**域级**的生我者（`UnitCall::Sire` 读的是 `t.ident.team.sire()`）——对**住在本域**
    /// 的那一枚内件，它答的是**编排域的父域**（`root`），**不是"起我那一枚线程"**。第一版照
    /// 旧用 `sire()`，真机上当场红了：树那枚把提示孔交给了 `root`，装配者等到期限
    /// （读数 `operator` + `operator:tip` → `system: assemble`，`reason=0xa`）。故这一格由
    /// 装配者自己递进来（同一个域里，"谁起的我"不是内核记得的那一个）。
    pub const fn args(self, assembler: usize) -> [usize; 2] {
        [self as usize, assembler]
    }

    /// 读回来。**读不懂 ⇒ [`Role::System`]**（照旧那一趟传的是空）。
    pub const fn of_args(args: &[usize]) -> Role {
        match args.first() {
            Some(1) => Role::Tree,
            Some(2) => Role::Roster,
            Some(3) => Role::League,
            _ => Role::System,
        }
    }

    /// **起我那一枚线程**的号（第二格）。`None` = 照旧那一趟（域级的 `Sire` 才对）。
    pub const fn assembler(args: &[usize]) -> Option<usize> {
        match args {
            [_, who, ..] => Some(*who),
            _ => None,
        }
    }
}

/// **起我那一枚线程**是谁（内件用）——从 `Spawn` 那一格 `args` 里读（见 [`Role::args`] 的照实记）。
///
/// **为什么不是 `runtime::env::unit::sire()`**：那一手答的是**域级**的生我者（见 [`Role::args`]）。
/// 三枚内件要的是"起我那一枚线程"，因为它们的孔要交给**编排者那一枚**，不是交给编排域的父域。
pub fn assembler() -> Option<TaskId> {
    Role::assembler(runtime::core::unit::args()).map(TaskId::new)
}

// **写出去与读回来同源**（"常量交给编译器"）：四条各绕一圈，写反一位**编不过**。
const _: () = {
    let all = [Role::System, Role::Tree, Role::Roster, Role::League];
    let mut i = 0;
    while i < all.len() {
        assert!(Role::of_args(&all[i].args(0)) as usize == i);
        i += 1;
    }
};
