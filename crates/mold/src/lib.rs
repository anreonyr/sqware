//! mold —— 四个过程宏：`#[derive(Frame)]`（**定长帧**）、`#[derive(Envcall)]`（**环境调用
//! 枚举**）、`#[derive(Fail)]`（**域失败词汇**）、`#[entry]`（**入口那一手**）。
//!
//! 四者互不相干，**各占一个文件**（[`frame`] / [`envcall`] / [`fail`] / [`entry`]）——本文件
//! 只有 crate 头注与四个入口，每个入口一句"吃什么、吐什么"，正文在各自那个文件里；
//! **四者之间一行都不共享**。
//!
//! **形态不同，各由"它要产出什么"定死**：三个 derive 产出的都是"**附属在你手写的那枚 item
//! 上的**东西"（`Frame` → 那枚结构体的 `LEN` 与三手；`Envcall` → 那枚枚举的 `impl` 与一枚
//! 新枚举、每格一个入口；`Fail` → 那枚词表的码与读法）；`#[entry]` 则要**改写**自己挂着的
//! 那一项（原函数留着、另加一个符号），故只能是 attribute。

mod entry;
mod envcall;
mod fail;
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
/// ＋ `call` ＋ **每格一个精确签名的入口**（属性：类级 `#[call(class = N, fail = XxxFail)]`，
/// 变体级 `#[ret(T)]` / `#[ret3(T)]` / `#[infallible]` / `#[manual]`）。
///
/// 详见 [`envcall`]。
#[proc_macro_derive(
    Envcall,
    attributes(call, ret, ret3, infallible, manual)
)]
pub fn derive_envcall(input: TokenStream) -> TokenStream {
    envcall::expand(input.into()).into()
}

/// **域失败词汇**：给一枚无字段枚举生成 `code` / `of_code` ＋ `FailCode`/`Display` 那一套
/// （唯一属性 `#[busy]`，标在「条件未就绪」那一枚上）。
///
/// 详见 [`fail`]。
#[proc_macro_derive(Fail, attributes(busy))]
pub fn derive_fail(input: TokenStream) -> TokenStream {
    fail::expand(input.into()).into()
}

/// **入口那一手**：把你写的 `main` 接到汇编要调的符号上（原函数留着，另加
/// `extern "C" fn clean_ret`）。
///
/// 详见 [`entry`]。
#[proc_macro_attribute]
pub fn entry(_attr: TokenStream, item: TokenStream) -> TokenStream {
    entry::expand(item.into()).into()
}
