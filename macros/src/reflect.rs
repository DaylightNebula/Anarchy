//! `#[derive(Reflect)]`, which implements `Reflect`, `Typed` and `FromValue`.

use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use syn::{Attribute, Data, DeriveInput, Expr, Fields, Ident, Index, LitStr, Member, Type, parse_quote};

/// A reflected field, skipped fields are not included.
struct Field {
    /// How to reach the field, a name or a position.
    member: Member,
    /// The field's name in `TypeInfo`, its position for tuple fields.
    name: String,
    ty: Type,
    docs: String,
    hints: Vec<(Ident, Expr)>
}

/// The fields of a struct or variant, reflected or not.
struct FieldSet {
    style: Style,
    /// Every field, `None` where it is skipped.
    all: Vec<Option<Field>>,
    /// How to reach every field, skipped or not.
    members: Vec<Member>
}

#[derive(Clone, Copy, PartialEq)]
enum Style { Unit, Tuple, Named }

impl FieldSet {
    fn parse(fields: &Fields) -> syn::Result<Self> {
        let style = match fields { Fields::Unit => Style::Unit, Fields::Unnamed(_) => Style::Tuple, Fields::Named(_) => Style::Named };
        let mut all = vec![];
        let mut members = vec![];
        for (i, field) in fields.iter().enumerate() {
            let (member, name) = match &field.ident {
                Some(ident) => (Member::Named(ident.clone()), ident.to_string()),
                None => (Member::Unnamed(Index::from(i)), i.to_string())
            };
            members.push(member.clone());
            let mut skip = false;
            let mut hints = vec![];
            for attr in field.attrs.iter().filter(|attr| attr.path().is_ident("reflect")) {
                attr.parse_nested_meta(|meta| {
                    if meta.path.is_ident("skip") {
                        skip = true;
                        Ok(())
                    } else if meta.path.is_ident("entity") {
                        hints.push((format_ident!("entity"), parse_quote!(true)));
                        Ok(())
                    } else if meta.path.is_ident("hint") {
                        meta.parse_nested_meta(|hint| {
                            let key = hint.path.get_ident().cloned().ok_or_else(|| hint.error("expected a hint name"))?;
                            hints.push((key, hint.value()?.parse()?));
                            Ok(())
                        })
                    } else {
                        Err(meta.error("unknown field attribute, expected `skip`, `entity` or `hint(..)`"))
                    }
                })?;
            }
            all.push((!skip).then(|| Field { member, name, ty: field.ty.clone(), docs: docs(&field.attrs), hints }));
        }
        Ok(Self { style, all, members })
    }

    fn reflected(&self) -> impl Iterator<Item = &Field> {
        self.all.iter().flatten()
    }

