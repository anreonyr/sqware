//! mold —— 五个过程宏：`#[derive(Frame)]`（**帧**）、`#[derive(Envcall)]`（**环境调用
//! 枚举**）、`#[derive(Fail)]`（**域失败词汇**：域内自 `-1` 起）、`#[derive(WireCodes)]`
//! （**线上那一格**：失败域 ↔ 答话那一格）、`#[entry]`（**入口那一手**）。
//!
//! 五者互不相干，**各占一个文件**（[`frame`] / [`envcall`] / [`fail`] / [`codes`] / [`entry`]）
//! ——本文件只有 crate 头注与五个入口，每个入口一句"吃什么、吐什么"，正文在各自那个文件里；
//! **五者之间一行都不共享**。
//!
//! **形态不同，各由"它要产出什么"定死**：四个 derive 产出的都是"**附属在你手写的那枚 item
//! 上的**东西"（`Frame` → 那枚结构体的 `LEN` 与一条 `impl Span`；`Envcall` → 那枚枚举的 `impl` 与一枚
//! 新枚举、每格一个入口；`Fail` → 那枚词表的域内码与读法；`WireCodes` → 那枚词表的线上码、
//! 两向读法与编译期断言）；`#[entry]` 则要**改写**自己挂着的那一项（原函数留着、
//! 另加一个符号），故只能是 attribute。

mod codes;
mod entry;
mod envcall;
mod fail;
mod frame;

use proc_macro::TokenStream;

/// **帧**：给一枚具名字段的结构体生成 `pub const LEN` ＋ `impl env::wire::Span`（**结构体归你
/// 写**——它本就是那张字段表，生成物使它自己就是过线的一格）；一格上一段重复写
/// `#[frame(count = <条数那一格>, fill = <空位那一枚>)]`。带参数的结构体也吃（泛型帧也是帧）。
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
#[proc_macro_derive(Frame, attributes(frame))]
pub fn derive_frame(input: TokenStream) -> TokenStream {
    frame::expand(input.into()).into()
}

/// **环境调用枚举**：给一枚带载荷的调用枚举生成 `slot` / `pack` / `from_wire` ＋ `Ret` 枚举
/// ＋ `call` ＋ **每格一个精确签名的入口**（属性：类级 `#[call(class = N, fail = XxxFail)]`，
/// 变体级 `#[ret(T)]` / `#[ret3(T)]` / `#[infallible]` / `#[manual]`）。
///
/// 详见 [`envcall`]。
#[proc_macro_derive(Envcall, attributes(call, ret, ret3, infallible, manual))]
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

/// **线上那一格**：给一枚无字段枚举生成 `pub const <名>: u8`（每格一枚）＋ `fail_to_code` /
/// `code_to_fail`（属性：枚举级 `#[wire(also(名 = 码, …), fallback = 变体)]`，变体级
/// `#[code(码)]` / `#[code(码, 名)]`）。
///
/// 详见 [`codes`]。
#[proc_macro_derive(WireCodes, attributes(wire, code))]
pub fn derive_wire_codes(input: TokenStream) -> TokenStream {
    codes::expand(input.into()).into()
}

/// **入口那一手**：把你写的 `main` 接到汇编要调的符号上（原函数留着，另加
/// `extern "C" fn clean_ret`）。
///
/// 详见 [`entry`]。
#[proc_macro_attribute]
pub fn entry(_attr: TokenStream, item: TokenStream) -> TokenStream {
    entry::expand(item.into()).into()
}
