//! `#[derive(Envcall)]` —— **环境调用枚举**的载荷 codec（方案 3，typed payload）。
//!
//! 输入一个带载荷的调用枚举，输出：
//!   * `slot(&self) -> usize`       —— 调用号（`#[call(class = N)]` 的 class << 32 | 判别号）
//!   * `pack(&self) -> [usize; 6]`  —— 字段按声明顺序 wire 化（`Wire::pack`）
//!   * `from_wire(slot, &[usize; 6])` —— 按 slot 取 variant，逐字段 `Wire::unpack`
//!   * `Ret` 枚举                   —— 每个标 `#[ret(T)]` 的 variant 一个载荷变体
//!   * `call(self) -> EnvResult<Ret>`—— 触发并判译（负值即错误）
//!
//! **两种返回宽度**：`#[ret(T)]` 走 `wire::FromPair`（读 `a0`/`a1`），`#[ret3(T)]` 走
//! `wire::FromTriple`（读 `a0..a2`）。宽度是**那一格载荷自己的事实**：一对寄存器说不完的
//! 才标 `ret3`（今天只有 `PieCall::Collect`），其余四十八格一个字不改。
//!
//! 通用性：`slot/pack/unpack` 与 `Ret` 只依赖 `Wire`（不绑 env 错误/汇编），sbi 等
//! S-mode 调用封装未来可复用同一 derive；`call` 则绑定 env 的 `EnvResult`/汇编入口。
//!
//! 本文件是该宏的全部：解析（[`ret_type`] / [`ret_wide_type`] / [`class`]）、变体那一格
//! （[`Variant`]）与展开（[`expand`]）——**与另外两个宏一行都不共享**。

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Attribute, Data, DeriveInput, Fields, Ident, Lit, Type, parse2};

/// 展开见文件头。
pub fn expand(input: TokenStream2) -> TokenStream2 {
    let ast = match parse2::<DeriveInput>(input) {
        Ok(ast) => ast,
        Err(e) => return e.to_compile_error(),
    };
    let name = &ast.ident;
    let class = match class(&ast.attrs) {
        Ok(c) => c,
        Err(e) => return e.to_compile_error(),
    };
    let vols = match variants(&ast) {
        Ok(v) => v,
        Err(e) => return e.to_compile_error(),
    };
    let ret_name = Ident::new(&format!("{}Ret", name), proc_macro2::Span::call_site());

    // 这一枚枚举里有没有宽返回的那一格——决定 `call()` 绑几口寄存器。
    let any_wide = vols.iter().any(|v| v.wide);
    // 第三口绑不绑名字：只有宽那一格用得上它，其余枚举绑成 `_v2`（不绑名字就不会有
    // "未使用的变量"那一 warn，而 `a2` 照样按 ABI 读回——线宽不因没人读而改变）。
    let v2_bind: TokenStream2 = if any_wide {
        quote! { v2 }
    } else {
        quote! { _v2 }
    };

    let slot_arms: Vec<_> = vols
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let form = v.shape(None);
            quote! { #form => ( #class << 32 ) | #i }
        })
        .collect();

    let pack_arms: Vec<_> = vols
        .iter()
        .map(|v| {
            let binds = v.binds();
            let form = v.shape(Some(&binds));
            if v.is_unit() {
                quote! { #form => [0usize; 6] }
            } else {
                let typs = v.types();
                let packs = binds
                    .iter()
                    .zip(typs.iter())
                    .map(|(b, t)| {
                        quote! {
                            <#t as crate::wire::Wire>::pack(&#b, &mut s, &mut i);
                        }
                    })
                    .collect::<Vec<_>>();
                quote! {
                    #form => {
                        let mut s = [0usize; 6];
                        let mut i = 0usize;
                        #(#packs)*
                        s
                    }
                }
            }
        })
        .collect();

    let unpack_arms: Vec<_> = vols
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let binds = v.binds();
            let form = v.shape(Some(&binds));
            if v.is_unit() {
                quote! { #i => Ok(#form) }
            } else {
                let typs = v.types();
                let unpacks = binds
                    .iter()
                    .zip(typs.iter())
                    .map(|(b, t)| {
                        quote! {
                            let #b: #t = <#t as crate::wire::Wire>::unpack(&regs, &mut i)?;
                        }
                    })
                    .collect::<Vec<_>>();
                quote! {
                    #i => {
                        let mut i = 0usize;
                        #(#unpacks)*
                        Ok(#form)
                    }
                }
            }
        })
        .collect();

    let ret_variants: Vec<_> = vols
        .iter()
        .map(|v| {
            let (id, ty) = (&v.ident, &v.ret);
            quote! {
                #id(#ty)
            }
        })
        .collect();

    let distill_arms: Vec<_> = vols
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let (id, ty) = (&v.ident, &v.ret);
            if v.wide {
                quote! {
                    #i => #ret_name::#id(<#ty as crate::wire::FromTriple>::from_triple(v0, v1, v2))
                }
            } else {
                quote! {
                    #i => #ret_name::#id(<#ty as crate::wire::FromPair>::from_pair(v0, v1))
                }
            }
        })
        .collect();

    let call_arms: Vec<_> = vols
        .iter()
        .map(|v| {
            let binds = v.binds();
            // 匹配与造值**同形**：同一份 `shape`，一处都不重写（从前这里写了两遍）。
            let form = v.shape(Some(&binds));
            quote! {
                #form => {
                    let this = #form;
                    let slot = Self::slot(&this);
                    let args = Self::pack(&this);
                    (slot, args)
                }
            }
        })
        .collect();

    let expanded = quote! {
        impl #name {
            #[inline]
            pub const fn slot(&self) -> usize {
                match self {
                    #(#slot_arms),*
                }
            }

            #[inline]
            pub fn pack(&self) -> [usize; 6] {
                match self {
                    #(#pack_arms),*
                }
            }

            /// 由调用号 + 寄存器组解码回本枚举（校验式，非法位 → Err）。
            #[inline]
            pub fn from_wire(slot: usize, regs: &[usize; 6]) -> Result<Self, crate::wire::Decode> {
                let index = (slot & 0xFFFF_FFFF) as usize;
                match index {
                    #(#unpack_arms),*,
                    _ => Err(crate::wire::Decode::BadSlot),
                }
            }
        }

        /// 本域调用的结果枚举（R3：每个原语一个锁定的返回类型）。
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum #ret_name {
            #(#ret_variants),*
        }

        impl #name {
            /// 触发并判译：a0 负 → `Err(EnvError)`，非负 → 蒸馏成域 Ret。
            #[inline]
            pub fn call(self) -> crate::ecall::EnvResult<#ret_name> {
                let (slot, args) = match self {
                    #(#call_arms),*
                };
                let (v0, v1, #v2_bind) = unsafe { crate::ecall::trap(slot, args) };
                if (v0 as isize) < 0 {
                    Err(crate::ecall::make_err(crate::ecall::EnvError::from_raw(
                        v0 as isize,
                    )))
                } else {
                    Ok(match slot & 0xFFFF_FFFF {
                        #(#distill_arms),*,
                        _ => unreachable!(),
                    })
                }
            }
        }
    };

    expanded
}

