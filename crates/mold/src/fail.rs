//! `#[derive(Fail)]` —— **域失败词汇**：域内自 `-1` 起的码，与它的读法。
//!
//! 输入一枚**无字段枚举**，每个变体显式写号：
//!
//! ```ignore
//! #[derive(Fail)]
//! pub enum MemoryFail {
//!     Denied = -1,
//!     OoM = -2,
//!     NotAligned = -3,
//! }
//! ```
//!
//! 产出（**这一处就是码的唯一真相**，别处一个数都不写）：
//!   * `pub const fn code(self) -> isize`（判别值即码）＋ `pub const fn of_code(code) -> Option<Self>`
//!   * `impl FailCode`（`code()`）＋ `Clone`/`Copy`/`Debug`/`PartialEq`/`Eq`/`Display`
//!   * 标了 `#[busy]` 的那一枚额外给 `pub const fn is_busy(self) -> bool`
//!   * 编译期断言：号自 `-1` 起按声明顺序连续、逐枚读得回来、表外答 `None`
//!
//! **号是各域自己的**：域内从 `-1` 起连续。同一条件在不同域不同号——读法按域
//! （调用点知道自己在调哪一域），故本 derive **不做也不要求**任何"全局唯一"检查。
//!
//! `Display` 出 `<域>:<变体名>`（域 = 枚举名去掉 `Fail` 后缀、小写）——跨域日志靠
//! 域名分，不靠号分。

use proc_macro2::{Literal, TokenStream as TokenStream2};
use quote::quote;
use syn::{Attribute, Data, DeriveInput, Expr, Fields, Lit, parse2};

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
            return syn::Error::new_spanned(&ast, "Fail 只用于枚举（域失败词汇）")
                .to_compile_error();
        }
    };
    if data.variants.is_empty() {
        return syn::Error::new_spanned(&ast, "空词表没有意义：一枚变体都没有").to_compile_error();
    }

    // 域名词：`MemoryFail` → `memory`。
    let full = name.to_string();
    let domain = full
        .strip_suffix("Fail")
        .unwrap_or(&full)
        .to_lowercase();

    let mut idents = Vec::new();
    let mut codes = Vec::new();
    let mut busy = None;
    for (at, v) in data.variants.iter().enumerate() {
        if !matches!(v.fields, Fields::Unit) {
            return syn::Error::new_spanned(v, "失败词汇的变体不带字段：一枚变体就是一格码")
                .to_compile_error();
        }
        let want = -(at as isize) - 1;
        let got = v.discriminant.as_ref().and_then(|(_, e)| int_of(e));
        match got {
            Some(g) if g == want => {}
            Some(g) => {
                return syn::Error::new_spanned(
                    v,
                    format!("这一格的号必须是 {want}（域内自 -1 起、按声明顺序连续），写的是 {g}"),
                )
                .to_compile_error();
            }
            None => {
                return syn::Error::new_spanned(
                    v,
                    format!("这一格要显式写号：`= {want}`（域内自 -1 起、按声明顺序连续）"),
                )
                .to_compile_error();
            }
        }
        if has_busy(&v.attrs) {
            if busy.is_some() {
                return syn::Error::new_spanned(
                    v,
                    "一个域只有一枚 `#[busy]`（「条件未就绪」是单数）",
                )
                .to_compile_error();
            }
            busy = Some(&v.ident);
        }
        idents.push(&v.ident);
        codes.push(Literal::isize_unsuffixed(want));
    }

    let out = Literal::isize_unsuffixed(-(idents.len() as isize) - 1);
    let domain_lit = Literal::string(&domain);
    let busy_fn = busy.map(|b| {
        quote! {
            impl #name {
                /// 条件未就绪（`#[busy]` 那一枚）——非阻塞原语的可重试信号。
                pub const fn is_busy(self) -> bool {
                    matches!(self, Self::#b)
                }
            }
        }
    });

    quote! {
        impl #name {
            /// 域内判别值：**自 `-1` 起按声明顺序连续**（判别值即码）。
            pub const fn code(self) -> isize {
                self as isize
            }

            /// 码读回词汇；表外答 `None`（调用点当场 `unreachable!`：表外码 = 不变量破裂）。
            pub const fn of_code(code: isize) -> Option<Self> {
                match code {
                    #(#codes => Some(Self::#idents),)*
                    _ => None,
                }
            }
        }

        impl crate::FailCode for #name {
            fn code(self) -> isize {
                self.code()
            }
        }

        impl Clone for #name {
            fn clone(&self) -> Self {
                *self
            }
        }

        impl Copy for #name {}

        impl core::fmt::Debug for #name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                let name = match self {
                    #(Self::#idents => stringify!(#idents),)*
                };
                core::write!(f, "{}::{}", stringify!(#name), name)
            }
        }

        impl PartialEq for #name {
            fn eq(&self, other: &Self) -> bool {
                self.code() == other.code()
            }
        }

        impl Eq for #name {}

        impl core::fmt::Display for #name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                use core::fmt::Write as _;
                let name = match self {
                    #(Self::#idents => stringify!(#idents),)*
                };
                write!(f, "{}: {}", #domain_lit, name)
            }
        }

        #busy_fn

        /// 逐域锁死：号自 `-1` 起连续、逐枚读得回来、表外答 `None`。
        /// 同 `env::ecall::Fail` 那两条编译期断言的纪律——**错一枚编不过**。
        const _: () = {
            #(assert!(#name::#idents.code() == #codes);)*
            #(assert!(matches!(#name::of_code(#codes), Some(#name::#idents)));)*
            assert!(#name::of_code(0).is_none());
            assert!(#name::of_code(1).is_none());
            assert!(#name::of_code(#out).is_none());
        };
    }
}

/// `#[busy]` 标记（derive 的辅助属性，见 `lib.rs` 的 `attributes(busy)`）。
fn has_busy(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|a| a.path().is_ident("busy"))
}

/// 判别值里的整数——**`-1` 在语法上是"一元负号 + 字面量"**，不是 `Expr::Lit`。
fn int_of(e: &Expr) -> Option<isize> {
    match e {
        Expr::Lit(syn::ExprLit {
            lit: Lit::Int(n), ..
        }) => n.base10_parse::<isize>().ok(),
        Expr::Unary(syn::ExprUnary {
            op: syn::UnOp::Neg(_),
            expr,
            ..
        }) => int_of(expr).map(|v| -v),
        _ => None,
    }
}
