//! `fail_codes!` —— **码表宏自己一份源**（协议与宿主靶同读这一份）。
//!
//! **照实记（它为什么住一个文件）**：这个宏原先写在 `lib.rs` 里（`#[macro_export]`）。帧那一半
//! 上宿主靶之后，靶里那些**逐字未改**的 `call.rs` 里有一次调用，而宿主靶**不依赖 `protocol`**
//! （它拖 `runtime`，宿主编译器编不出那两处 riscv 内联汇编）⇒ 宏得有个**两边都读得到**的家。
//! 挪出来之后**调用点一行都不用改**，靠的是**两样都要**：`#[macro_use]`（把宏带进本 crate
//! 后面那些模块的作用域）与 `#[macro_export]`（保住"出 crate"那一份）。宿主靶那边同样是
//! 两行：`#[macro_use] #[path] mod fail_codes;` 写在**帧模块之前**。
//!
//! **照实记（先只留 `#[macro_export]` 的那一版编不过）**：宏的可见性按**正文先后**算，而
//! `#[macro_export]` 只把名字放进 crate 根的名字空间、不会自动带进各子模块 ⇒ 编出来 11 处
//! `cannot find macro fail_codes`（五份 `call.rs` 各一处）。
//!
//! 结构那一格：本宏独立成一份源——协议与各宿主靶**同读这一份**（上面那两行就是它的用法）。
//!
//! ---
//!
/// **答话那一格里的"一个失败都不是"**：六家（principal / coalition / operator / board / line /
/// supply）与驱动各自那几族（如 `programs::driver::rtc`）**共用这一个号**。各家的**失败码**按自己
/// 的失败域排（同一个概念在两家是别的号——见各族 `frame.rs` 的注），而"没失败"只有一个号。
/// `fail_codes!` 的第二个参数就是它。
///
/// **照实记（这一格原先是七份常量）**：七份 `pub const OK: u8 = 0;` 各写一遍，值全靠自律一致；
/// 收在这里最省——**本文件每个宿主靶本来就编**（见上面那两行），故一处都不多要。crate 内各家
/// `pub use crate::fail_codes::OK;`；**出 crate 那一份**（`programs::driver::rtc` —— 驱动自己的
/// 协议住在 `programs` 里）走 `protocol::OK` 那条转出（那是本 crate 的**上游**）。
pub const OK: u8 = 0;

/// 码表：**失败域 ↔ 线上答话那一格**，四家同一个形状。
///
/// - 正向一律 `fail_to_code(Option<Fail>) -> u8`：`None`（没失败）⇒ `OK`；
/// - 反向一律 `code_to_fail(u8) -> Option<Fail>`：`OK` ⇒ `None`。
///
/// **反向只在双射时生成**。非双射的表（几种失败归同一个码）**不给反向**——由人写并注明
/// "反不回来"。这是判据，不是风格：给非双射的表生成反向，等于把"对偶"说成假的。
///
/// **出 crate**（`#[macro_export]`）：第二个实例到了——驱动的**具体协议**住各驱动自己的目录
/// （那一条裁定见 [`driver`]），而它同样要一张"失败域 ↔ 线上那一格"的表。手抄一遍就是两处编。
#[macro_export]
macro_rules! fail_codes {
    ($(#[$meta:meta])* bijective $fail:ty; $ok:ident; $($variant:path => $code:ident),+ $(,)?) => {
        $(#[$meta])*
        pub const fn fail_to_code(fail: Option<$fail>) -> u8 {
            match fail {
                None => $ok,
                $(Some($variant) => $code),+
            }
        }

        /// 线上答话那一格 → 失败域。`OK`（没失败）那一格一定答 `None`——读的人靠动作码先分流。
        ///
        /// **`BAD`（这一问读不懂）在不在表里，由各家自己的表说**：板那一侧它独立一格、留在
        /// 表外（`system::board::call`），`driver::rtc` 那一侧它与"没走到"（`Fail::Denied`）
        /// 合流——那边的持有者从来不说"我没接住"这句话（接不住就是没有孔可回）。
        ///
        /// 表外那一格与读不懂的码一律答 `None`：两个 `None` 不是同一件事，读的人靠动作码先分流。
        pub const fn code_to_fail(code: u8) -> Option<$fail> {
            match code {
                $ok => None,
                $($code => Some($variant)),+,
                _ => None,
            }
        }
    };
    ($(#[$meta:meta])* lossy $fail:ty; $ok:ident; $($arm:pat => $code:ident),+ $(,)?) => {
        $(#[$meta])*
        pub const fn fail_to_code(fail: Option<$fail>) -> u8 {
            match fail {
                None => $ok,
                $(Some($arm) => $code),+
            }
        }
    };
}