/// 一枚变体：**名字、字段、返回载荷、返回宽度**四件事绑在一起。
///
/// 从前它们是四个并行数组（`vs` / `flds` / `rets` / `wides`），靠下标对齐——于是每一处用它的
/// 地方都要写一遍 `&vs[i]` / `&flds[i]` / `rets[i].clone()`，下标写错一格**编得过**。
struct Variant {
    ident: Ident,
    fields: Fields,
    /// 这一格的返回载荷：`#[ret(T)]` 读 `a0`/`a1`，`#[ret3(T)]` 读 `a0..a2`。
    ///
    /// **不是 `Option`**：`Ret` 枚举一个变体一格载荷 ⇒ "这一格没标返回"是那张表的错，
    /// 在 [`variants`] 里当场报、不往后传 `None`。
    ret: Type,
    /// 标的是 `#[ret3(T)]` ⇒ 蒸馏走 `FromTriple` 而不是 `FromPair`。
    wide: bool,
}

impl Variant {
    /// 字段类型，按声明顺序。
    fn types(&self) -> Vec<Type> {
        match &self.fields {
            Fields::Named(named) => named.named.iter().map(|f| f.ty.clone()).collect(),
            Fields::Unnamed(unnamed) => unnamed.unnamed.iter().map(|f| f.ty.clone()).collect(),
            Fields::Unit => Vec::new(),
        }
    }

    /// 字段名，按声明顺序（无名那一族现造 `f0`/`f1`…——名字只在展开体里用，不落到用户面）。
    fn binds(&self) -> Vec<Ident> {
        match &self.fields {
            Fields::Named(named) => named
                .named
                .iter()
                .map(|f| f.ident.clone().unwrap())
                .collect(),
            Fields::Unnamed(unnamed) => (0..unnamed.unnamed.len())
                .map(|i| Ident::new(&format!("f{i}"), proc_macro2::Span::call_site()))
                .collect(),
            Fields::Unit => Vec::new(),
        }
    }

    fn is_unit(&self) -> bool {
        matches!(self.fields, Fields::Unit)
    }

