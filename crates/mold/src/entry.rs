//! `#[entry]` —— **入口那一手**：把 bin 里那个 `main` 与"汇编要调的那个符号"接上。
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
//!
//! **照实记（它绑下游）**：展开体里写死 `programs::entry::entry(#name)`——这一枚宏**不是**
//! 通用的，它认的就是 `programs` 那一格入口胶水。与 `#[derive(Envcall)]` 的主张不同：那个只
//! 依赖 `Wire`，这个依赖一个具体的下游 crate。
//!
//! 本文件是该宏的全部——**与另外两个宏一行都不共享**。

use proc_macro2::TokenStream;
use quote::quote;
use syn::{ItemFn, parse2};

/// 展开见文件头。
pub fn expand(item: TokenStream) -> TokenStream {
    let func: ItemFn = match parse2(item) {
        Ok(func) => func,
        Err(e) => return e.to_compile_error(),
    };
    let name = &func.sig.ident;
    if !func.sig.inputs.is_empty() {
        return syn::Error::new_spanned(
            &func.sig.inputs,
            "#[entry] 的 main 不收参数（参数从 `runtime::core::unit::args()` 取）",
        )
        .to_compile_error();
    }
    quote! {
        #func

        /// 汇编 `_start` 调的那一格（名字见宏的头注；这里不让它叫 `main`——那个名字归你）。
        #[unsafe(no_mangle)]
        extern "C" fn clean_ret() {
            programs::entry::entry(#name)
        }
    }
}
