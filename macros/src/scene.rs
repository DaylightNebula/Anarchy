//! `scene!`, which builds a `Scene` from the scene format written as Rust.

use proc_macro2::{Delimiter, TokenStream, TokenTree};
use quote::{ToTokens, quote};
use syn::{Ident, LitStr, ext::IdentExt, Member, Path, Token, braced, bracketed, parenthesized, parse::{Parse, ParseStream}, token};

pub struct SceneInput {
    resources: Vec<Patch>,
    roots: Vec<Entity>
}

struct Entity {
    name: Option<String>,
    inherits: Option<String>,
    patches: Vec<Patch>,
    children: Vec<Entity>
}

enum Patch {
    /// `Type { a: x }`, fields set on `Type::default()`.
    Fields(Path, Vec<FieldPatch>),
    /// `Type(args)`, used as an expression.
    Call(Path, TokenStream),
    /// `Type`, its `Default`.
    Bare(Path),
    /// `{ expr }`, any expression.
    Expr(TokenStream)
}

struct FieldPatch {
    member: Member,
    value: FieldValue
}

enum FieldValue {
    /// `Type { b: y }`, a nested patch of the field's own fields.
    Nested(Vec<FieldPatch>),
    /// Any other expression, assigned to the field.
    Expr(TokenStream)
}

impl Parse for SceneInput {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut scene = SceneInput { resources: vec![], roots: vec![] };
        while !input.is_empty() {
            if input.peek(Token![@]) {
                input.parse::<Token![@]>()?;
                let patch = Patch::parse(input)?;
                if matches!(patch, Patch::Expr(_)) { return Err(input.error("expected a resource type after `@`")) }
                scene.resources.push(patch);
            } else {
                scene.roots.push(input.parse()?);
            }
            if input.is_empty() { break }
            input.parse::<Token![,]>()?;
        }
        Ok(scene)
    }
}

/// An entity, scene or field name: an identifier or a string.
fn name(input: ParseStream) -> syn::Result<String> {
    if input.peek(LitStr) { Ok(input.parse::<LitStr>()?.value()) } else { Ok(input.call(Ident::parse_any)?.to_string()) }
}

impl Parse for Entity {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut entity = Entity { name: None, inherits: None, patches: vec![], children: vec![] };
        if input.peek(Token![#]) {
            input.parse::<Token![#]>()?;
            entity.name = Some(name(input)?);
        }
        if input.peek(Token![:]) && !input.peek(Token![::]) {
            input.parse::<Token![:]>()?;
            entity.inherits = Some(name(input)?);
        }
        while !input.is_empty() && !input.peek(Token![,]) {
            if input.peek(token::Bracket) {
                let content;
                bracketed!(content in input);
                while !content.is_empty() {
                    entity.children.push(content.parse()?);
                    if content.is_empty() { break }
                    content.parse::<Token![,]>()?;
                }
                break;
            }
            entity.patches.push(Patch::parse(input)?);
        }
        if entity.name.is_none() && entity.inherits.is_none() && entity.patches.is_empty() && entity.children.is_empty() {
            return Err(input.error("expected an entity"));
        }
        Ok(entity)
    }
}

impl Patch {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        if input.peek(token::Brace) {
            let content;
            braced!(content in input);
            return Ok(Patch::Expr(content.parse()?));
        }
        let path = input.parse::<Path>()?;
        if input.peek(token::Brace) {
            let content;
            braced!(content in input);
            Ok(Patch::Fields(path, FieldPatch::parse_all(&content)?))
        } else if input.peek(token::Paren) {
            let content;
            parenthesized!(content in input);
            Ok(Patch::Call(path, content.parse()?))
        } else {
            Ok(Patch::Bare(path))
        }
    }

    /// An expression building the patched value.
    fn build(&self) -> TokenStream {
        match self {
            Patch::Fields(path, fields) => {
                let target = quote! { __value };
                let sets = fields.iter().map(|field| field.set(&target));
                quote! {{
                    let mut __value: #path = ::std::default::Default::default();
                    #(#sets)*
                    __value
                }}
            }
            Patch::Call(path, args) => quote! { #path(#args) },
            Patch::Bare(path) => quote! { <#path as ::std::default::Default>::default() },
            Patch::Expr(expr) => quote! {{ #expr }}
        }
    }
}

impl FieldPatch {
    fn parse_all(input: ParseStream) -> syn::Result<Vec<Self>> {
        let mut fields = vec![];
        while !input.is_empty() {
            let member = input.parse::<Member>()?;
            input.parse::<Token![:]>()?;
            let mut tokens = TokenStream::new();
            while !input.is_empty() && !input.peek(Token![,]) {
                tokens.extend([input.parse::<TokenTree>()?]);
            }
            if tokens.is_empty() { return Err(input.error("expected a value")) }
            fields.push(FieldPatch { member, value: FieldValue::from_tokens(tokens)? });
            if input.is_empty() { break }
            input.parse::<Token![,]>()?;
        }
        Ok(fields)
    }

    /// Statements setting this field on `target`.
    fn set(&self, target: &TokenStream) -> TokenStream {
        let member = &self.member;
        let target = quote! { #target.#member };
        match &self.value {
            FieldValue::Nested(fields) => {
                let sets = fields.iter().map(|field| field.set(&target));
                quote! { #(#sets)* }
            }
            FieldValue::Expr(expr) => quote! { #target = #expr; }
        }
    }
}

impl FieldValue {
    fn from_tokens(tokens: TokenStream) -> syn::Result<Self> {
        let trees = tokens.clone().into_iter().collect::<Vec<_>>();
        // `Path { .. }` is a nested patch, anything else is an expression
        if let [path @ .., TokenTree::Group(group)] = &trees[..]
            && group.delimiter() == Delimiter::Brace
            && !path.is_empty()
            && syn::parse2::<Path>(path.iter().cloned().collect()).is_ok()
        {
            let fields = syn::parse::Parser::parse2(FieldPatch::parse_all, group.stream())?;
            return Ok(FieldValue::Nested(fields));
        }
        // string literals convert, so `name: "bob"` sets a `String`
        if let [TokenTree::Literal(literal)] = &trees[..] && literal.to_string().starts_with('"') {
            return Ok(FieldValue::Expr(quote! { ::std::convert::Into::into(#literal) }));
        }
        Ok(FieldValue::Expr(tokens))
    }
}

impl Entity {
    fn build(&self) -> TokenStream {
        let name = option_string(&self.name);
        let inherits = option_string(&self.inherits);
        let patches = self.patches.iter().map(|patch| {
            let value = patch.build();
            quote! { ::anarchy::ScenePatch::typed(move || #value) }
        });
        let children = self.children.iter().map(Entity::build);
        quote! {
            ::anarchy::SceneEntity {
                name: #name,
                inherits: #inherits,
                components: ::std::vec![#(#patches),*],
                children: ::std::vec![#(#children),*]
            }
        }
    }
}

fn option_string(value: &Option<String>) -> TokenStream {
    match value {
        Some(value) => quote! { ::std::option::Option::Some(::std::string::String::from(#value)) },
        None => quote! { ::std::option::Option::None }
    }
}

impl ToTokens for SceneInput {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let resources = self.resources.iter().map(|patch| {
            let value = patch.build();
            quote! { ::anarchy::ResourcePatch::typed(move || #value) }
        });
        let roots = self.roots.iter().map(Entity::build);
        tokens.extend(quote! {
            ::anarchy::Scene {
                resources: ::std::vec![#(#resources),*],
                roots: ::std::vec![#(#roots),*]
            }
        });
    }
}
