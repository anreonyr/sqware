//! `#[derive(Frame)]` —— **帧**那一族的一处定义：给一枚具名字段的结构体，生成 `LEN` ＋
//! `store` / `store_at` / `fetch` / `fetch_at`（**结构体归你写**——它本就是那张字段表）。
//!
//! ```ignore
//! #[derive(env::Frame)]
//! #[derive(Clone, Copy, PartialEq, Eq, Debug)]
//! pub struct Tip {
//!     pub who: TaskId,
//!     pub name: Tag,
//!     pub reply: PieToken,
//! }
//! ```
//!
//! 生成的东西**一眼看得完**（没有隐藏机制）：`pub const LEN`（**最长那一形**：各格的
//! [`env::wire::Span::MAX`] 求和）、`store(&self, &mut [u8; LEN])`（**只有全定长那一形才有**）、
//! `store_at(&self, &mut [u8], at) -> Option<usize>`、
//! `fetch(&[u8]) -> Option<Self>`、`fetch_at(&[u8], at) -> Option<(Self, usize)>`。
//!
//! **偏移一处都不写**——两半由**同一张字段表**生成，故"同一条长度写两处、改一处漏一处
//! **编得过**"那个病**写不出来**（协调那一帧栽的正是它：那边的照实记写着"靠注释说必须同值"）。
//! **变长那一形也一样**：`#[frame(count = <条数那一格>, fill = <空位初值>)]` 标在 `[T; CAP]`
//! 那一格上，它按 [`env::wire::store_tail`] / [`env::wire::fetch_tail`] 走游标——族里不再手写
//! `2 + i * WIDTH` 这种句子（用户裁定的"尾巴不许手写"，到这一手才成为机制）。
//!
//! **它只管"一张字段表"与"一段重复"**：多形分派（按动作码 / 长度 / 首格选形）**不归它**，
//! 那几族各有各的手（`Req` / `Union` / `Tip`），derive 只管每一形**内部**的顺序与偏移。
//!
//! **字段的字节编解码归 [`env::wire::Span`]**（定长那一格是它的 blanket impl）：derive 只负责
//! "顺序与游标"，一格自己是多宽、怎么写，是那一格自己的事。
//!
//! **照实记（它走过三站）**：它从前是 `env/src/wire/field.rs` 里的一条 `macro_rules!`
//! （`#[macro_export]`，故名字落在 `env` 的 **crate 根**上，与同 crate 里的同名模块撞过车）；
//! 随后改成 function-like 的过程宏 `frame!`（诊断从此能指到**那一格字段**）；今天收成
//! `#[derive(Frame)]`。**为什么最后一站是 derive**：`frame!` 吃进去的那张字段表**本身就是一枚
//! 结构体**——宏却替用户把它写了一遍（连各格的 `pub` 与那行
//! `#[derive(Clone, Copy, PartialEq, Eq, Debug)]` 都是宏注入的）。收成 derive 之后那枚结构体
//! 回到源码里：字段可见性、字段上的文档、IDE 的跳转都在用户那一边看得见，而生成的 `LEN` /
//! `store` / `fetch` **一个字没变**（展开物逐字节比对过）。
//!
//! 本文件是该宏的全部——**与另外两个宏一行都不共享**。

use proc_macro2::{Ident, Span, TokenStream};
use quote::quote;
use syn::{Data, DeriveInput, Fields, parse2};

