//! Derive macros for [`anarchy`](https://github.com/DaylightNebula/anarchy).
//!
//! The generated code refers to `::anarchy`, so use these through the
//! re-exports in `anarchy` rather than depending on this crate directly.

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{Attribute, Data, DeriveInput, Fields, parse_macro_input, parse_quote};

/// Implements `AsAny` by returning `self` as `dyn Any`.
#[proc_macro_derive(AsAny)]
pub fn derive_as_any(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    as_any_impl(&input).into()
}

/// Implements `Component`, `ComponentMeta` and `AsAny`. The type must also implement `Debug` and `Send`.
///
/// Also implements serde's `Serialize` and `Deserialize`, and `Component::serialize`
/// on top of them. `#[serde(...)]` attributes work as with serde's own derives.
/// Non generic types also implement `SerializedComponent`, registered with typetag under
/// `"<crate>::<Type>"`, so they can be deserialized into a `DynComponent`.
/// Opt out with `#[component(no_serialize)]`, leaving `Component::serialize` returning `None`.
#[proc_macro_derive(Component, attributes(component, serde))]
pub fn derive_component(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;
    let as_any = as_any_impl(&input);

    let no_serialize = match no_serialize(&input) {
        Ok(no_serialize) => no_serialize,
        Err(err) => return err.to_compile_error().into(),
    };
    // `Component::serialize` hands out `self` as `Serialize`, which needs every type parameter to be.
    let generics = if no_serialize { input.generics.clone() } else { bound_type_params(&input.generics, parse_quote!(::anarchy::serde::Serialize)) };
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    let (serialize, serde_impls) = match no_serialize {
        true => (quote! {}, quote! {}),
        false => match serde_impls(&input) {
            Ok(serde_impls) => {
                // typetag registers concrete types only, so generic components can't be tagged.
                let tagged = generics.params.is_empty();
                let as_serialized = tagged.then(|| quote! {
                    fn as_serialized(&self) -> Option<&dyn ::anarchy::SerializedComponent> { Some(self) }
                });
                let tagged_impl = tagged.then(|| {
                    let tag = format!("{}::{}", std::env::var("CARGO_CRATE_NAME").unwrap_or_default(), name);
                    quote! {
                        const _: () = {
                            use ::anarchy::typetag;
                            #[typetag::serde(name = #tag)]
                            impl ::anarchy::SerializedComponent for #name {}
                        };
                    }
                });
                (quote! {
                    fn serialize(&self) -> Option<(&'static str, &dyn ::anarchy::erased_serde::Serialize)> {
                        Some((<Self as ::anarchy::ComponentMeta>::name(), self))
                    }
                    #as_serialized
                }, quote! { #serde_impls #tagged_impl })
            }
            Err(err) => return err.to_compile_error().into(),
        },
    };

    quote! {
        impl #impl_generics ::anarchy::Component for #name #ty_generics #where_clause {
            #serialize
        }
        impl #impl_generics ::anarchy::ComponentMeta for #name #ty_generics #where_clause {}
        #as_any
        #serde_impls
    }.into()
}

/// Implements `Resource`, `ResourceMeta` and `AsAny`. The type must also implement `Debug` and `Send`.
#[proc_macro_derive(Resource)]
pub fn derive_resource(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
    let as_any = as_any_impl(&input);

    quote! {
        impl #impl_generics ::anarchy::Resource for #name #ty_generics #where_clause {}
        impl #impl_generics ::anarchy::ResourceMeta for #name #ty_generics #where_clause {}
        #as_any
    }.into()
}

/// Whether a `#[component(no_serialize)]` attribute is present.
fn no_serialize(input: &DeriveInput) -> syn::Result<bool> {
    let mut no_serialize = false;
    for attr in input.attrs.iter().filter(|attr| attr.path().is_ident("component")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("no_serialize") {
                no_serialize = true;
                Ok(())
            } else {
                Err(meta.error("unknown component attribute"))
            }
        })?;
    }
    Ok(no_serialize)
}

/// Implements `Serialize` and `Deserialize` through serde's own derives, applied with
/// `#[serde(remote = ...)]` to a copy of the type that keeps only its serde attributes.
fn serde_impls(input: &DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let name = &input.ident;
    let remote_name = format_ident!("__{}Serde", name);

    let mut remote = input.clone();
    remote.ident = remote_name.clone();
    remote.vis = syn::Visibility::Inherited;
    retain_serde_attrs(&mut remote.attrs);
    match &mut remote.data {
        Data::Struct(data) => retain_serde_field_attrs(&mut data.fields),
        Data::Enum(data) => for variant in &mut data.variants {
            retain_serde_attrs(&mut variant.attrs);
            retain_serde_field_attrs(&mut variant.fields);
        },
        Data::Union(_) => return Err(syn::Error::new_spanned(
            name,
            "unions can't be serialized, add #[component(no_serialize)]",
        )),
    }
    let remote_path = name.to_string();

    // Bound every type parameter by the trait being implemented, as serde does by default.
    let ser_generics = bound_type_params(&input.generics, parse_quote!(::anarchy::serde::Serialize));
    let mut de_generics = bound_type_params(&input.generics, parse_quote!(::anarchy::serde::Deserialize<'de>));
    de_generics.params.insert(0, parse_quote!('de));

    let (_, ty_generics, _) = input.generics.split_for_impl();
    let (ser_impl_generics, _, ser_where_clause) = ser_generics.split_for_impl();
    let (de_impl_generics, _, de_where_clause) = de_generics.split_for_impl();

    Ok(quote! {
        const _: () = {
            #[derive(::anarchy::serde::Serialize, ::anarchy::serde::Deserialize)]
            #[serde(crate = "::anarchy::serde", remote = #remote_path)]
            #[allow(dead_code)]
            #remote

            impl #ser_impl_generics ::anarchy::serde::Serialize for #name #ty_generics #ser_where_clause {
                fn serialize<__S: ::anarchy::serde::Serializer>(&self, serializer: __S) -> Result<__S::Ok, __S::Error> {
                    #remote_name::serialize(self, serializer)
                }
            }

            impl #de_impl_generics ::anarchy::serde::Deserialize<'de> for #name #ty_generics #de_where_clause {
                fn deserialize<__D: ::anarchy::serde::Deserializer<'de>>(deserializer: __D) -> Result<Self, __D::Error> {
                    #remote_name::deserialize(deserializer)
                }
            }
        };
    })
}

fn bound_type_params(generics: &syn::Generics, bound: syn::TypeParamBound) -> syn::Generics {
    let mut generics = generics.clone();
    for param in generics.type_params_mut() {
        param.bounds.push(bound.clone());
    }
    generics
}

fn retain_serde_attrs(attrs: &mut Vec<Attribute>) {
    attrs.retain(|attr| attr.path().is_ident("serde"));
}

fn retain_serde_field_attrs(fields: &mut Fields) {
    for field in fields.iter_mut() {
        retain_serde_attrs(&mut field.attrs);
        field.vis = syn::Visibility::Inherited;
    }
}

fn as_any_impl(input: &DeriveInput) -> proc_macro2::TokenStream {
    let name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    quote! {
        impl #impl_generics ::anarchy::AsAny for #name #ty_generics #where_clause {
            fn as_any(&self) -> &dyn ::std::any::Any { self }
            fn as_any_mut(&mut self) -> &mut dyn ::std::any::Any { self }
        }
    }
}
