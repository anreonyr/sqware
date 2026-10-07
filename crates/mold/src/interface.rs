use std::collections::BTreeSet;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Attribute, Fields, Ident, Item, ItemEnum, ItemMod, LitInt, LitStr, Meta, Token};
use syn::parse::Parser;
use syn::punctuated::Punctuated;

pub fn expand(attr: TokenStream, item: TokenStream) -> TokenStream {
    generate(attr, item).unwrap_or_else(|error| error.to_compile_error())
}

struct Symbol {
    variant: Ident,
    constant: Ident,
    key: LitStr,
    name: LitStr,
    code: u8,
}
struct Operation {
    variant: Ident,
    frame: Ident,
    grant: Ident,
    code: u8,
}

fn take(attrs: &mut Vec<Attribute>, name: &str) -> syn::Result<Option<Attribute>> {
    let mut found = None;
    for at in (0..attrs.len()).rev() {
        if attrs[at].path().is_ident(name) {
            if found.is_some() { return Err(syn::Error::new_spanned(&attrs[at], "duplicate attribute")); }
            found = Some(attrs.remove(at));
        }
    }
    Ok(found)
}

fn symbols(mut item: ItemEnum, kind: &str, id: &LitStr) -> syn::Result<(ItemEnum, Vec<Symbol>)> {
    if !item.generics.params.is_empty() || item.variants.is_empty() {
        return Err(syn::Error::new_spanned(item, "interface symbols must be a nonempty, nongeneric enum"));
    }
    let mut result = Vec::new();
    let mut keys = BTreeSet::new();
    let mut codes = BTreeSet::new();
    for variant in &mut item.variants {
        if !matches!(variant.fields, Fields::Unit) || variant.discriminant.is_some() {
            return Err(syn::Error::new_spanned(variant, "declare symbol codes in the attribute"));
        }
        let attr = take(&mut variant.attrs, kind)?
            .ok_or_else(|| syn::Error::new_spanned(&variant.ident, "missing symbol attribute"))?;
        let mut key = None;
        let mut legacy = None;
        let mut code = None;
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("key") && key.is_none() {
                key = Some(meta.value()?.parse::<LitStr>()?);
            } else if meta.path.is_ident("legacy") && legacy.is_none() {
                legacy = Some(meta.value()?.parse::<LitStr>()?);
            } else if kind == "grant" && meta.path.is_ident("code") && code.is_none() {
                code = Some(meta.value()?.parse::<LitInt>()?.base10_parse::<u8>()?);
            } else { return Err(meta.error("unknown or repeated symbol option")); }
            Ok(())
        })?;
        let key = key.ok_or_else(|| syn::Error::new_spanned(&attr, "a stable key is required"))?;
        if key.value().is_empty() || !keys.insert(key.value()) {
            return Err(syn::Error::new_spanned(key, "empty or duplicate stable key"));
        }
        let code = if kind == "grant" {
            let code = code.ok_or_else(|| syn::Error::new_spanned(&attr, "an explicit grant code is required"))?;
            if code == 0 || !codes.insert(code) {
                return Err(syn::Error::new_spanned(&attr, "zero or duplicate grant code"));
            }
            code
        } else { 0 };
        let name = legacy.unwrap_or_else(|| LitStr::new(
            &format!("{}/{}/{}", id.value(), kind, key.value()), key.span(),
        ));
        if name.value().is_empty() { return Err(syn::Error::new_spanned(name, "empty mark name")); }
        result.push(Symbol {
            variant: variant.ident.clone(),
            constant: format_ident!("{}", variant.ident.to_string().to_uppercase()),
            key, name, code,
        });
    }
    Ok((item, result))
}

