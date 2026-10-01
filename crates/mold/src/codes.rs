//! `#[derive(WireCodes)]` —— **线上那一格**：失败域 ↔ 答话那一格（`u8`）的码表。
//!
//! 输入一枚无字段枚举，每枚变体上写它落在线上哪一格：
//!
//! ```ignore
//! #[derive(Clone, Copy, PartialEq, Eq, Debug, WireCodes)]
//! #[wire(also(BAD = 4))]              // 只有码、没有变体的格（可选）
//! pub enum Fail {
//!     /// 本控制器上没有这条线 ⇒ 回头查树。
//!     #[code(1)] Unknown,             // 名默认 = 变体名大写 ⇒ 出 `pub const UNKNOWN: u8 = 1;`
//!     #[code(2)] Taken,
//!     #[code(3, DENIED)] Denied,      // 码名与默认不同时才写第二个参数
//! }
//! ```
//!
//! 产出（**这条链上码只写一处：变体那一行**）：
//!   * 每一格一枚 `pub const <名>: u8 = <码>;`（变体那句 doc 原样抄过去）
//!   * `pub const fn fail_to_code(fail: Option<Fail>) -> u8`：`None`（没失败）⇒ 作用域里的 `OK`
//!   * `pub const fn code_to_fail(code: u8) -> Option<Fail>`：`OK` ⇒ `None`；表外 ⇒ `None`
//!     （枚举上写了 `#[wire(fallback = 某变体)]` 时折成它）
//!   * 编译期断言：码唯一、不与 `OK` 撞、逐枚读得回来、`also` 那几格反向落表外（或兜底）
//!
//! **`OK` 由调用处提供**（各族那一行 `pub use …::OK;`）：本 derive 不认识 crate 路径，
//! 与 `#[derive(Fail)]` 只管"域内自 `-1` 起"同一条纪律——**一处定义，别处不写第二个数**。

use proc_macro2::{Literal, TokenStream as TokenStream2};
use quote::quote;
use syn::punctuated::Punctuated;
use syn::{Attribute, Data, DeriveInput, Expr, ExprLit, Fields, Ident, Lit, Meta, Token, parse2};

