//! `#[derive(Frame)]` —— **定长帧**那一族的一处定义：给一枚具名字段的结构体，生成 `LEN` ＋
//! `store` / `store_in` / `fetch` 三手（**结构体归你写**——它本就是那张字段表）。
//!
//! ```ignore
//! #[derive(env::Frame)]
//! #[derive(Clone, Copy, PartialEq, Eq, Debug)]
//! pub struct Tip {
//!     pub who: TaskId,
//!     pub name: Name,
//!     pub reply: PieToken,
//! }
//! ```
//!
//! 生成的东西**一眼看得完**（没有隐藏机制）：`pub const LEN`（**字段宽度之和**）、
//! `store(&self, &mut [u8; LEN])`、`store_in(&mut [u8]) -> Option<usize>`、
//! `fetch(&[u8]) -> Option<Self>`。
//!
//! **偏移一处都不写**——两半由**同一张字段表**生成，故"同一条长度写两处、改一处漏一处
//! **编得过**"那个病**写不出来**（协调那一帧栽的正是它：那边的照实记写着"靠注释说必须同值"）。
//!
//! **它只管定长字段序列**：变长（一条路几段不定）与重复（计数 ＋ 一段数组）那两类**不归它**，
//! 那几族各有各的手写 `store` / `fetch`（今住在各族的 `impl Message` 里）。
//!
//! **字段的字节编解码归 [`env::wire::Field`]**（`WIDTH` / `store` / `fetch`）：derive 只负责
//! "顺序与偏移"，一格自己是多宽、怎么写，是那一格自己的事。
//!
//! **照实记（它走过三站）**：它从前是 `env/src/wire/field.rs` 里的一条 `macro_rules!`
//! （`#[macro_export]`，故名字落在 `env` 的 **crate 根**上，与同 crate 里的同名模块撞过车）；
//! 随后改成 function-like 的过程宏 `frame!`（诊断从此能指到**那一格字段**）；今天收成
//! `#[derive(Frame)]`。**为什么最后一站是 derive**：`frame!` 吃进去的那张字段表**本身就是一枚
//! 结构体**——宏却替用户把它写了一遍（连各格的 `pub` 与那行
//! `#[derive(Clone, Copy, PartialEq, Eq, Debug)]` 都是宏注入的）。收成 derive 之后那枚结构体
//! 回到源码里：字段可见性、字段上的文档、IDE 的跳转都在用户那一边看得见，而生成的 `LEN` /
//! `store` / `store_in` / `fetch` **一个字没变**（展开物逐字节比对过）。
//!
//! 本文件是该宏的全部——**与另外两个宏一行都不共享**。

use proc_macro2::{Ident, Span, TokenStream};
use quote::quote;
use syn::{Data, DeriveInput, Fields, parse2};

/// 给一枚具名字段的结构体生成：`LEN` ＋ `store` / `store_in` / `fetch`。展开见文件头。
///
/// **生成的路径是 `::env::wire::Field`**（过程宏没有 `$crate`）：故调用方的 extern prelude
/// 里要有 `env`——`contract` / `programs` 都有。`env` 自己若要这个 derive，先写一句
/// `extern crate self as env;`（今天没有这个需要）。
pub fn expand(input: TokenStream) -> TokenStream {
    let ast: DeriveInput = match parse2(input) {
        Ok(ast) => ast,
        Err(e) => return e.to_compile_error(),
    };
    let name = &ast.ident;
    let Data::Struct(data) = &ast.data else {
        return syn::Error::new_spanned(&ast, "`Frame` 只吃结构体（一张定长帧的字段表）")
            .to_compile_error();
    };
    let Fields::Named(named) = &data.fields else {
        return syn::Error::new_spanned(&ast, "`Frame` 只吃具名字段的定长帧").to_compile_error();
    };
    if named.named.is_empty() {
        return syn::Error::new_spanned(&ast, "一张字段表至少要有一格").to_compile_error();
    }
    // 参数那一格**没有对应物**：生成的 `impl` 不转发 `generics`，放过去只会换来一句
    // `missing generics for struct`——在这里当场报。
    if !ast.generics.params.is_empty() || ast.generics.where_clause.is_some() {
        return syn::Error::new_spanned(
            &ast.generics,
            "`Frame` 不支持带参数的结构体：定长帧的字段表没有参数那一格",
        )
        .to_compile_error();
    }
    let mut idents = Vec::new();
    let mut types = Vec::new();
    for field in &named.named {
        idents.push(field.ident.clone().expect("具名字段"));
        types.push(field.ty.clone());
    }

    // **游标与那一截缓冲的名字要**卫生**（`Span::mixed_site`）**：生成的 `let at = …` 与调用方
    // 那几格字段是**两个不同的标识符**。照实记：这一条是"树那一族"那一刀当场撞出来的——
    // 它的字段表里有一格就叫 `at`（容器坐标），宏自己的游标被那格遮住，报的是
    // `binary assignment operation += cannot be applied to type Where`。**这是宏的错，不是
    // 表的错**：`at` 是个再自然不过的字段名。
    let at = Ident::new("at", Span::mixed_site());
    let head = Ident::new("head", Span::mixed_site());

    quote! {
        impl #name {
            /// 这一帧线上占几字节：**字段宽度之和**（一处定义）。
            pub const LEN: usize = 0 #(+ <#types as ::env::wire::Field>::WIDTH)*;

            /// 写进 `out`（**缓冲刚好这么大**——静态成立，故这一手不可能失败）。
            pub fn store(&self, out: &mut [u8; Self::LEN]) {
                // 恒 `Some`：`out` 恰好 `LEN` 字节。不是吞失败。
                let _ = self.store_in(out);
            }

            /// 写进一只**更大的**缓冲：`out.len() < LEN` ⇒ `None`，否则写完返 [`Self::LEN`]。
            ///
            /// **照实记（为什么还要这一版）**：`store` 要的是定长数组（`&mut [u8; LEN]`），而
            /// 一族常常**只有一只缓冲、形状各有长短**（板那族是 41 / 33 / 1）——从大缓冲里切出来
            /// 的 `&mut [u8]` 转不回定长数组。这一版就是那一格：不 `expect`、不拷贝一次。
            pub fn store_in(&self, out: &mut [u8]) -> Option<usize> {
                let #head = out.get_mut(..Self::LEN)?;
                let mut #at = 0usize;
                #(
                    <#types as ::env::wire::Field>::store(
                        &self.#idents,
                        &mut #head[#at..#at + <#types as ::env::wire::Field>::WIDTH],
                    );
                    #at += <#types as ::env::wire::Field>::WIDTH;
                )*
                let _ = #at;
                Some(Self::LEN)
            }

            /// 从 `bytes` 读回来；**长度不足** ⇒ `None`（不猜、不崩）。
            pub fn fetch(bytes: &[u8]) -> Option<Self> {
                let mut #at = 0usize;
                #(
                    let #idents = <#types as ::env::wire::Field>::fetch(
                        bytes.get(#at..#at + <#types as ::env::wire::Field>::WIDTH)?,
                    )?;
                    #at += <#types as ::env::wire::Field>::WIDTH;
                )*
                let _ = #at;
                Some(Self { #(#idents),* })
            }
        }
    }
}