fn generate(attr: TokenStream, input: TokenStream) -> syn::Result<TokenStream> {
    let options = Punctuated::<Meta, Token![,]>::parse_terminated.parse2(attr)?;
    let mut id = None;
    for option in options {
        if let Meta::NameValue(value) = &option {
            if value.path.is_ident("id") && id.is_none() {
                let expression = &value.value;
                id = Some(syn::parse2::<LitStr>(quote!(#expression))?);
                continue;
            }
        }
        return Err(syn::Error::new_spanned(option, "expected one stable interface id"));
    }
    let id = id.ok_or_else(|| syn::Error::new(proc_macro2::Span::call_site(), "missing interface id"))?;
    if id.value().is_empty() { return Err(syn::Error::new_spanned(id, "empty interface id")); }
    let mut module: ItemMod = syn::parse2(input)?;
    let (_, items) = module.content.take()
        .ok_or_else(|| syn::Error::new_spanned(&module, "interface requires an inline module"))?;
    let mut channel = None;
    let mut grant = None;
    let mut request = None;
    let mut others = Vec::new();
    let mut replies = Vec::new();
    for mut item in items {
        match &mut item {
            Item::Enum(e) if e.attrs.iter().any(|a| a.path().is_ident("channels")) => {
                take(&mut e.attrs, "channels")?;
                if channel.is_some() { return Err(syn::Error::new_spanned(e, "duplicate channels enum")); }
                channel = Some(symbols(e.clone(), "channel", &id)?);
            }
            Item::Enum(e) if e.attrs.iter().any(|a| a.path().is_ident("grants")) => {
                take(&mut e.attrs, "grants")?;
                if grant.is_some() { return Err(syn::Error::new_spanned(e, "duplicate grants enum")); }
                grant = Some(symbols(e.clone(), "grant", &id)?);
            }
            Item::Enum(e) if e.attrs.iter().any(|a| a.path().is_ident("requests")) => {
                take(&mut e.attrs, "requests")?;
                if request.is_some() { return Err(syn::Error::new_spanned(e, "duplicate requests enum")); }
                request = Some(e.clone());
            }
            Item::Struct(s) if s.attrs.iter().any(|a| a.path().is_ident("reply")) => {
                take(&mut s.attrs, "reply")?;
                let name = &s.ident;
                let (ig, tg, wc) = s.generics.split_for_impl();
                replies.push(quote! {
                    #[derive(env::Frame)] #s
                    impl #ig ::wire::Message for #name #tg #wc {
                        type In = Self;
                        type Buf = [u8; Self::LEN];
                        const EMPTY: Self::Buf = [0; Self::LEN];
                        fn store(&self, out: &mut [u8]) -> Option<usize> {
                            <Self as env::wire::Span>::store_at(self, out, 0)
                        }
                        fn fetch(bytes: &[u8]) -> Option<Self> {
                            let (value, end) = <Self as env::wire::Span>::fetch_at(bytes, 0)?;
                            (end == bytes.len()).then_some(value)
                        }
                    }
                });
            }
            _ => others.push(item),
        }
    }
    let (channel, channels) = channel.ok_or_else(|| syn::Error::new_spanned(&module, "missing channels enum"))?;
    let (grant, grants) = grant.ok_or_else(|| syn::Error::new_spanned(&module, "missing grants enum"))?;
    let request = request.ok_or_else(|| syn::Error::new_spanned(&module, "missing requests enum"))?;
    if !request.generics.params.is_empty() || request.variants.is_empty() || replies.is_empty() {
        return Err(syn::Error::new_spanned(request, "requests must be nongeneric and nonempty, with a reply"));
    }
    let wire = &request.ident;
    let vis = &request.vis;
    let wattrs = &request.attrs;
    let mut frames = Vec::new();
    let mut operations = Vec::new();
    let mut codes = BTreeSet::new();
    let mut names = BTreeSet::new();
    for mut variant in request.variants {
        let attr = take(&mut variant.attrs, "operation")?
            .ok_or_else(|| syn::Error::new_spanned(&variant, "missing operation attribute"))?;
        let mut code = None;
        let mut selected = None;
        let mut frame = None;
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("code") && code.is_none() {
                code = Some(meta.value()?.parse::<LitInt>()?.base10_parse::<u8>()?);
            } else if meta.path.is_ident("grant") && selected.is_none() {
                selected = Some(meta.value()?.parse::<Ident>()?);
            } else if meta.path.is_ident("frame") && frame.is_none() {
                frame = Some(meta.value()?.parse::<Ident>()?);
            } else { return Err(meta.error("unknown or repeated operation option")); }
            Ok(())
        })?;
        let code = code.ok_or_else(|| syn::Error::new_spanned(&attr, "missing operation code"))?;
        if code == 0 || !codes.insert(code) {
            return Err(syn::Error::new_spanned(&attr, "zero or duplicate operation code"));
        }
        let selected = selected.ok_or_else(|| syn::Error::new_spanned(&attr, "missing grant"))?;
        if !grants.iter().any(|g| g.variant == selected) {
            return Err(syn::Error::new_spanned(selected, "unknown grant"));
        }
        let frame = frame.ok_or_else(|| syn::Error::new_spanned(&attr, "missing frame name"))?;
        if !names.insert(frame.to_string()) || variant.discriminant.is_some() {
            return Err(syn::Error::new_spanned(frame, "duplicate frame or unexpected discriminant"));
        }
        let Fields::Named(fields) = &variant.fields else {
            return Err(syn::Error::new_spanned(variant, "requests require named fields"));
        };
        let mut generated_fields = Vec::new();
        for field in &fields.named {
            let ident = field.ident.as_ref().unwrap();
            if ident == "op" { return Err(syn::Error::new_spanned(field, "op is generated from the operation code")); }
            let attrs = &field.attrs;
            let ty = &field.ty;
            generated_fields.push(quote!(#(#attrs)* pub #ident: #ty,));
        }
        let attrs = &variant.attrs;
        frames.push(quote! {
            #(#attrs)* #[derive(env::Frame)]
            #vis struct #frame { pub op: u8, #(#generated_fields)* }
        });
        operations.push(Operation { variant: variant.ident, frame, grant: selected, code });
    }
    let wvariants: Vec<_> = operations.iter().map(|o| &o.variant).collect();
    let ftypes: Vec<_> = operations.iter().map(|o| &o.frame).collect();
    let opcodes: Vec<_> = operations.iter().map(|o| o.code).collect();
    let opconsts: Vec<_> = operations.iter().map(|o| format_ident!("{}", o.variant.to_string().to_uppercase())).collect();
    let selected: Vec<_> = operations.iter().map(|o| &o.grant).collect();
    let gtype = &grant.ident;
    let gvariants: Vec<_> = grants.iter().map(|g| &g.variant).collect();
    let gkeys: Vec<_> = grants.iter().map(|g| &g.key).collect();
    let gnames: Vec<_> = grants.iter().map(|g| &g.name).collect();
    let gcodes: Vec<_> = grants.iter().map(|g| g.code).collect();
    let indices: Vec<_> = (0..grants.len()).collect();
    let n = grants.len();
    let ctype = &channel.ident;
    let cvariants: Vec<_> = channels.iter().map(|c| &c.variant).collect();
    let cconsts: Vec<_> = channels.iter().map(|c| &c.constant).collect();
    let cnames: Vec<_> = channels.iter().map(|c| &c.name).collect();
    let attrs = &module.attrs;
    let mvis = &module.vis;
    let mname = &module.ident;
    Ok(quote! {
        #(#attrs)* #mvis mod #mname {
            #(#others)* #(#replies)* #(#frames)*
            pub const INTERFACE_ID: &str = #id;
            #(pub const #opconsts: u8 = #opcodes;)*
            #(#wattrs)* #vis enum #wire { #(#wvariants(#ftypes),)* }
            impl #wire {
                pub const LEN: usize = {
                    let mut len = 0;
                    #(if #ftypes::LEN > len { len = #ftypes::LEN; })*
                    len
                };
                pub fn take(bytes: &[u8]) -> Option<Self> {
                    match *bytes.first()? {
                        #(#opcodes => {
                            let (value, end) = <#ftypes as env::wire::Span>::fetch_at(bytes, 0)?;
                            (end == bytes.len()).then_some(Self::#wvariants(value))
                        },)*
                        _ => None,
                    }
                }
                pub fn store(&self, out: &mut [u8]) -> Option<usize> {
                    match self {
                        #(Self::#wvariants(value) if value.op == #opcodes =>
                            <#ftypes as env::wire::Span>::store_at(value, out, 0),)*
                        _ => None,
                    }
                }
            }
            impl ::wire::Message for #wire {
                type In = Self;
                type Buf = [u8; Self::LEN];
                const EMPTY: Self::Buf = [0; Self::LEN];
                fn store(&self, out: &mut [u8]) -> Option<usize> {
                    #wire::store(self, out)
                }
                fn fetch(bytes: &[u8]) -> Option<Self> { #wire::take(bytes) }
            }
            #[derive(Clone, Copy, PartialEq, Eq, Debug)] #channel
            impl #ctype {
                pub const fn mark(self) -> env::Mark {
                    match self { #(Self::#cvariants => env::Mark::of(#cnames),)* }
                }
            }
            #(pub const #cconsts: env::Mark = #ctype::#cvariants.mark();)*
            pub const CHANNELS: &[::env::marks::Definition] = &[
                #(::env::marks::Definition { name: #cnames, mark: #cconsts },)*
            ];
            #[derive(Clone, Copy, PartialEq, Eq, Debug)] #grant
            impl #gtype {
                pub const COUNT: usize = #n;
                pub const ALL: [Self; #n] = [#(Self::#gvariants,)*];
                pub const fn at(self) -> u8 { match self { #(Self::#gvariants => #gcodes,)* } }
                pub const fn index(self) -> usize { match self { #(Self::#gvariants => #indices,)* } }
                pub const fn from_action(code: u8) -> Option<Self> {
                    match code { #(#gcodes => Some(Self::#gvariants),)* _ => None }
                }
                pub const fn name(self) -> &'static str { match self { #(Self::#gvariants => #gkeys,)* } }
                pub const fn mark(self) -> env::Mark { match self { #(Self::#gvariants => env::Mark::of(#gnames),)* } }
                pub const fn for_wire(value: &#wire) -> Self {
                    match value { #(#wire::#wvariants(_) => Self::#selected,)* }
                }
                pub const fn of_wire(value: &#wire) -> u8 { Self::for_wire(value).at() }
                pub const MARKS: [env::Mark; #n] = [#(Self::#gvariants.mark(),)*];
                pub const DECLARATIONS: [::env::marks::Definition; #n] = [
                    #(::env::marks::Definition { name: #gnames, mark: Self::#gvariants.mark() },)*
                ];
            }
            pub fn grant_of(mark: env::Mark) -> Option<#gtype> {
                #gtype::ALL.into_iter().find(|grant| grant.mark() == mark)
            }
            pub const REGISTRY: &[::env::marks::Definition] = &[
                #(::env::marks::Definition { name: #cnames, mark: #cconsts },)*
                #(::env::marks::Definition { name: #gnames, mark: #gtype::#gvariants.mark() },)*
            ];
            const _: () = {
                let mut i = 0;
                while i < REGISTRY.len() {
                    assert!(REGISTRY[i].mark.get() != env::Mark::NONE.get(), "reserved empty mark");
                    i += 1;
                }
                assert!(::env::marks::conflict(&[REGISTRY]).is_none(), "interface mark collision");
            };
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(grants: TokenStream, requests: TokenStream) -> syn::Result<TokenStream> {
        generate(quote!(id = "test.v1"), quote! {
            pub mod example {
                #[channels] enum Channel { #[channel(key = "reply")] Reply }
                #[grants] enum Grant { #grants }
                #[requests] enum Wire { #requests }
                #[reply] struct Reply { status: u8 }
            }
        })
    }
    fn grants() -> TokenStream { quote!(#[grant(code = 7, key = "create")] Create) }
    fn operation(code: u8, frame: Ident, grant: Ident) -> TokenStream {
        quote!(#[operation(code = #code, frame = #frame, grant = #grant)] Build { value: u64 })
    }
    #[test]
    fn rejects_duplicate_operation_codes() {
        let first = operation(1, format_ident!("Build"), format_ident!("Create"));
        let second = quote!(#[operation(code = 1, frame = Claim, grant = Create)] Claim { value: u64 });
        assert!(check(grants(), quote!(#first, #second)).unwrap_err().to_string().contains("duplicate operation"));
    }
    #[test]
    fn rejects_unknown_grants_and_zero_operation_codes() {
        for (code, grant) in [(1, "Missing"), (0, "Create")] {
            assert!(check(grants(), operation(code, format_ident!("Build"), format_ident!("{grant}"))).is_err());
        }
    }
    #[test]
    fn rejects_duplicate_grant_codes_and_stable_keys() {
        for second in [quote!(#[grant(code = 7, key = "other")] Other),
            quote!(#[grant(code = 8, key = "create")] Other)] {
            let first = grants();
            assert!(check(quote!(#first, #second), operation(1, format_ident!("Build"), format_ident!("Create"))).is_err());
        }
    }
    #[test]
    fn rejects_explicit_op_field() {
        assert!(check(grants(), quote!(#[operation(code = 1, frame = Ask, grant = Create)] Build { op: u8 })).is_err());
    }
    #[test]
    fn accepts_nonsequential_codes_and_shared_grants() {
        let input = quote! {
            #[operation(code = 42, frame = Claim, grant = Create)] Claim { value: u64 },
            #[operation(code = 1, frame = Ask, grant = Create)] Build { value: u64 }
        };
        let output = check(grants(), input).unwrap();
        syn::parse2::<ItemMod>(output).unwrap();
    }
}