/// 展开见文件头。
pub fn expand(input: TokenStream2) -> TokenStream2 {
    let ast = match parse2::<DeriveInput>(input) {
        Ok(ast) => ast,
        Err(e) => return e.to_compile_error(),
    };
    let name = &ast.ident;
    let data = match &ast.data {
        Data::Enum(e) => e,
        _ => {
            return syn::Error::new_spanned(&ast, "WireCodes 只用于枚举（失败词汇）").to_compile_error();
        }
    };

    // 枚举上那一格：`also(NAME = N, …)` 与 `fallback = 变体`。
    let mut also: Vec<(Ident, u8)> = Vec::new();
    let mut fallback: Option<Ident> = None;
    for a in &ast.attrs {
        if !a.path().is_ident("wire") {
            continue;
        }
        let metas = match a.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated) {
            Ok(m) => m,
            Err(e) => return e.to_compile_error(),
        };
        for m in metas {
            match &m {
                Meta::List(l) if l.path.is_ident("also") => {
                    let inner = match l.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated) {
                        Ok(i) => i,
                        Err(e) => return e.to_compile_error(),
                    };
                    for cell in inner {
                        match cell {
                            Meta::NameValue(nv) => match int_of(&nv.value) {
                                Some(v) => also.push((path_ident(&nv.path), v)),
                                None => {
                                    return syn::Error::new_spanned(&nv.value, "also 的码要是 0..=255 的整数")
                                        .to_compile_error();
                                }
                            },
                            other => {
                                return syn::Error::new_spanned(other, "also 里写成 `名 = 码`")
                                    .to_compile_error();
                            }
                        }
                    }
                }
                Meta::NameValue(nv) if nv.path.is_ident("fallback") => match ident_of(&nv.value) {
                    Some(id) => fallback = Some(id),
                    None => {
                        return syn::Error::new_spanned(&nv.value, "fallback 写一枚变体名")
                            .to_compile_error();
                    }
                },
                other => {
                    return syn::Error::new_spanned(other, "wire 只认 `also(名 = 码, …)` 与 `fallback = 变体`")
                        .to_compile_error();
                }
            }
        }
    }

    // 每枚变体：`#[code(N)]` / `#[code(N, 名)]`；名默认取变体名大写。
    let mut idents: Vec<&Ident> = Vec::new();
    let mut cnames: Vec<Ident> = Vec::new();
    let mut codes: Vec<u8> = Vec::new();
    let mut docs: Vec<Vec<&Attribute>> = Vec::new();
    for v in &data.variants {
        if !matches!(v.fields, Fields::Unit) {
            return syn::Error::new_spanned(v, "失败词汇的变体不带字段：一枚变体就是一格码")
                .to_compile_error();
        }
        let mut code: Option<u8> = None;
        let mut cname: Option<Ident> = None;
        for a in &v.attrs {
            if !a.path().is_ident("code") {
                continue;
            }
            let vals = match a.parse_args_with(Punctuated::<Expr, Token![,]>::parse_terminated) {
                Ok(v) => v,
                Err(e) => return e.to_compile_error(),
            };
            let mut it = vals.into_iter();
            match it.next().and_then(|e| int_of(&e)) {
                Some(n) => code = Some(n),
                None => {
                    return syn::Error::new_spanned(a, "code 至少要给一个码：`#[code(3)]` / `#[code(3, DENIED)]`")
                        .to_compile_error();
                }
            }
            if let Some(e) = it.next() {
                match ident_of(&e) {
                    Some(id) => cname = Some(id),
                    None => {
                        return syn::Error::new_spanned(e, "code 第二个参数是码名（一枚标识符）")
                            .to_compile_error();
                    }
                }
            }
        }
        let Some(code) = code else {
            return syn::Error::new_spanned(
                v,
                "这一枚没写落在线上哪一格：加 `#[code(码)]`（不打算上线的变体不该出现在失败词汇里）",
            )
            .to_compile_error();
        };
        let cname = cname.unwrap_or_else(|| Ident::new(&v.ident.to_string().to_uppercase(), v.ident.span()));
        idents.push(&v.ident);
        cnames.push(cname);
        codes.push(code);
        docs.push(v.attrs.iter().filter(|a| a.path().is_ident("doc")).collect());
    }

    let consts = cnames.iter().zip(&codes).zip(&docs).map(|((c, n), d)| {
        let n = Literal::u8_unsuffixed(*n);
        quote! { #(#d)* pub const #c: u8 = #n; }
    });
    let also_consts = also.iter().map(|(c, n)| {
        let n = Literal::u8_unsuffixed(*n);
        quote! { pub const #c: u8 = #n; }
    });
    let tail = match &fallback {
        Some(fb) => quote! { Some(#name::#fb) },
        None => quote! { None },
    };
    // 码唯一（逐对）＋ 不与 `OK` 撞。
    let mut uniq = Vec::new();
    for i in 0..codes.len() {
        let a = &cnames[i];
        uniq.push(quote! { assert!(#a != OK); });
        for j in (i + 1)..codes.len() {
            let b = &cnames[j];
            uniq.push(quote! { assert!(#a != #b, "两格共用一个码"); });
        }
    }
    let also_checks = also.iter().map(|(c, _)| match &fallback {
        Some(fb) => quote! { assert!(matches!(code_to_fail(#c), Some(#name::#fb))); },
        None => quote! { assert!(code_to_fail(#c).is_none()); },
    });

    quote! {
        #(#consts)*
        #(#also_consts)*

        /// 失败域 → 线上那一格；`None`（没失败）⇒ `OK`。
        pub const fn fail_to_code(fail: Option<#name>) -> u8 {
            match fail {
                None => OK,
                #(Some(#name::#idents) => #cnames,)*
            }
        }

        /// 线上那一格 → 失败域；`OK` ⇒ `None`，表外 ⇒ `None`（或枚举上那枚 `fallback`）。
        pub const fn code_to_fail(code: u8) -> Option<#name> {
            match code {
                OK => None,
                #(#cnames => Some(#name::#idents),)*
                _ => #tail,
            }
        }

        /// 逐格锁死：码唯一、不与 `OK` 撞、逐枚读得回来。
        const _: () = {
            #(#uniq)*
            #(assert!(matches!(code_to_fail(#cnames), Some(#name::#idents)));)*
            #(#also_checks)*
        };
    }
}

/// `名 = 值` 里的那枚名（`also` 用）。
fn path_ident(p: &syn::Path) -> Ident {
    p.segments
        .last()
        .map(|s| s.ident.clone())
        .unwrap_or_else(|| Ident::new("?", proc_macro2::Span::call_site()))
}

/// 表达式里那枚标识符（`fallback = X` 与 `#[code(3, X)]` 用）。
fn ident_of(e: &Expr) -> Option<Ident> {
    match e {
        Expr::Path(p) => p.path.get_ident().cloned(),
        _ => None,
    }
}

/// 0..=255 的整数（`code(…)` 与 `also(… = N)` 用）。
fn int_of(e: &Expr) -> Option<u8> {
    match e {
        Expr::Lit(ExprLit { lit: Lit::Int(n), .. }) => n.base10_parse::<u8>().ok(),
        _ => None,
    }
}
