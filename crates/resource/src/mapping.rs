//! 映射：一枚共享页借映进本域，得到一段视图。

use core::marker::PhantomData;

use env::PieToken;

use crate::{mapping_kernel::MappingKernel, view::View};

/// 泊位：一枚 Pole 门闩 + 它在本域的那段视图。
pub struct Dock<K: MappingKernel> {
    pie: PieToken,
    view: View,
    _kernel: PhantomData<fn() -> K>,
}

impl<K: MappingKernel> Dock<K> {
    /// 映射：把一枚已授权的共享页借映进本域 → 视图。
    ///
    /// 收下 `pie`（本层持门闩）。同一枚门闩再开一次是幂等的
    /// （内核返同一个 VA），调用方仍须协调撤图。
    ///
    /// # Errors
    /// - `Denied` — 门闩不在本任务表里 / 权限不含 `FETCH`
    /// - `Dead`   — 资源已封印
    /// - `HandedOver`  — 这一枚被我交出去了（`ONLY` 资源的锚还在、接收方手里那枚还活着）
    /// - `OoM`    — 本域空间备不出这么长的一段
    pub fn open(pie: PieToken) -> Result<Dock<K>, K::Error> {
        let (base, size) = K::open(pie)?;
        Ok(Dock {
            pie,
            view: View::new(base, size),
            _kernel: PhantomData,
        })
    }

    /// 把视图拷一份交出去——驱动要的就是它（`View` 是 `Copy`，故借 `&self`）。
    pub fn view(&self) -> View {
        self.view
    }

    /// 这一枚页的号（**交给别人**用：共享内存那一档要把"我们看的是同一段"说出去）。
    pub fn pie_token(&self) -> PieToken {
        self.pie
    }

    /// 撤图：撤掉这次映射（幂等）。**不 `release` 门闩**——它与 `self` 一起放下。
    ///
    /// # Errors
    /// - `Denied` — 门闩已不在表里 / 权限不含 `FETCH`
    /// - `HandedOver`  — 这一枚已经交出去了（同 [`Dock::open`]）
    ///
    /// **`Dead` 不在其列，这是刻意的**：撤图撤的是**调用方自己那张 PTE**，故 `shut`
    /// 不过存活闸——资源封印之后，已经借进来的那段映射仍然撤得掉。与 `Release`
    /// 「你总得能放下手里的东西」同一条口径。
    pub fn shut(self) -> Result<(), K::Error> {
        K::shut(self.pie)
    }
}
