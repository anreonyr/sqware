//! `#[entry]` —— **入口那一手**的过程宏：把 bin 里那个 `main` 与"汇编要调的那个符号"接上。
//!
//! ```ignore
//! #[entry]
//! fn main() -> Result<(), fail::Fail> { … }
//! ```
//!
//! 展开成两件东西（**你写的那个函数一个字没动**）：
//!
//! ```ignore
//! fn main() -> Result<(), fail::Fail> { … }        // 原样
//!
//! #[unsafe(no_mangle)]
//! extern "C" fn clean_ret() { programs::entry::entry(main) }
//! ```
//!
//! `_start` 的汇编调的就是 `clean_ret`（见 `programs/src/entry.rs`）——**符号名不再是 `main`**，
//! 于是：
//!
//! - 你那个 `main` 照旧叫 `main`（不必改名、不必加 attribute）；
//! - 也不必把它藏进一个 `mod` 里躲名字冲突（上一版那层 `mod __entry` 因此撤掉）；
//! - 生成的这一层是**宏展开**，不是 `OUT_DIR` 里的一个文件 ⇒ bin 的源码里不再有
//!   `include!(concat!(env!("OUT_DIR"), …))` 那行路径咒语。
//!
//! 名字为什么叫 `clean_ret`：入口那一手做的是"**干净地回来**"——`main` 正常返回时，
//! 把退出码与那句话折成 `Report` 交给内核；`main` 自己退场（`-> !`）时这一层根本走不到。

use proc_macro::TokenStream;
use quote::quote;
use syn::{ItemFn, parse_macro_input};

/// 见 crate 头注。
#[proc_macro_attribute]
pub fn entry(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let func = parse_macro_input!(item as ItemFn);
    let name = &func.sig.ident;
    if !func.sig.inputs.is_empty() {
        return syn::Error::new_spanned(
            &func.sig.inputs,
            "#[entry] 的 main 不收参数（参数从 `runtime::env::unit::args()` 取）",
        )
        .to_compile_error()
        .into();
    }
    quote! {
        #func

        /// 汇编 `_start` 调的那一格（名字见宏的头注；这里不让它叫 `main`——那个名字归你）。
        #[unsafe(no_mangle)]
        extern "C" fn clean_ret() {
            programs::entry::entry(#name)
        }
    }
    .into()
}
