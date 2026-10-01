//! schedule — **装配的相**：一台服务从"立了账、造了身子"到"在树上答得动"，分几相走、每相谁动手。
//!
//! ```text
//!   立账 → 造身子 → 装通道        ← 不是手：这三件对整张单逐条做（`control/{assemble,mod}.rs`）
//!   ── BEFORE_LAUNCH ──          放行之前：它一步都还没跑
//!       放行（`Control::launch`：起步 ＋ 递整机物料）—— 夹在相与相之间的**生命那一步**
//!   ── AFTER_RELEASE ──          放行之后、它答得动之前：接那两条装配路，再等凭据齐
//!   ── AFTER_READY ──            它答得动之后：认下"答案从哪来"那一类事实
//! ```
//!
//! # 为什么要有这一份（照实记）
//!
//! 从前这三相拧在 [`Assembly::assemble`](crate::system::Assembly::assemble) 一段正文里：88 行
//! 里 56 行说的是**别的轴**的字段（板 15 / 树 36 / 名册 5）——"生命"与"存在信号"在同一段里
//! 交错，于是**没有一处改动看得出牵动谁**：板那一格是不是还有人读、树那几格谁在读，都得把
//! 那一段通读一遍才答得出。
//!
//! 今天那两个问题各有一处答案：
//!
//! - **次序**：相是一张声明出来的表（本文件就是那张表），正文按相走，不再逐行手排位次；
//! - **读者**：一手**住在它自己那一轴的文件里**（[`Act`]），**那一格由它自己读**——板那一格
//!   在 `board/bridge.rs` 里读、树那几格在 `operator/bridge.rs` 里读、身份那一格在
//!   `principal/bridge.rs` 里读。
//!
//! 故"撤掉一条轴"是一次**局部**改动：那一格 ＋ 那一手 ＋ 表里那一行，三处同进同出。
//!
//! **它为什么不叫 `Hand`**（照实记）：`board` / `operator` 那两个 bridge 里**已经有一个 `hand`**
//! ——它说的是"把一枚孔递出去"（`hand(reply, host)`：转授那一手），与本处"装配的一手"是两件事
//! ⇒ 两个词都留着，本处那一型叫 [`Act`]。
//!
//! **手不按 id 挂钩**（照实记：Bevy 那种 `add_systems(Phase, sys)` 注册表在这不需要）：一张单是
//! **编译期定死**的（`PROGRAMS` 是 `const`），故手直接写进表里——**表就是注册表**，不必再来一枚
//! 运行期才查得到的名字。

use crate::program::{Died, Program};
use crate::system::Assembly;
use crate::system::control::assemble as control;
use crate::system::control::{Error, Service};
use crate::system::operator::bridge as operator;
use crate::service::principal::bridge as principal;

/// **一手**：某一轴在某一相里对某一台做的一件事。
///
/// 签名里只有"这一台"（[`Program`]）与"它的身子"（[`Service`]，起手那两样）：**读哪一格由这一手
/// 自己定** ⇒ 那一格的读者与写它的那份声明住得开（见本文件头注）。
///
/// **它返的是一句话，不是一个号**：装配失败的号是按**这一台**分的（`Program::demand.died`），
/// 由 [`advance`] 折出来；手只报"死在装配哪一步"（与 `service::step` 同款）。
pub type Act = fn(&mut Assembly, &Program, &mut Service) -> Result<(), &'static str>;

/// **一相**：这一相里那几只手，**次序即契约**。
pub type Phase = &'static [Act];

/// **放行之前**：此刻这一台一步都还没跑——凡"它一起来就该知道"的都要在放行前落定（今天只有
/// 身份那一件；晚一步落地，它第一次 `resolve(self)` 就问空了）。
pub const BEFORE_LAUNCH: Phase = &[principal::bind];

/// **放行之后、它答得动之前**：接那两条装配路（存在信号 / 树），再等它把凭据交齐。
///
/// **次序是契约**（照实记：这一格换过位置）：等就绪要等的那一条凭据，要到**挂上树、拿到物料、
/// 把每一台落完格**之后才铸得出来 ⇒ 它必须排在板 / 树那两手之后（理由与实测写在
/// [`await_ready`](crate::system::control::assemble::await_ready)）。
pub const AFTER_RELEASE: Phase = &[operator::attach_client, control::await_ready];

/// **它答得动之后**：认下"答案从哪来"那一类事实——谁持树（那一枚提示之路）、谁是名册
/// （它交上来的那一枚定面门牌）。**两件都由运行期的那一枚孔认**，不读声明。
pub const AFTER_READY: Phase = &[operator::hold, principal::adopt_roster];

/// **走一相**：逐手；哪一手不成，折成**这一台自己的号**（读数 = 程序名 ＋ 那一手自己的步名）。
pub fn advance(
    assembly: &mut Assembly,
    phase: Phase,
    program: &Program,
    service: &mut Service,
) -> Result<(), Died> {
    for &act in phase {
        act(assembly, program, service).map_err(|why| super::fail(program, Error::Step(why)))?;
    }
    Ok(())
}