/// 给一枚具名字段的结构体生成：`LEN` ＋ `store` / `store_at` / `fetch` / `fetch_at`。
/// 展开见文件头。
///
/// **生成的路径是 `::env::wire::Span`**（过程宏没有 `$crate`）：故调用方的 extern prelude
/// 里要有 `env`——`protocol` / `programs` 都有。`env` 自己若要这个 derive，先写一句
/// `extern crate self as env;`（今天没有这个需要）。
pub fn expand(input: TokenStream) -> TokenStream {
    let ast: DeriveInput = match parse2(input) {
        Ok(ast) => ast,
        Err(e) => return e.to_compile_error(),
    };
    let name = &ast.ident;
    let Data::Struct(data) = &ast.data else {
        return syn::Error::new_spanned(&ast, "`Frame` 只吃结构体（一张帧的字段表）")
            .to_compile_error();
    };
    let Fields::Named(named) = &data.fields else {
        return syn::Error::new_spanned(&ast, "`Frame` 只吃具名字段的帧").to_compile_error();
    };
    if named.named.is_empty() {
        return syn::Error::new_spanned(&ast, "一张字段表至少要有一格").to_compile_error();
    }
    // 参数那一格**没有对应物**：生成的 `impl` 不转发 `generics`，放过去只会换来一句
    // `missing generics for struct`——在这里当场报。
    if !ast.generics.params.is_empty() || ast.generics.where_clause.is_some() {
        return syn::Error::new_spanned(
            &ast.generics,
            "`Frame` 不支持带参数的结构体：帧的字段表没有参数那一格",
        )
        .to_compile_error();
    }

    let mut idents = Vec::new();
    let mut maxes = Vec::new();
    let mut stores = Vec::new();
    let mut fetches = Vec::new();
    let mut seen: Vec<Ident> = Vec::new();
    // "一段重复"那一格：它在 ⇒ 这一表是**变长**的，`store` 那一手不给（它给不出长度）。
    // 一处两处都行——每一处只要求它自己的条数格**声明得更靠前**（下面逐格判）。
    let mut tailed = false;

    // **游标与那几只局部要**卫生**（`Span::mixed_site`）**：生成的 `let at = …` 与调用方
    // 那几格字段是**两个不同的标识符**。照实记：这一条是"树那一族"那一刀当场撞出来的——
    // 它的字段表里有一格就叫 `at`（容器坐标），宏自己的游标被那格遮住，报的是
    // `binary assignment operation += cannot be applied to type Where`。**这是宏的错，不是
    // 表的错**：`at` 是个再自然不过的字段名。
    let at = Ident::new("at", Span::mixed_site());
    let one = Ident::new("one", Span::mixed_site());
    let k = Ident::new("k", Span::mixed_site());
    let arr = Ident::new("arr", Span::mixed_site());

    for field in &named.named {
        let ident = field.ident.clone().expect("具名字段");
        let ty = field.ty.clone();
        let shape = match shape(field) {
            Ok(shape) => shape,
            Err(e) => return e.to_compile_error(),
        };
        match shape {
            None => {
                maxes.push(quote!(<#ty as ::env::wire::Span>::MAX));
                stores.push(quote! {
                    #at = <#ty as ::env::wire::Span>::store_at(&self.#ident, out, #at)?;
                });
                fetches.push(quote! {
                    let #one = <#ty as ::env::wire::Span>::fetch_at(bytes, #at)?;
                    let #ident = #one.0;
                    #at = #one.1;
                });
            }
            Some(Tail { count, fill }) => {
                let syn::Type::Array(array) = &ty else {
                    return syn::Error::new_spanned(
                        &ty,
                        "`count` 只能标在数组格上（`[T; CAP]` 那一形：定容容器）",
                    )
                    .to_compile_error();
                };
                // 条数那一格必须**更靠前**（fetch 要先把它读出来），而且是整数（生成的正文要 `as usize`）。
                if !seen.contains(&count) {
                    return syn::Error::new_spanned(
                        field,
                        format!("`count = {count}`：这一格必须在它之前声明（读的时候要先把它读出来）"),
                    )
                    .to_compile_error();
                }
                let count_ty = named
                    .named
                    .iter()
                    .find(|f| f.ident.as_ref() == Some(&count))
                    .map(|f| f.ty.clone())
                    .expect("上面刚判过它在");
                if !is_int(&count_ty) {
                    return syn::Error::new_spanned(
                        &count_ty,
                        "条数那一格要是整数（`u8` / `u32` / `usize` …）",
                    )
                    .to_compile_error();
                }
                let (elem, cap) = (&array.elem, &array.len);
                tailed = true;
                maxes.push(quote!(#cap * <#elem as ::env::wire::Field>::WIDTH));
                stores.push(quote! {
                    {
                        let #k = self.#count as usize;
                        if #k > #cap {
                            return None;
                        }
                        #at = ::env::wire::store_tail(out, #at, &self.#ident[..#k])?;
                    }
                });
                fetches.push(quote! {
                    let #ident = {
                        let #k = #count as usize;
                        if #k > #cap {
                            return None;
                        }
                        let mut #arr = [#fill; #cap];
                        ::env::wire::fetch_tail(bytes, #at, &mut #arr[..#k])?;
                        #at += #k * <#elem as ::env::wire::Field>::WIDTH;
                        #arr
                    };
                });
            }
        }
        seen.push(ident.clone());
        idents.push(ident);
    }

    // 变长那一形**不给 `store`**：写进一只定长数组之后"这一帧几字节"就丢了——留着它会变成
    // 一把静默的短刀（族里要长度，走 `store_at`）。
    let store = if tailed {
        quote!()
    } else {
        quote! {
            /// 写进 `out`（**缓冲刚好这么大**——静态成立，故这一手不可能失败）。
            pub fn store(&self, out: &mut [u8; Self::LEN]) {
                // 恒 `Some`：`out` 恰好 `LEN` 字节。不是吞失败。
                let _ = self.store_at(out, 0);
            }
        }
    };

    quote! {
        impl #name {
            /// 这一帧**最长那一形**占几字节：各格的 `env::wire::Span::MAX` 求和（一处定义）。
            pub const LEN: usize = 0 #(+ #maxes)*;

            #store

            /// 从游标 `at` 写起，返**实际长度**（装不下 ⇒ `None`）。
            ///
            /// **照实记（为什么有这一手，而不是只有 `store`）**：`store` 要的是定长数组
            /// （`&mut [u8; LEN]`），而一族常常**只有一只缓冲、形状各有长短**（板那族是
            /// 41 / 33 / 1）——从大缓冲里切出来的 `&mut [u8]` 转不回定长数组；变长那一形更给不出
            /// "刚好"那个长度。这一手就是那一格：不 `expect`、不拷贝一次。
            ///
            /// **照实记（装不下时前面几格可能已经写了）**：逐格写、边写边判，故 `None` 不保证
            /// "一支笔都没落"。写不进去本来不是正常路径（各族的缓冲按 `LEN` 开）。
            pub fn store_at(&self, out: &mut [u8], mut #at: usize) -> Option<usize> {
                #(#stores)*
                let _ = #at;
                Some(#at)
            }

            /// 从 `bytes` 头上读回来；**长度不足** ⇒ `None`（不猜、不崩）。
            ///
            /// **它不判尾部长度**——"恰好"还是"够长"是各族的判据（见各族的 `Message::fetch`）。
            pub fn fetch(bytes: &[u8]) -> Option<Self> {
                Self::fetch_at(bytes, 0).map(|#one| #one.0)
            }

            /// 从游标 `at` 读起，返**值与读完之后的游标**。
            pub fn fetch_at(bytes: &[u8], mut #at: usize) -> Option<(Self, usize)> {
                #(#fetches)*
                let _ = #at;
                Some((Self { #(#idents),* }, #at))
            }
        }
    }
}

/// 一格上的 `#[frame(count = <条数那一格>, fill = <空位那一枚>)]`。
///
/// 没有这条属性 ⇒ `None`（定长那一格）。
struct Tail {
    count: Ident,
    fill: syn::Expr,
}

/// 读一格自己的形状：`#[frame(...)]` 有 ⇒ 这一段是"重复"，没有 ⇒ 定长。
fn shape(field: &syn::Field) -> syn::Result<Option<Tail>> {
    let mut count: Option<Ident> = None;
    let mut fill: Option<syn::Expr> = None;
    for attr in &field.attrs {
        if !attr.path().is_ident("frame") {
            continue;
        }
        let mut said = false;
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("count") {
                count = Some(meta.value()?.parse()?);
            } else if meta.path.is_ident("fill") {
                fill = Some(meta.value()?.parse()?);
            } else {
                return Err(meta.error(
                    "`#[frame(...)]` 只认 `count = <条数那一格>` 与 `fill = <空位那一枚>`",
                ));
            }
            said = true;
            Ok(())
        })?;
        if !said {
            return Err(syn::Error::new_spanned(
                attr,
                "`#[frame(...)]` 是空的：一段重复要写 `count = <条数那一格>, fill = <空位那一枚>`",
            ));
        }
    }
    match (count, fill) {
        (None, None) => Ok(None),
        (Some(count), Some(fill)) => Ok(Some(Tail { count, fill })),
        _ => Err(syn::Error::new_spanned(
            field,
            "`count` 与 `fill` 要一起给：前者说条数在哪一格，后者说空位那一枚是什么",
        )),
    }
}

/// 条数那一格得是整数——生成的正文要 `as usize`。
fn is_int(ty: &syn::Type) -> bool {
    let syn::Type::Path(path) = ty else {
        return false;
    };
    let Some(seg) = path.path.segments.last() else {
        return false;
    };
    matches!(
        seg.ident.to_string().as_str(),
        "u8" | "u16" | "u32" | "u64" | "u128" | "usize"
    )
}
