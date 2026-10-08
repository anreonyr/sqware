use proc_macro2::TokenStream;
use quote::quote;
use syn::parse::Parser;
use syn::punctuated::Punctuated;
use syn::{Expr, ItemStruct, Meta, Path, Token};

pub fn expand(attr: TokenStream, item: TokenStream) -> TokenStream {
    let options = match Punctuated::<Meta, Token![,]>::parse_terminated.parse2(attr) {
        Ok(options) => options,
        Err(error) => return error.to_compile_error(),
    };
    let mut request = None;
    let mut response = None;
    let mut mark = None;
    let mut back = None;
    for option in options {
        let Meta::NameValue(value) = option else {
            return syn::Error::new_spanned(
                option,
                "expected request, response, mark, or back path",
            )
            .to_compile_error();
        };
        let slot = if value.path.is_ident("request") {
            &mut request
        } else if value.path.is_ident("response") {
            &mut response
        } else if value.path.is_ident("mark") {
            &mut mark
        } else if value.path.is_ident("back") {
            &mut back
        } else {
            return syn::Error::new_spanned(value, "unknown contract option").to_compile_error();
        };
        if slot.is_some() {
            return syn::Error::new_spanned(value, "repeated contract option").to_compile_error();
        }
        let Expr::Path(expression) = value.value else {
            return syn::Error::new_spanned(value, "contract options require a path")
                .to_compile_error();
        };
        match syn::parse2::<Path>(quote!(#expression)) {
            Ok(path) => *slot = Some(path),
            Err(error) => return error.to_compile_error(),
        }
    }
    let Some(request) = request else {
        return syn::Error::new(proc_macro2::Span::call_site(), "missing request type")
            .to_compile_error();
    };
    let Some(response) = response else {
        return syn::Error::new(proc_macro2::Span::call_site(), "missing response type")
            .to_compile_error();
    };
    if mark.is_some() != back.is_some() {
        return syn::Error::new(
            proc_macro2::Span::call_site(),
            "mark and back must be declared together",
        )
        .to_compile_error();
    }
    let input = match syn::parse2::<ItemStruct>(item) {
        Ok(input) => input,
        Err(error) => return error.to_compile_error(),
    };
    let name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
    let binding = match (mark, back) {
        (Some(mark), Some(back)) => quote! {
            impl #impl_generics #name #ty_generics #where_clause {
                pub const BACK: ::env::Mark = #mark;
                pub fn back(
                    request: &<#request as ::wire::Message>::In,
                ) -> ::env::PieToken {
                    #back(request)
                }
            }
        },
        _ => quote!(),
    };
    quote! {
        #input
        impl #impl_generics ::wire::Contract for #name #ty_generics #where_clause {
            type Request = #request;
            type Response = #response;
        }
        #binding
    }
    .into()
}
