//! 编码、通信接口、环境调用与程序入口的过程宏。

mod codes;
mod contract;
mod entry;
mod envcall;
mod fail;
mod frame;
mod interface;

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
#[proc_macro_derive(Envcall, attributes(call, slot, ret, ret3, infallible, manual))]
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

/// Generate request frames, replies, grants and marks from an inline interface module.
#[proc_macro_attribute]
pub fn interface(attr: TokenStream, item: TokenStream) -> TokenStream {
    interface::expand(attr.into(), item.into()).into()
}

/// Declare a typed wire request/response pair and, when present, its explicit reply path.
#[proc_macro_attribute]
pub fn contract(attr: TokenStream, item: TokenStream) -> TokenStream {
    contract::expand(attr.into(), item.into()).into()
}
