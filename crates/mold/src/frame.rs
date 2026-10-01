//! `#[derive(Frame)]` —— **帧**那一族的一处定义：给一枚具名字段的结构体，生成 `LEN` ＋
//! `store_at` / `fetch` / `fetch_at`（**结构体归你写**——它本就是那张字段表）。
//!
//! ```ignore
//! #[derive(env::Frame)]
//! #[derive(Clone, Copy, PartialEq, Eq, Debug)]
//! pub struct Tip {
//!     pub who: TaskId,
//!     pub name: Slot<31>,
//!     pub reply: PieToken,
//! }
//! ```
//!
//! 生成的东西**一眼看得完**（没有隐藏机制）：`pub const LEN`（**最长那一形**：各格的
//! [`env::wire::Span::MAX`] 求和，一段重复按 `MAX × 条数`；**有格不报上界时**由族写
//! `#[frame(len = …)]` 给）、
//! `store_at(&self, &mut [u8], at) -> Option<usize>`、
//! `fetch(&[u8]) -> Option<Self>`、`fetch_at(&[u8], at) -> Option<(Self, usize)>`。
//!
//! **偏移一处都不写**——两半由**同一张字段表**生成，故"同一条长度写两处、改一处漏一处
//! **编得过**"那个病**写不出来**（协调那一帧栽的正是它：那边写着"靠注释说必须同值"）。
//! **变长那一形也一样**：`#[frame(count = <条数那一格>, fill = <空位初值>)]` 标在 `[T; CAP]`
//! 那一格上，它按 [`env::wire::store_tail`] / [`env::wire::fetch_tail`] 走游标——那两个函数认
//! [`env::wire::Span`]，故**定长项与变长项同一条路**，族里不再手写
//! `2 + i * WIDTH` 这种句子（用户裁定的"尾巴不许手写"，到这一手才成为机制）。
//!
//! **它只管"一张字段表"与"一段重复"**：多形分派（按动作码 / 长度 / 首格选形）**不归它**，
//! 那几族各有各的手（`Req` / `Union` / `Tip`），derive 只管每一形**内部**的顺序与偏移。
//!
//! **字段的字节编解码归 [`env::wire::Span`]**（定长那一格是它的 blanket impl）：derive 只负责
//! "顺序与游标"，一格自己是多宽、怎么写，是那一格自己的事。
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
    let len = match frame_len(&ast.attrs) {
        Ok(len) => len,
        Err(e) => return e.to_compile_error(),
    };
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
                        format!(
                            "`count = {count}`：这一格必须在它之前声明（读的时候要先把它读出来）"
                        ),
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
                // **上界**：一段重复最长就是"每一项都是最长那一形"（定长项的 `MAX` 就是它的宽）；
                // 元素不报上界 ⇒ 这一段也不报（`times` 交回 `None`）。
                maxes.push(quote!(::env::wire::times(#cap, <#elem as ::env::wire::Span>::MAX)));
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
                        let mut #arr = [const { #fill }; #cap];
                        #at = ::env::wire::fetch_tail(bytes, #at, &mut #arr[..#k])?;
                        #arr
                    };
                });
            }
        }
        seen.push(ident.clone());
        idents.push(ident);
    }

    // `LEN` 两形：**族给了**就直接用它；**没给**就各格求和——求和时遇到"不报上界"的那一格
    // （名字是 `String`，`MAX = None`）当场编不过，报的就是该写什么。
    let len_def = match &len {
        Some(expr) => quote! {
            /// 这一帧**最长那一形**占几字节：**由族说**（`#[frame(len = …)]`）——因为这一表里有格
            /// 不报上界（名字那一格是 `String`），各格求和求不出来。
            pub const LEN: usize = #expr;
        },
        None => quote! {
            /// 这一帧**最长那一形**占几字节：各格的 `env::wire::Span::MAX` 求和、一段重复按
            /// `MAX × 条数`（一处定义）。
            pub const LEN: usize = match ::env::wire::total(&[#(#maxes),*]) {
                Some(total) => total,
                None => panic!(
                    "这一帧有格不报上界（名字是 String，MAX = None）：给这一帧写 #[frame(len = …)]"
                ),
            };
        },
    };

    quote! {
        impl #name {
            #len_def

            /// 从游标 `at` 写起，返**实际长度**（装不下 ⇒ `None`）。
            ///
            /// **实际长度不等于 `LEN`**：变长那一格只写有效字节，`LEN` 是**上界**（缓冲按它备）。
            ///
            /// **（装不下时前面几格可能已经写了）**：逐格写、边写边判，故 `None` 不保证
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

/// 结构体上的 `#[frame(len = <这一帧的上界>)]`。
///
/// **什么时候要它**：这一表里有格**不报上界**（名字那一格是 `String`，`MAX = None`）——那时各格
/// 求和不出来，界由**族**说（协议事实：这一族最长多少）。全定长／定容的帧不必写，求和自动。
fn frame_len(attrs: &[syn::Attribute]) -> syn::Result<Option<syn::Expr>> {
    let mut len: Option<syn::Expr> = None;
    for attr in attrs {
        if !attr.path().is_ident("frame") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("len") {
                if len.is_some() {
                    return Err(meta.error("`len` 给了两次"));
                }
                len = Some(meta.value()?.parse()?);
                Ok(())
            } else {
                Err(meta.error(
                    "结构体上的 `#[frame(...)]` 只认 `len = <这一帧的上界>`（`count` / `fill` 标在字段上）",
                ))
            }
        })?;
    }
    Ok(len)
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