    /// `::anarchy::VariantKind` describing these fields.
    fn kind(&self) -> TokenStream {
        let infos = self.reflected().map(|field| {
            let Field { name, ty, docs, hints, .. } = field;
            let hints = hints.iter().map(|(key, value)| {
                let key = key.to_string();
                quote! { (#key, ::anarchy::Value::from(#value)) }
            });
            quote! { ::anarchy::FieldInfo { name: #name, ty: <#ty as ::anarchy::Typed>::type_info, docs: #docs, hints: vec![#(#hints),*] } }
        });
        match self.style {
            Style::Unit => quote! { ::anarchy::VariantKind::Unit },
            Style::Tuple => quote! { ::anarchy::VariantKind::Tuple(vec![#(#infos),*]) },
            Style::Named => quote! { ::anarchy::VariantKind::Struct(vec![#(#infos),*]) }
        }
    }

    /// Builds these fields from `value` into `path`, a struct or variant path.
    fn construct(&self, path: TokenStream) -> TokenStream {
        let names = self.reflected().map(|field| &field.name);
        let values = self.all.iter().scan(0usize, |index, field| Some(match field {
            Some(Field { name, ty, .. }) => {
                let i = *index;
                *index += 1;
                quote! { fields.take::<#ty>(#name, #i)? }
            }
            None => quote! { ::std::default::Default::default() }
        }));
        let body = match self.style {
            Style::Unit => quote! {},
            Style::Tuple => quote! { (#(#values),*) },
            Style::Named => {
                let members = &self.members;
                quote! { { #(#members: #values),* } }
            }
        };
        quote! {{
            let fields = ::anarchy::FieldValues::new(value, &[#(#names),*])?;
            let _ = &fields;
            Ok(#path #body)
        }}
    }

    /// A pattern binding the reflected fields as `__f0`, `__f1`, ..., and the bindings.
    fn pattern(&self, path: TokenStream) -> (TokenStream, Vec<Ident>) {
        let mut bindings = vec![];
        let pattern = match self.style {
            Style::Unit => path,
            Style::Tuple => {
                let parts = self.all.iter().map(|field| match field {
                    Some(_) => {
                        let binding = format_ident!("__f{}", bindings.len());
                        bindings.push(binding.clone());
                        quote! { #binding }
                    }
                    None => quote! { _ }
                }).collect::<Vec<_>>();
                quote! { #path(#(#parts),*) }
            }
            Style::Named => {
                let parts = self.reflected().map(|field| {
                    let binding = format_ident!("__f{}", bindings.len());
                    bindings.push(binding.clone());
                    let member = &field.member;
                    quote! { #member: #binding }
                }).collect::<Vec<_>>();
                quote! { #path { #(#parts,)* .. } }
            }
        };
        (pattern, bindings)
    }
}

/// The doc comment in `attrs`, with the leading space of each line removed.
fn docs(attrs: &[Attribute]) -> String {
    let lines = attrs.iter()
        .filter(|attr| attr.path().is_ident("doc"))
        .filter_map(|attr| match &attr.meta {
            syn::Meta::NameValue(nv) => match &nv.value {
                Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(s), .. }) => Some(s.value()),
                _ => None
            },
            _ => None
        })
        .map(|line| line.strip_prefix(' ').map(str::to_string).unwrap_or(line))
        .collect::<Vec<_>>();
    lines.join("\n").trim().to_string()
}

/// Whether the type has `#[reflect(default)]`.
fn reflect_default(input: &DeriveInput) -> syn::Result<bool> {
    let mut default = false;
    for attr in input.attrs.iter().filter(|attr| attr.path().is_ident("reflect")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("default") {
                default = true;
                Ok(())
            } else {
                Err(meta.error("unknown type attribute, expected `default`"))
            }
        })?;
    }
    Ok(default)
}

pub fn derive(input: DeriveInput) -> syn::Result<TokenStream> {
    let name = &input.ident;
    let default = reflect_default(&input)?;
    let type_docs = docs(&input.attrs);

    let mut generics = input.generics.clone();
    let params = generics.type_params().map(|param| param.ident.clone()).collect::<Vec<_>>();
    let where_clause = generics.make_where_clause();
    for param in params {
        where_clause.predicates.push(parse_quote!(#param: ::anarchy::Reflect + ::anarchy::Typed + ::anarchy::FromValue));
    }
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    let (kind, reflect_kind, fields_impl, from_value, fallback) = match &input.data {
        Data::Struct(data) => {
            let fields = FieldSet::parse(&data.fields)?;
            let kind = fields.kind();
            let members = fields.reflected().map(|field| &field.member).collect::<Vec<_>>();
            let indices = (0..members.len()).collect::<Vec<_>>();
            let count = members.len();
            let fields_impl = quote! {
                impl #impl_generics ::anarchy::ReflectFields for #name #ty_generics #where_clause {
                    fn field_count(&self) -> usize { #count }
                    fn field_at(&self, index: usize) -> ::std::option::Option<&dyn ::anarchy::Reflect> {
                        match index { #(#indices => Some(&self.#members),)* _ => None }
                    }
                    fn field_at_mut(&mut self, index: usize) -> ::std::option::Option<&mut dyn ::anarchy::Reflect> {
                        match index { #(#indices => Some(&mut self.#members),)* _ => None }
                    }
                }
            };
            let from_value = match default {
                true => quote! {
                    let mut this = <Self as ::std::default::Default>::default();
                    ::anarchy::Reflect::apply(&mut this, value)?;
                    Ok(this)
                },
                false => fields.construct(quote! { Self })
            };
            let fallback = match default {
                true => quote! { Some(<Self as ::std::default::Default>::default()) },
                false => quote! { <Self as ::anarchy::FromValue>::from_value(&::anarchy::Value::Unit).ok() }
            };
            (quote! { ::anarchy::TypeKind::Struct(#kind) }, quote! { Struct }, fields_impl, from_value, fallback)
        }
        Data::Enum(data) => {
            if data.variants.is_empty() {
                return Err(syn::Error::new_spanned(name, "Reflect can't be derived for enums without variants"));
            }
            let variants = data.variants.iter()
                .map(|variant| Ok((variant, FieldSet::parse(&variant.fields)?)))
                .collect::<syn::Result<Vec<_>>>()?;

            let infos = variants.iter().map(|(variant, fields)| {
                let variant_name = variant.ident.to_string();
                let docs = docs(&variant.attrs);
                let kind = fields.kind();
                quote! { ::anarchy::VariantInfo { name: #variant_name, docs: #docs, kind: #kind } }
            });

            let mut count_arms = vec![];
            let mut ref_arms = vec![];
            let mut index_arms = vec![];
            let mut from_arms = vec![];
            for (i, (variant, fields)) in variants.iter().enumerate() {
                let ident = &variant.ident;
                let (pattern, bindings) = fields.pattern(quote! { Self::#ident });
                let count = bindings.len();
                let indices = (0..count).collect::<Vec<_>>();
                count_arms.push(quote! { #pattern => #count });
                ref_arms.push(quote! { #pattern => match index { #(#indices => Some(#bindings),)* _ => None } });
                index_arms.push(quote! { #pattern => #i });
                let variant_name = ident.to_string();
                let construct = fields.construct(quote! { Self::#ident });
                from_arms.push(quote! { #variant_name => #construct });
            }

            let fields_impl = quote! {
                impl #impl_generics ::anarchy::ReflectFields for #name #ty_generics #where_clause {
                    #[allow(unused_variables)]
                    fn field_count(&self) -> usize { match self { #(#count_arms,)* } }
                    #[allow(unused_variables)]
                    fn field_at(&self, index: usize) -> ::std::option::Option<&dyn ::anarchy::Reflect> {
                        match self { #(#ref_arms,)* }
                    }
                    #[allow(unused_variables)]
                    fn field_at_mut(&mut self, index: usize) -> ::std::option::Option<&mut dyn ::anarchy::Reflect> {
                        match self { #(#ref_arms,)* }
                    }
                }

                impl #impl_generics ::anarchy::ReflectEnum for #name #ty_generics #where_clause {
                    #[allow(unused_variables)]
                    fn variant_index(&self) -> usize { match self { #(#index_arms,)* } }
                    fn set_variant(&mut self, variant: &str, value: &::anarchy::Value) -> ::anarchy::__private::anyhow::Result<()> {
                        let value = ::anarchy::Value::Enum { variant: variant.into(), value: ::std::boxed::Box::new(value.clone()) };
                        *self = <Self as ::anarchy::FromValue>::from_value(&value)?;
                        Ok(())
                    }
                }
            };
            let short_name = name.to_string();
            let from_value = quote! {
                let ::anarchy::Value::Enum { variant, value } = value else {
                    ::anarchy::__private::anyhow::bail!("expected a variant of {}, found {}", #short_name, value)
                };
                match variant.as_str() {
                    #(#from_arms,)*
                    other => ::anarchy::__private::anyhow::bail!("{} has no variant `{}`", #short_name, other)
                }
            };
            let first = data.variants[0].ident.to_string();
            let fallback = match default {
                true => quote! { Some(<Self as ::std::default::Default>::default()) },
                false => quote! {
                    <Self as ::anarchy::FromValue>::from_value(&::anarchy::Value::Enum {
                        variant: #first.into(),
                        value: ::std::boxed::Box::new(::anarchy::Value::Unit)
                    }).ok()
                }
            };
            (quote! { ::anarchy::TypeKind::Enum(vec![#(#infos),*]) }, quote! { Enum }, fields_impl, from_value, fallback)
        }
        Data::Union(_) => return Err(syn::Error::new_spanned(name, "Reflect can't be derived for unions"))
    };

    // inventory registers concrete types only, generic ones are registered by hand
    let auto_register = input.generics.params.is_empty().then(|| quote! {
        ::anarchy::__private::inventory::submit! {
            ::anarchy::__private::AutoRegistration(|| {
                use ::anarchy::__private::{ProbeComponent as _, ProbeNotComponent as _, ProbeResource as _, ProbeNotResource as _};
                let probe = &&::anarchy::__private::RegistrationProbe::<#name>::new();
                ::anarchy::TypeRegistration {
                    component: probe.reflect_component(),
                    resource: probe.reflect_resource(),
                    ..::anarchy::TypeRegistration::of::<#name>()
                }
            })
        }
    });

    let type_docs = LitStr::new(&type_docs, Span::call_site());
    Ok(quote! {
        const _: () = {
            impl #impl_generics ::anarchy::Typed for #name #ty_generics #where_clause {
                fn type_info() -> &'static ::anarchy::TypeInfo {
                    static INFO: ::anarchy::TypeInfoCell = ::anarchy::TypeInfoCell::new();
                    INFO.get_or_init::<Self>(|| ::anarchy::TypeInfo {
                        type_name: ::std::any::type_name::<Self>(),
                        short_name: ::anarchy::short_type_name(::std::any::type_name::<Self>()),
                        docs: #type_docs,
                        kind: #kind
                    })
                }
            }

            impl #impl_generics ::anarchy::Reflect for #name #ty_generics #where_clause {
                fn reflect_info(&self) -> &'static ::anarchy::TypeInfo { <Self as ::anarchy::Typed>::type_info() }
                fn reflect_ref(&self) -> ::anarchy::ReflectRef<'_> { ::anarchy::ReflectRef::#reflect_kind(self) }
                fn reflect_mut(&mut self) -> ::anarchy::ReflectMut<'_> { ::anarchy::ReflectMut::#reflect_kind(self) }
                fn reflect_any(&self) -> &dyn ::std::any::Any { self }
                fn reflect_any_mut(&mut self) -> &mut dyn ::std::any::Any { self }
                fn into_any(self: ::std::boxed::Box<Self>) -> ::std::boxed::Box<dyn ::std::any::Any> { self }
            }

            #fields_impl

            impl #impl_generics ::anarchy::FromValue for #name #ty_generics #where_clause {
                fn from_value(value: &::anarchy::Value) -> ::anarchy::__private::anyhow::Result<Self> { #from_value }
                fn fallback() -> ::std::option::Option<Self> { #fallback }
            }

            #auto_register
        };
    })
}
