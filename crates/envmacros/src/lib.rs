//! envmacros — `#[derive(Envcall)]`：给环境调用枚举生成载荷 codec。
//!
//! 方案 3（typed payload）的 proc-macro 实现。输入一个带载荷的调用枚举，输出：
//!   * `slot(&self) -> usize`       —— 调用号（`#[call(class = N)]` 的 class << 32 | 判别号）
//!   * `pack(&self) -> [usize; 6]`  —— 字段按声明顺序 wire 化（`Wire::pack`）
//!   * `unpack(slot, &[usize; 6])`  —— 按 slot 取 variant，逐字段 `Wire::unpack`
//!   * `Ret` 枚举                   —— 每个标 `#[ret(T)]` 的 variant 一个载荷变体
//!   * `call(self) -> EnvResult<Ret>`—— 触发并判译（负值即错误）
//!
//! 通用性：`slot/pack/unpack` 与 `Ret` 只依赖 `Wire`（不绑 env 错误/汇编），sbi 等
//! S-mode 调用封装未来可复用同一 derive；`call` 则绑定 env 的 `EnvResult`/汇编入口。

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Attribute, Data, DeriveInput, Fields, Ident, Lit, Type, parse_macro_input};

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

/// 从 `DeriveInput` 提取 variant 列表：`[(name, fields, ret_type)]`。
fn variants(ast: &DeriveInput) -> syn::Result<Vec<(Ident, Fields, Option<Type>)>> {
    let data = match &ast.data {
        Data::Enum(e) => e,
        _ => return Err(syn::Error::new_spanned(ast, "Envcall only supports enums")),
    };
    let mut out = Vec::new();
    for v in data.variants.iter() {
        let ret = ret_type(&v.attrs)?;
        out.push((v.ident.clone(), v.fields.clone(), ret));
    }
    Ok(out)
}

fn field_types(f: &Fields) -> Vec<Type> {
    match f {
        Fields::Named(named) => named.named.iter().map(|f| f.ty.clone()).collect(),
        Fields::Unnamed(unnamed) => unnamed.unnamed.iter().map(|f| f.ty.clone()).collect(),
        Fields::Unit => Vec::new(),
    }
}

fn field_names_as_ident(f: &Fields) -> Vec<Ident> {
    match f {
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

fn is_unit(f: &Fields) -> bool {
    matches!(f, Fields::Unit)
}

fn is_unnamed(f: &Fields) -> bool {
    matches!(f, Fields::Unnamed(_))
}

fn pat_for(v: &Ident, f: &Fields, binds: &[Ident]) -> TokenStream2 {
    if is_unit(f) {
        quote! { Self::#v }
    } else if is_unnamed(f) {
        quote! { Self::#v(#(#binds),*) }
    } else {
        quote! { Self::#v { #(#binds),* } }
    }
}

fn expr_for(v: &Ident, f: &Fields, binds: &[Ident]) -> TokenStream2 {
    if is_unit(f) {
        quote! { Self::#v }
    } else if is_unnamed(f) {
        quote! { Self::#v(#(#binds),*) }
    } else {
        quote! { Self::#v { #(#binds),* } }
    }
}

#[proc_macro_derive(Envcall, attributes(call, ret))]
pub fn derive_envcall(input: TokenStream) -> TokenStream {
    let ast = parse_macro_input!(input as DeriveInput);
    let name = &ast.ident;
    let class = match class(&ast.attrs) {
        Ok(c) => c,
        Err(e) => return e.to_compile_error().into(),
    };
    let vols = match variants(&ast) {
        Ok(v) => v,
        Err(e) => return e.to_compile_error().into(),
    };
    let nkind = vols.len();
    let ret_name = Ident::new(&format!("{}Ret", name), proc_macro2::Span::call_site());

    // Split into parallel vectors.
    let mut vs: Vec<Ident> = Vec::new();
    let mut flds: Vec<Fields> = Vec::new();
    let mut rets: Vec<Option<Type>> = Vec::new();
    for (v, f, r) in vols {
        vs.push(v);
        flds.push(f);
        rets.push(r);
    }

    let slot_arms: Vec<_> = (0..nkind)
        .map(|i| {
            let v = &vs[i];
            let f = &flds[i];
            let pat = if is_unit(f) {
                quote! { Self::#v }
            } else if is_unnamed(f) {
                quote! { Self::#v(..) }
            } else {
                quote! { Self::#v { .. } }
            };
            quote! { #pat => ( #class << 32 ) | #i }
        })
        .collect();

    let pack_arms: Vec<_> = (0..nkind)
        .map(|i| {
            let v = &vs[i];
            let f = &flds[i];
            let typs = field_types(f);
            let binds = field_names_as_ident(f);
            let pat = pat_for(v, f, &binds);
            if is_unit(f) {
                quote! { #pat => [0usize; 6] }
            } else {
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
                    #pat => {
                        let mut s = [0usize; 6];
                        let mut i = 0usize;
                        #(#packs)*
                        s
                    }
                }
            }
        })
        .collect();

    let unpack_arms: Vec<_> = (0..nkind)
        .map(|i| {
            let v = &vs[i];
            let f = &flds[i];
            let typs = field_types(f);
            let binds = field_names_as_ident(f);
            if is_unit(f) {
                quote! { #i => Ok(Self::#v) }
            } else {
                let unpacks = binds
                    .iter()
                    .zip(typs.iter())
                    .map(|(b, t)| {
                        quote! {
                            let #b: #t = <#t as crate::wire::Wire>::unpack(&regs, &mut i)?;
                        }
                    })
                    .collect::<Vec<_>>();
                let build = pat_for(v, f, &binds);
                quote! {
                    #i => {
                        let mut i = 0usize;
                        #(#unpacks)*
                        Ok(#build)
                    }
                }
            }
        })
        .collect();

    let ret_variants: Vec<_> = (0..nkind)
        .map(|i| {
            let v = &vs[i];
            let ty = rets[i].clone().expect("every variant must have #[ret(T)]");
            quote! {
                #v(#ty)
            }
        })
        .collect();

    let distill_arms: Vec<_> = (0..nkind)
        .map(|i| {
            let v = &vs[i];
            let ty = rets[i].clone().expect("ret type");
            quote! {
                #i => #ret_name::#v(<#ty as crate::wire::FromPair>::from_pair(v0, v1))
            }
        })
        .collect();

    let call_arms: Vec<_> = (0..nkind)
        .map(|i| {
            let v = &vs[i];
            let f = &flds[i];
            let binds = field_names_as_ident(f);
            let pat = pat_for(v, f, &binds);
            let expr = expr_for(v, f, &binds);
            quote! {
                #pat => {
                    let this = #expr;
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
                let (v0, v1) = unsafe { crate::ecall::warpper(slot, args) };
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

    expanded.into()
}
