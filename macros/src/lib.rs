use proc_macro::TokenStream;
use quote::quote;
use syn::{DeriveInput, parse_macro_input};

/// Implements `AsAny` by returning `self` as `dyn Any`.
#[proc_macro_derive(AsAny)]
pub fn derive_as_any(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    as_any_impl(&input).into()
}

/// Implements `Component`, `ComponentMeta` and `AsAny`. The type must also implement `Debug` and `Send`.
#[proc_macro_derive(Component)]
pub fn derive_component(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
    let as_any = as_any_impl(&input);

    quote! {
        impl #impl_generics ::anarchy::Component for #name #ty_generics #where_clause {}
        impl #impl_generics ::anarchy::ComponentMeta for #name #ty_generics #where_clause {}
        #as_any
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
