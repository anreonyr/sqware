//! pairing — **boot 给的账**与**装配者送到子域手里的账**，编解码只此一份。
//!
//! 两条路共用同一套形状（`Pair` = 定长 40 字节：名字块 + 句柄）：
//!
//! ```text
//!   ① boot → 装配者   配对块（boot 扫设备树产出，只读借映）
//!   ② 装配者 → 子域   记录数组（装配者按 needs 发货，推进会话那条通道）
//! ```
//!
//! **契约住在这里**：一端 [`Root::pack`]（发货）、一端 [`unpack`]（收货）。以前这两半
//! 分别长在两个 `main.rs` 里，靠"两边都读同一张需求单"对齐——改一处忘一处就静默错位。
//! 现在它们在同一份代码里成对，**条数从需求单自己算**（发货方不必再抄一遍"要几样"），
//! 且 [`Root`] 把"启动参数 → 清单 → 配对块"这段 boot 记账也收在一处。
//!
//! **两个 bin 各用一半**（发货方用 `Root` / `pack`，收货方用 `unpack`），故本模块对
//! "另一半没被用到"不报警告——那正是成对放在一起的目的。

#![allow(dead_code)]

use alloc::vec::Vec;

use env::wire::{args as boot_args, manifest};
use env::{Name, PAIR_LEN, Pair, PieToken};

use super::needs::Need;

/// boot 给装配者的两块账：清单（装了哪些程序）与配对块（有哪些门闩）。
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

    /// 清单：这台机器装了哪些程序（装配者按名字挑）。
    ///
    /// 每次给一个**新的游标**（`Entries` 是一次性读的），故调用方可以按需重读。
    pub fn programs(&self) -> manifest::Entries<'static> {
        manifest::Entries::new(self.view).expect("清单头已在 take 时验过")
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

    /// 发货：按需求单把门闩一枚一枚交给收方，产出要推进通道的那段字节。
    ///
    /// `ship` 把那一枚交出去（它看得到这条需求的权与形态），返回**种在收方表里**的
    /// 句柄（收方正是靠那个号说话）。`None` = 某一枚不在配对块里（装配错了）。
    ///
    /// **条数取自需求单**：调用方不抄"要几样"，也就不可能与单子走偏。
    pub fn pack(&self, ship: impl Fn(PieToken, &Need) -> Option<PieToken>) -> Option<Vec<u8>> {
        let mut out: Vec<u8> = Vec::new();
        out.try_reserve(super::needs::PLIC.len() * PAIR_LEN).ok()?;
        for need in super::needs::PLIC.iter() {
            let src = self.token(need.name)?;
            let at = ship(src, need)?;
            let pair = Pair::new(Name::new(need.name).ok()?, at);
            out.extend_from_slice(pair_bytes(&pair));
        }
        Some(out)
    }
}

/// 收货：把那段字节按需求单解回来，一枚一枚按 **`Slot`** 交给 `f`。
///
/// 名字对不上单子的条目**直接跳过**（不是错：单子只增不减时会有旧记录）。
pub fn unpack(bytes: &[u8], mut f: impl FnMut(usize, PieToken)) {
    for i in 0..bytes.len() / PAIR_LEN {
        // SAFETY: 记录与块同源（`Pair` 的尺寸由编译期断言锁死为 `PAIR_LEN`）；缓冲只
        // 保证 1 字节对齐，故 `read_unaligned`。越界由上面的除法挡掉。
        let at = unsafe { bytes.as_ptr().add(i * PAIR_LEN) };
        let rec = unsafe { core::ptr::read_unaligned(at.cast::<Pair>()) };
        let Some(name) = rec.name() else { continue };
        for need in super::needs::PLIC.iter() {
            if need.name == name.as_str() {
                f(need.slot as usize, rec.token());
            }
        }
    }
}

/// 一条记录的字节：**名字块 + 句柄**——尺寸由 `Pair` 自己锁死，这里只是一次只读的
/// 字节视图（`Pair` 是 `repr(C)`，内容即线格式）。
fn pair_bytes(pair: &Pair) -> &[u8; PAIR_LEN] {
    // SAFETY: `Pair` 是 `repr(C)`、尺寸由编译期断言等于 `PAIR_LEN`，只读解释为字节安全。
    unsafe { &*(pair as *const Pair).cast::<[u8; PAIR_LEN]>() }
}
