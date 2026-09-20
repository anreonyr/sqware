//! boot — **boot 给引导域的两块账**：清单（装了哪些程序）与配对块（有哪些门闩）。
//!
//! 这是**这台机器的事实**，不是协议：它读的是启动参数（`env::wire::args`），行的还是
//! "谁被装进来了"这件事。固件面（单子与回单）住在 [`protocol::firmware`]；把需求单翻成
//! 单子上的一条住在 [`crate::supervisor::pairing`]。

use env::wire::{args as boot_args, manifest};
use env::{PAIR_LEN, Pair, PieToken};

/// boot 给引导域的两块账：清单（装了哪些程序）与配对块（有哪些门闩）。
pub struct Root {
    view: &'static [u8],
    pairs: &'static [u8],
}

impl Root {
    /// 从启动参数取出两块账。`None` = 参数不足 / 清单头非法（不该发生）。
    pub fn take() -> Option<Root> {
        let a = runtime::env::unit::args();
        if a.len() < boot_args::LEN {
            return None;
        }
        let (view, len) = (a[boot_args::VIEW] as *const u8, a[boot_args::VIEW_LEN]);
        // `COUNT` 是**条数**，不是字节数（布局见 `env::wire::args`）。
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

    /// 配对块里按名字取一枚门闩（名字是 boot 给的原样，见 `needs`）。
    pub fn token(&self, want: &str) -> Option<PieToken> {
        for i in 0..self.pairs.len() / PAIR_LEN {
            // SAFETY: 块是 boot 只读借映的一段，逐条定长；本域只读。
            // 用 `read_unaligned` 是因为记录步长 40 字节而块只保证页对齐。
            let at = unsafe { self.pairs.as_ptr().add(i * PAIR_LEN) };
            let rec = unsafe { core::ptr::read_unaligned(at.cast::<Pair>()) };
            if rec.name().is_some_and(|n| n.as_str() == want) {
                return Some(rec.token());
            }
        }
        None
    }
}