    /// **变体那一格的唯一一处拼法**：匹配与造值**逐字同形**，故只有这一个函数。
    ///
    /// `binds = None` ⇒ 字段位写 `..`：只匹配、不绑名字（不绑就不会长出"未使用的变量"）。
    /// 从前这件事有三个出处：`pat_for` 与 `expr_for` 是一对复制品，`slot(&self)` 里是第三份。
    fn shape(&self, binds: Option<&[Ident]>) -> TokenStream2 {
        let v = &self.ident;
        match (&self.fields, binds) {
            (Fields::Unit, _) => quote! { Self::#v },
            (Fields::Unnamed(_), Some(b)) => quote! { Self::#v(#(#b),*) },
            (Fields::Unnamed(_), None) => quote! { Self::#v(..) },
            (Fields::Named(_), Some(b)) => quote! { Self::#v { #(#b),* } },
            (Fields::Named(_), None) => quote! { Self::#v { .. } },
        }
    }
}

/// 解析 `#[ret(T)]` 属性里的返回类型。
fn ret_type(attrs: &[Attribute]) -> syn::Result<Option<Type>> {
    for attr in attrs {
        if attr.path().is_ident("ret") {
            let ty = attr.parse_args::<Type>()?;
            return Ok(Some(ty));
        }
    }
    Ok(None)
}

/// 解析 `#[ret3(T)]` 属性里的返回类型（**宽返回那一格**：读 `a0..a2`）。
///
/// 与 [`ret_type`] 分成两个函数、而不是一个函数认两种拼法：**一格载荷最多有一个返回
/// 宽度**，两处各自只认自己那个属性名，撞上了（同一 variant 两个都标）由
/// [`variants`] 当场报错，而不是让后一个静默覆盖前一个。
fn ret_wide_type(attrs: &[Attribute]) -> syn::Result<Option<Type>> {
    for attr in attrs {
        if attr.path().is_ident("ret3") {
            let ty = attr.parse_args::<Type>()?;
            return Ok(Some(ty));
        }
    }
    Ok(None)
}

/// 解析 `#[call(class = N)]` 属性里的 class。
fn class(attrs: &[Attribute]) -> syn::Result<usize> {
    for attr in attrs {
        if attr.path().is_ident("call") {
            let mut value = None;
            let _ = attr.parse_nested_meta(|nested| {
                if nested.path.is_ident("class") {
                    let lit: Lit = nested.value()?.parse()?;
                    if let Lit::Int(i) = lit {
                        value = Some(i.base10_parse::<usize>()?);
                    }
                }
                Ok(())
            });
            if let Some(v) = value {
                return Ok(v);
            }
            return Err(syn::Error::new_spanned(attr, "expected #[call(class = N)]"));
        }
    }
    Err(syn::Error::new_spanned(
        &attrs[0],
        "expected #[call(class = N)] on the enum",
    ))
}

/// 从 `DeriveInput` 提取变体列表；三条当场报错，都落在**那一格的名字**上：不是枚举、
/// 两种返回宽度都标了、**这一格没标返回**。
fn variants(ast: &DeriveInput) -> syn::Result<Vec<Variant>> {
    let data = match &ast.data {
        Data::Enum(e) => e,
        _ => return Err(syn::Error::new_spanned(ast, "Envcall only supports enums")),
    };
    data.variants
        .iter()
        .map(|v| {
            let (narrow, wide) = (ret_type(&v.attrs)?, ret_wide_type(&v.attrs)?);
            let (ret, is_wide) = match (narrow, wide) {
                (Some(_), Some(_)) => {
                    return Err(syn::Error::new_spanned(
                        &v.ident,
                        "改一格载荷的返回宽度：`#[ret(T)]` 与 `#[ret3(T)]` 只能标一个",
                    ));
                }
                // **照实记（这一条从前不在这里报）**：`variants` 曾把"没标返回"折成 `None`
                // 放过去，到 `rets[i].clone().expect("every variant must have #[ret(T)]")`
                // 才炸——用户拿到的是 `error: proc macro panicked`（消息埋在 `help:` 里、
                // **不指那一格**）。`fid.rs` 那 49 格里漏标一格，找它只能靠人眼。返回载荷是
                // 这个宏的**不变式**（`Ret` 枚举一个变体一格载荷），故它在这里就该断。
                (None, None) => {
                    return Err(syn::Error::new_spanned(
                        &v.ident,
                        "这一格没有返回载荷：标 `#[ret(T)]`（读 `a0`/`a1`）或 `#[ret3(T)]`（读 `a0..a2`）",
                    ));
                }
                (Some(t), None) => (t, false),
                (None, Some(t)) => (t, true),
            };
            Ok(Variant {
                ident: v.ident.clone(),
                fields: v.fields.clone(),
                ret,
                wide: is_wide,
            })
        })
        .collect()
}
