//! boot — **boot 给引导域的两块账**：清单（装了哪些程序）与配对块（有哪些门闩）。
//!
//! 这是**这台机器的事实**，不是协议：它读的是启动参数（`plan::args`），行的还是
//! "谁被装进来了"这件事。物料面（单子与回单）住在 [`protocol::driver::supply`]；要哪几样由
//! **收方**自己开单（三张都在 [`plan::assembly`]，开口的形态就是 `Need`）。

use alloc::format;

use plan::key::{DTB, IRQ, REGION};
use plan::{args as boot_args, manifest};
use env::{PieToken};
use plan::{Key, PAIR_LEN, Pair};

/// boot 给引导域的两块账：清单（装了哪些程序）与配对块（有哪些门闩）。
pub struct Root {
    view: &'static [u8],
    pairs: &'static [u8],
}

impl Root {
    /// 从启动参数取出两块账。`None` = 参数不足 / 清单头非法（不该发生）。
    pub fn take() -> Option<Root> {
        let a = runtime::core::unit::args();
        if a.len() < boot_args::LEN {
            return None;
        }
        let (view, len) = (a[boot_args::VIEW] as *const u8, a[boot_args::VIEW_LEN]);
        // `COUNT` 是**条数**，不是字节数（布局见 `plan::args`）。
        let (pairs, count) = (a[boot_args::PAIRS] as *const u8, a[boot_args::COUNT]);
        // SAFETY: boot 把这两区只读映射进本域，长度即启动参数给的字节数；本域只读。
        let view = unsafe { core::slice::from_raw_parts(view, len) };
        let pairs = unsafe { core::slice::from_raw_parts(pairs, count * PAIR_LEN) };
        // 清单头先验一遍：非法即装配不成立。
        manifest::Entries::new(view)?;
        Some(Root { view, pairs })
    }

    /// 清单：这台机器装了哪些程序（引导域按名字挑）。
    ///
    /// 每次给一个**新的游标**（`Entries` 是一次性读的），故调用方可以按需重读。
    pub fn programs(&self) -> manifest::Entries<'static> {
        manifest::Entries::new(self.view).expect("清单头已在 take 时验过")
    }

    /// 清单那块字节的读面（`Catalog` 的两种来源之一）。
    pub fn view(&self) -> &'static [u8] {
        self.view
    }

    /// 配对块里按**坐标**取一枚门闩。
    ///
    /// **坐标唯一**：区不重叠（设备 `reg` 段与载荷区各有各的基址），`dtb` / `irq` 各自只有
    /// 一件——故不再有"同名取第一枚"这回事。判别号读不懂的记录当场跳过。
    pub fn token(&self, key: Key) -> Option<PieToken> {
        for i in 0..self.pairs.len() / PAIR_LEN {
            if self.key(i) == Some(key) {
                return Some(self.record(i).token());
            }
        }
        None
    }

    /// 配对块的自述（一行）：按判别号数它有什么。
    ///
    /// **照实记**：从前这里印的是"重名"（同一节点的多段 `reg` 造出两条同名记录，而按名取只
    /// 够得到第一枚）。坐标换成区之后那笔账不存在了——两段各有各的基址，各是各的坐标。
    /// 这条读数因此改报**块里有什么**：`region` 是区段的条数（设备 + 载荷区），
    /// `dtb` / `irq` 各一件，`bad` 是读不懂的条数。
    pub fn report_pairs(&self) {
        let n = self.pairs.len() / PAIR_LEN;
        let (mut region, mut dtb, mut irq, mut bad) = (0, 0, 0, 0);
        for i in 0..n {
            match self.key(i).map(|key| key.parts().0) {
                Some(REGION) => region += 1,
                Some(DTB) => dtb += 1,
                Some(IRQ) => irq += 1,
                _ => bad += 1,
            }
        }
        let _ = runtime::env::debug::put(&format!(
            "root: block n={n} region={region} dtb={dtb} irq={irq} bad={bad}"
        ));
    }

    /// 第 `i` 条的坐标（定长记录，块只保证页对齐 ⇒ `read_unaligned`）。
    fn key(&self, i: usize) -> Option<Key> {
        self.record(i).key()
    }

    /// 第 `i` 条记录。
    fn record(&self, i: usize) -> Pair {
        // SAFETY: 块是 boot 只读借映的一段，逐条定长（`i` 由调用方按条数界内给出）；
        // 步长 24 字节而块只保证页对齐，故 `read_unaligned`。
        let at = unsafe { self.pairs.as_ptr().add(i * PAIR_LEN) };
        unsafe { core::ptr::read_unaligned(at.cast::<Pair>()) }
    }
}
