//! dock — **Pole 的 runtime 封装**：把一枚共享页借映进本域，得到一段视图。
//!
//! 内核里它**仍是一枚 Pole**（`AnyPie::Pole`，没有新资源）：页块与 `Open`/`Shut`/`Narrow`
//! 都在内核，"怎么用"封装在这一层。与 `Port` 包着 `HolePie` 同构：**种类归内核，用法归
//! runtime**。
//!
//! 三件各包一种 primitive，各加一样东西：
//!
//! ```text
//! Port   两枚 Hole   加**配对**   往哪推、从哪收、对面是谁
//! Dock   一枚 Pole   加**成对**   起点与长度成对的一段内存
//! Bell   一枚 Nole   加**约束**   三拍 ring / wait / hush（只有一条方向）
//! ```
//!
//! # 为什么视图是**另一个类型**
//!
//! 映射与视图的可复制性相反：映射不该被复制（`shut` 要消费它），而视图**必须**能被复制
//! ——`static UART` 配 `Uart: Copy`，两个线程各取一份（同一张页表，不需要锁）。故两个类型：
//! [`Dock`] 不 `Copy`，[`View`] 是 `Copy` 的值。
//!
//! # 两条动词，与 `PolePie` 同形
//!
//! `open` / `shut` 就是 `PieCall::Open` / `Shut`，**本层不加新动词**——多出来的只有一件事：
//! 把"起点 + 长度"成对地带回来。于是"一段不知道多长的内存"在类型上不存在。
//!
//! # `View::size` 报的是**映射的那一段**
//!
//! 页是映射粒度、设备树 `reg` 是所有权粒度：外来区按页界向两侧
//! 撑开，故 UART 的 `reg` 只有 0x100 而 `size()` 是 4096。本层报后者——前者内核不知道。
//!
//! # 门闩收下，但不代管它的收尾
//!
//! `shut` 只撤图，**不 `release` 门闩**——与 `Port::shut` 同一条理由：门闩的收尾是策略，
//! 不进结构。

use env::EnvResult;

use crate::env::mail::PolePie;

/// 视图：一段**已映射进本域**的内存。
///
/// 起点与长度成对才是一段区间——单独一个数拿出去没有意义，故两半私有、只能一起取走。
/// `Copy`：视图本来就是可复制的（两个线程各要一份正是既有用法）。
///
/// **它不承诺"用完即失效"**：交给谁就是把地址给出去了，而地址是裸数——可以被复制、存进
/// static、加偏移。撤图靠 [`Dock::shut`]，那是礼节，不是证明。
#[derive(Clone, Copy, Debug)]
pub struct View {
    base: usize,
    size: usize,
}

impl View {
    /// 视图起点。
    pub fn base(self) -> usize {
        self.base
    }

    /// 视图长度 ＝ **映射的那一段**（页对齐），不是设备声明的 `reg` 长度。
    pub fn size(self) -> usize {
        self.size
    }
}

/// 泊位：一枚 Pole 门闩 + 它在本域的那段视图。
pub struct Dock {
    pie: PolePie,
    view: View,
}

impl Dock {
    /// 映射：把一枚已授权的共享页借映进本域 → 视图。
    ///
    /// 收下 `pie`（本层持门闩，与 `Port` / `Bell` 同构）。同一枚门闩再开一次是幂等的
    /// （内核返同一个 VA），但那要求另造一个句柄——本层不为此开入口。
    ///
    /// # Errors
    /// - `Denied` — 门闩不在本任务表里 / 权限不含 `FETCH`
    /// - `Dead`   — 资源已封印
    /// - `Caged`  — 这一枚被我交出去了（`ONLY` 资源的锚还在、接收方手里那枚还活着）
    /// - `OoM`    — 本域空间备不出这么长的一段
    pub fn open(pie: PolePie) -> EnvResult<Dock> {
        let (base, size) = pie.open()?;
        Ok(Dock {
            pie,
            view: View { base, size },
        })
    }

    /// 把视图拷一份交出去——驱动要的就是它（`View` 是 `Copy`，故借 `&self`）。
    pub fn view(&self) -> View {
        self.view
    }

    /// 撤图：撤掉这次映射（幂等）。**不 `release` 门闩**——它与 `self` 一起放下。
    ///
    /// # Errors
    /// - `Denied` — 门闩已不在表里 / 权限不含 `FETCH`
    /// - `Caged`  — 这一枚已经交出去了（同 [`Dock::open`]）
    ///
    /// **`Dead` 不在其列，这是刻意的**：撤图撤的是**调用方自己那张 PTE**，故 `shut`
    /// 不过存活闸（`envcall/pie.rs` 的 `shut` 只 `locate` + 判权 + 判锚）——资源封印之后，
    /// 已经借进来的那段映射仍然撤得掉。与 `Release`「你总得能放下手里的东西」同一条口径。
    pub fn shut(self) -> EnvResult<()> {
        self.pie.shut()
    }
}
