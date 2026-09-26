//! mold —— 三个过程宏：`#[derive(Frame)]`（**定长帧**）、`#[derive(Envcall)]`（**环境调用
//! 枚举**）、`#[entry]`（**入口那一手**）。
//!
//! 三者互不相干，**各占一个文件**（[`frame`] / [`envcall`] / [`entry`]）——本文件只有 crate
//! 头注与三个入口，每个入口一句"吃什么、吐什么"，正文在各自那个文件里；**三者之间一行都不
//! 共享**。
//!
//! **形态不同，各由"它要产出什么"定死**：两个 derive 产出的都是"**附属在你手写的那枚 item
//! 上的**东西"（`Frame` → 那枚结构体的 `LEN` 与三手；`Envcall` → 那枚枚举的 `impl` 与一枚
//! 新枚举）；`#[entry]` 则要**改写**自己挂着的那一项（原函数留着、另加一个符号），故只能是
//! attribute。

mod entry;
mod envcall;
mod frame;

use proc_macro::TokenStream;

/// **定长帧**：给一枚具名字段的结构体生成 `LEN` ＋ `store` / `store_in` / `fetch`
/// （**结构体归你写**——它本就是那张字段表）。
///
/// ```ignore
/// #[derive(env::Frame)]
/// #[derive(Clone, Copy, PartialEq, Eq, Debug)]
/// pub struct Tip {
///     pub who: TaskId,
///     pub name: Name,
///     pub reply: PieToken,
/// }
/// ```
///
/// 详见 [`frame`]。
#[proc_macro_derive(Frame)]
pub fn derive_frame(input: TokenStream) -> TokenStream {
    frame::expand(input.into()).into()
}

/// **环境调用枚举**：给一枚带载荷的调用枚举生成 `slot` / `pack` / `from_wire` ＋ `Ret` 枚举
/// ＋ `call`（三个属性 `#[call(class = N)]` / `#[ret(T)]` / `#[ret3(T)]`）。
///
/// 详见 [`envcall`]。
#[proc_macro_derive(Envcall, attributes(call, ret, ret3))]
pub fn derive_envcall(input: TokenStream) -> TokenStream {
    envcall::expand(input.into()).into()
}

/// **入口那一手**：把你写的 `main` 接到汇编要调的符号上（原函数留着，另加
/// `extern "C" fn clean_ret`）。
///
/// 详见 [`entry`]。
#[proc_macro_attribute]
pub fn entry(_attr: TokenStream, item: TokenStream) -> TokenStream {
    entry::expand(item.into()).into()
}
