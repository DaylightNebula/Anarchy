//! Derive macros for [`anarchy`](https://github.com/DaylightNebula/anarchy).
//!
//! The generated code refers to `::anarchy`, so use these through the
//! re-exports in `anarchy` rather than depending on this crate directly.

use proc_macro::TokenStream;
use quote::quote;
use syn::{DeriveInput, parse_macro_input};

mod reflect;
mod scene;

/// Builds a `Scene` from the scene format written as Rust, type checked at compile time.
/// Components don't need to be registered or reflected.
///
/// ```ignore
/// let scene = scene! {
///     @Gravity { y: -9.8 },             // a resource
///     #Player                           // an entity named Player
///     Transform { pos: Vec3 { x: 1.0 } } // `Transform::default()` with pos.x set
///     Health(100)                       // any `Type(args)` expression
///     Marker                            // `Marker::default()`
///     { Shape::Circle(0.5) }            // any expression in braces
///     [
///         #Sword :Weapon                // inherits the Weapon scene
///         Transform { pos: Vec3 { x: 0.5 } },
///     ],
/// };
/// ```
///
/// - `Type { field: value }` sets only the fields given on `Type::default()`.  A field
///   value written as `Type { .. }` sets that field's own fields in turn, and string
///   literals are converted with `Into`.
/// - Each patch is a `move` closure called every time the scene spawns, so captured
///   variables must be cloned inside it rather than moved out.
/// - Entity references like `Target(#Enemy)` are only read by text scenes.
#[proc_macro]
pub fn scene(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as scene::SceneInput);
    quote!(#input).into()
}

/// Implements `Reflect`, `Typed` and `FromValue` for a struct or enum.  Type parameters
/// must also implement all three.
///
/// - `///` doc comments on the type, its fields and variants are kept in its `TypeInfo`.
/// - `#[reflect(default)]` on the type builds missing fields from `Default` instead of
///   each field's fallback, so the type must implement `Default`.
/// - `#[reflect(skip)]` on a field hides it, and builds it with `Default`.
/// - `#[reflect(entity)]` on a field of entity ids (`EntityID`, `Option<EntityID>`,
///   `Vec<EntityID>`, ...) makes them entity references, so saved scenes name them.
/// - `#[reflect(hint(key = value, ...))]` on a field adds editor hints, each value is
///   converted with `Value::from`.
#[proc_macro_derive(Reflect, attributes(reflect))]
pub fn derive_reflect(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    reflect::derive(input).unwrap_or_else(syn::Error::into_compile_error).into()
}

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
