use convert_case::{Case, Casing};
use proc_macro2::{Span, TokenStream};
use quote::quote;
use syn::{punctuated::Punctuated, token::{Colon, Paren, Plus}, *};

#[proc_macro]
pub fn log(item: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let span = proc_macro::Span::call_site();
    let file = span.file();
    let file_path = LitStr::new(&file, Span::call_site());

    let tokens: proc_macro2::TokenStream = item.into();

    quote! {
        anarchy::logger::log(#file_path, #tokens)
    }.into()
}

#[proc_macro]
pub fn error(item: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let span = proc_macro::Span::call_site();
    let file = span.file();
    let file_path = LitStr::new(&file, Span::call_site());

    let tokens: proc_macro2::TokenStream = item.into();

    quote! {
        anarchy::logger::log(#file_path, anarchy::logger::ERROR, &format!(#tokens))
    }.into()
}

#[proc_macro]
pub fn warn(item: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let span = proc_macro::Span::call_site();
    let file = span.file();
    let file_path = LitStr::new(&file, Span::call_site());

    let tokens: proc_macro2::TokenStream = item.into();

    quote! {
        anarchy::logger::log(#file_path, anarchy::logger::WARNING, &format!(#tokens))
    }.into()
}

#[proc_macro]
pub fn info(item: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let span = proc_macro::Span::call_site();
    let file = span.file();
    let file_path = LitStr::new(&file, Span::call_site());

    let tokens: proc_macro2::TokenStream = item.into();

    quote! {
        anarchy::logger::log(#file_path, anarchy::logger::INFO, &format!(#tokens))
    }.into()
}

#[proc_macro]
pub fn debug(item: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let span = proc_macro::Span::call_site();
    let file = span.file();
    let file_path = LitStr::new(&file, Span::call_site());

    let tokens: proc_macro2::TokenStream = item.into();

    quote! {
        anarchy::logger::log(#file_path, anarchy::logger::DEBUG, &format!(#tokens))
    }.into()
}

#[proc_macro]
pub fn trace(item: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let span = proc_macro::Span::call_site();
    let file = span.file();
    let file_path = LitStr::new(&file, Span::call_site());

    let tokens: proc_macro2::TokenStream = item.into();

    quote! {
        anarchy::logger::log(#file_path, anarchy::logger::TRACE, &format!(#tokens))
    }.into()
}

#[proc_macro]
pub fn cge_builder(item: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = parse_macro_input!(item as LitInt);
    let tuple_size: u32 = input.base10_parse().expect("Failed to parse input");

    let mut impl_generics = TokenStream::default();
    let mut impl_for = TokenStream::default();
    let mut id_builder = TokenStream::default();
    let mut output_builder = TokenStream::default();
    let mut req_builder = TokenStream::default();
    let mut opt_builder = TokenStream::default();
    let mut extractor_builder = TokenStream::default();

    for idx in 0 .. tuple_size {
        let letter = ALPHABET.chars().nth(idx as usize).unwrap().to_string();
        let ident = Ident::new(&letter, Span::call_site());

        impl_for.extend(quote! { #ident, });
        impl_generics.extend(quote! { #ident: ComponentQueryPart, });
        id_builder.extend(quote! { #ident::id(), });
        output_builder.extend(quote! { #ident::Output, });
        req_builder.extend(quote! { #ident::append_req_mask(builder); });
        opt_builder.extend(quote! { #ident::append_opt_mask(builder); });
        extractor_builder.extend(quote! { #ident::extract(iter.next().flatten()), });
    }

    quote! {
        impl <#impl_generics> ComponentGroupExtractor for (#impl_for) {
            type GroupOutput = (#output_builder);

            fn search_ids() -> Vec<ComponentID> { vec![ #id_builder ] }

            fn append_req_mask(builder: &mut MaskBuilder) {
                #req_builder
            }

            fn append_opt_mask(builder: &mut MaskBuilder) {
                #opt_builder
            }

            fn new_iter<'a, D: WorldDatabase>(db: &'a D) -> QueryIter<'a, Self> {
                let mut req_builder = MaskBuilder::new();
                Self::append_req_mask(&mut req_builder);
                let req_mask = req_builder.build();

                let search_ids = Self::search_ids();

                let iter = db.query(req_mask.clone())
                    .flat_map(move |(comp_mask, chunk)| {
                        let mut ctx: Option<ExtractContext> = None;
                        let search_ids = search_ids.clone();
                        chunk.map(move |entity| {
                            let (extract, new_ctx) = {
                                let (mut iter, new_ctx) = extract_comps(&*entity, &search_ids, &ctx);
                                let extract = (#extractor_builder);
                                (extract, new_ctx)
                            };
                            if let Some(new) = new_ctx { ctx = Some(new); }
                            (entity.0, comp_mask.clone(), extract)
                        })
                    });
                QueryIter { 0: Box::new(iter) }
            }
        }
    }.into()
}


#[proc_macro_attribute]
pub fn system(_attr: proc_macro::TokenStream, item: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let item = parse_macro_input!(item as ItemFn);
    let ident = &item.sig.ident;
    let vis = &item.vis;
    let block = &item.block;
    let ident_lit_str = LitStr::new(&ident.to_string(), Span::call_site());

    // create inputs structure
    let mut inputs = quote! {};
    inputs.extend(quote!{ world: &anarchy::World, });
    inputs.extend(quote!{ schedule_id: anarchy::scheduler::ScheduleID, });

    // create unpack structrue
    let mut unpack = quote! {};

    let output = match item.sig.output {
        syn::ReturnType::Default => quote! { () },
        syn::ReturnType::Type(_, ty) => quote! { #ty },
    };

    // find final input
    let mut final_input = quote! { () };
    for input in &item.sig.inputs {
        match input {
            syn::FnArg::Typed(pat_type) => {
                match &*pat_type.ty {
                    syn::Type::Path(type_path) => {
                        let primary_segment = type_path.path.segments.last().expect("No last path segment in type.");
                        let ty_ident = &primary_segment.ident;
                        let ty_generics = match &primary_segment.arguments {
                            syn::PathArguments::AngleBracketed(args) => &args.args,
                            _ => panic!("Invalid query inputs")
                        };
                        
                        match ty_ident.to_string().as_str() {
                            "Input" => {
                                final_input = quote! { #ty_generics };
                            }
                            _ => {}
                        };
                    },
                    _ => {}
                }
            },
            _ => {}
        }
    }

    // unpack inputs
    for input in &item.sig.inputs {
        match input {
            syn::FnArg::Receiver(_receiver) => { /* We don't use these for systems */ },
            syn::FnArg::Typed(pat_type) => {
                // get types name
                let ident = match &*pat_type.pat {
                    syn::Pat::Ident(ident) => &ident.ident,
                    _ => todo!(),
                };
                
                match &*pat_type.ty {
                    syn::Type::Path(type_path) => {
                        let primary_segment = type_path.path.segments.last().expect("No last path segment in type.");

                        unpack.extend(quote! {
                            let (mut #ident, mut _inputs) = <#primary_segment as anarchy::SystemExtractor<&#final_input>>::extract(schedule_id, world, _inputs);
                        });
                    },
                    _ => panic!("Unknown pat type {:?}", pat_type.ty),
                };
            }
        }
    }

    quote! {
        #[allow(non_camel_case_types)]
        #vis struct #ident;
        impl anarchy::System<#final_input, Result<#output, Box<dyn std::error::Error>>> for #ident {
            fn name(&self) -> &str { #ident_lit_str }

            fn execute<'a>(
                &self,
                schedule_id: anarchy::ScheduleID,
                world: &'a anarchy::World,
                _inputs: &'a #final_input
            ) -> Result<#output, Box<dyn std::error::Error>> {
                let mut _inputs = Some(&_inputs);
                #unpack
                let result = #block;
                #[allow(unreachable_code)] // this is needed if/when block returns
                Ok(result)
            }
        }
    }.into()
}


#[proc_macro_derive(Getters)]
pub fn getters(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let ast = parse_macro_input!(input as DeriveInput);
    let ident = &ast.ident;
    let (impl_generics, ty_generics, where_clause) = ast.generics.split_for_impl();

    let fields = if let syn::Data::Struct(syn::DataStruct { fields: Fields::Named(fields), .. }) = &ast.data {
        &fields.named
    } else {
        panic!("Getters derive macro only works on structs with named fields");
    };

    let getters = fields.iter().filter_map(|field| {
        let should_skip = field.attrs.iter().any(|attr| attr.path().is_ident("skip_getter"));
        if should_skip { return None; }

        let field_name = field.ident.as_ref()?;
        let field_type = &field.ty;

        // Generate the getter method
        let getter_name = field_name;
        Some(quote! {
            pub fn #getter_name(&self) -> &#field_type {
                &self.#field_name
            }
        })
    }).collect::<proc_macro2::TokenStream>();

    quote! {
        impl #impl_generics #ident #ty_generics #where_clause {
            #getters
        }
    }.into()
}

#[proc_macro_derive(GettersMut)]
pub fn getters_mut(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let ast = parse_macro_input!(input as DeriveInput);
    let ident = &ast.ident;
    let (impl_generics, ty_generics, where_clause) = ast.generics.split_for_impl();

    let fields = if let syn::Data::Struct(syn::DataStruct { fields: Fields::Named(fields), .. }) = &ast.data {
        &fields.named
    } else {
        panic!("Getters derive macro only works on structs with named fields");
    };

    let getters = fields.iter().filter_map(|field| {
        let should_skip = field.attrs.iter().any(|attr| attr.path().is_ident("skip_getter"));
        if should_skip { return None; }

        let field_name = field.ident.as_ref()?;
        let field_type = &field.ty;

        // Generate the getter method
        let getter_mut_name = Ident::new(&format!("{}_mut", field_name.to_string()), Span::call_site());
        Some(quote! {
            pub fn #getter_mut_name(&mut self) -> &mut #field_type {
                &mut self.#field_name
            }
        })
    }).collect::<proc_macro2::TokenStream>();

    quote! {
        impl #impl_generics #ident #ty_generics #where_clause {
            #getters
        }
    }.into()
}

#[proc_macro_derive(Setters)]
pub fn setters(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let ast = parse_macro_input!(input as DeriveInput);
    let ident = &ast.ident;
    let (impl_generics, ty_generics, where_clause) = ast.generics.split_for_impl();

    let fields = if let syn::Data::Struct(syn::DataStruct { fields: Fields::Named(fields), .. }) = &ast.data {
        &fields.named
    } else {
        panic!("Getters derive macro only works on structs with named fields");
    };

    let getters = fields.iter().filter_map(|field| {
        let should_skip = field.attrs.iter().any(|attr| attr.path().is_ident("skip_setter"));
        if should_skip { return None; }

        let field_name = field.ident.as_ref()?;
        let field_type = &field.ty;

        // Generate the getter method
        let setter_name = Ident::new(&format!("set_{}", field_name.to_string()), Span::call_site());
        Some(quote! {
            pub fn #setter_name(&mut self, #field_name: #field_type) -> &mut Self {
                self.#field_name = #field_name;
                self
            }
        })
    }).collect::<proc_macro2::TokenStream>();

    quote! {
        impl #impl_generics #ident #ty_generics #where_clause {
            #getters
        }
    }.into()
}

#[proc_macro_derive(Event)]
pub fn event_derive(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let ast = parse_macro_input!(input as DeriveInput);
    let name = &ast.ident;
    let (impl_generics, ty_generics, where_clause) = ast.generics.split_for_impl();

    let upper_snake_name = name.to_string().to_case(Case::UpperSnake);
    let id_name = format!("{}_ID", upper_snake_name);
    let id_name = Ident::new(&id_name, Span::call_site());
    let next_id_name = format!("{}_NEXT_ID", upper_snake_name);
    let next_id_name = Ident::new(&next_id_name, Span::call_site());

    quote! {
        static #id_name: std::sync::OnceLock<anarchy::events::EventID> = std::sync::OnceLock::new();

        impl #impl_generics mutual::AsAny for #name #ty_generics #where_clause {
            fn as_any(&self) -> &dyn std::any::Any { self }
            fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
        }

        impl #impl_generics anarchy::events::EventImpl for #name #ty_generics #where_clause {
            fn get_id(&self) -> anarchy::events::EventID { 
                use anarchy::events::EventMeta;
                Self::id() 
            }
        }

        static #next_id_name: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        impl #impl_generics anarchy::events::EventMeta for #name #ty_generics #where_clause {
            fn id() -> anarchy::events::EventID {
                *#id_name.get_or_init(|| {
                    anarchy::events::NEXT_EVENT_ID
                        .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                })
            }

            fn next_instance_id() -> anarchy::events::EventInstanceID { #next_id_name.fetch_add(1, std::sync::atomic::Ordering::SeqCst) }
        }
    }.into()
}

#[proc_macro_derive(Component)]
pub fn component_derive(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let ast = parse_macro_input!(input as DeriveInput);
    let name = &ast.ident;
    let (impl_generics, ty_generics, where_clause) = ast.generics.split_for_impl();

    let bit_mask_name = name.to_string().to_case(Case::UpperSnake);
    let bit_mask_name = format!("{}_BIT_MASK", bit_mask_name);
    let bit_mask_name = Ident::new(&bit_mask_name, Span::call_site());

    let expanded = quote! {
        static #bit_mask_name: std::sync::OnceLock<anarchy::ecs::components::ComponentID> = std::sync::OnceLock::new();
        
        impl #impl_generics anarchy::ecs::components::ComponentMeta for #name #ty_generics #where_clause {
            fn bit_mask() -> anarchy::ecs::components::ComponentID {
                *#bit_mask_name.get_or_init(|| {
                    anarchy::ecs::components::NEXT_BIT_MASK
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                })
            }
        }
        
        impl #impl_generics anarchy::ecs::components::Component for #name #ty_generics #where_clause {
            fn get_bit_mask(&self) -> anarchy::ecs::components::ComponentID { 
                use anarchy::ecs::components::ComponentMeta;
                Self::bit_mask() 
            }
        }

        impl #impl_generics mutual::AsAny for #name #ty_generics #where_clause {
            fn as_any(&self) -> &dyn std::any::Any { self }
            fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
        }
    };

    // Return the generated code as a TokenStream
    expanded.into()
}

#[proc_macro_derive(Resource)]
pub fn resource_derive(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let ast = parse_macro_input!(input as DeriveInput);
    let name = &ast.ident;
    let (impl_generics, ty_generics, where_clause) = ast.generics.split_for_impl();

    let id_name = name.to_string().to_case(Case::UpperSnake);
    let id_name = format!("{}_ID", id_name);
    let id_name = Ident::new(&id_name, Span::call_site());

    let expanded = quote! {
        static #id_name: std::sync::OnceLock<anarchy::ecs::resources::ResourceID> = std::sync::OnceLock::new();
        
        impl #impl_generics anarchy::ecs::resources::ResourceMeta for #name #ty_generics #where_clause {
            fn id() -> anarchy::ecs::resources::ResourceID {
                *#id_name.get_or_init(|| {
                    anarchy::ecs::resources::NEXT_RESOURCE_ID
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                })
            }
        }

        unsafe impl #impl_generics Send for #name #ty_generics #where_clause {}
        unsafe impl #impl_generics Sync for #name #ty_generics #where_clause {}
        
        impl #impl_generics anarchy::ecs::resources::Resource for #name #ty_generics #where_clause {
            fn get_id(&self) -> anarchy::ecs::resources::ResourceID { 
                use anarchy::ecs::resources::ResourceMeta;
                Self::id() 
            }
        }

        impl #impl_generics mutual::AsAny for #name #ty_generics #where_clause {
            fn as_any(&self) -> &dyn std::any::Any { self }
            fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
        }
    };

    // Return the generated code as a TokenStream
    expanded.into()
}

// NOTE: D, I, O is missing, this is due to generic D being using for databases in the new function
const ALPHABET: &str = "ABCEFGHJKLMNPQRSTUVWXYZ";
const ALPHABET_LOWER: &str = "abcefghjklmnpqrstuvwxyz";

#[proc_macro]
pub fn system_builder(item: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = parse_macro_input!(item as LitInt);
    let tuple_size: usize = input.base10_parse().expect("Failed to parse input");

    let mut generics = proc_macro2::TokenStream::new();
    let mut generics_assign = proc_macro2::TokenStream::new();
    let mut extractor = proc_macro2::TokenStream::new();
    let mut fields = proc_macro2::TokenStream::new();

    for idx in 0 .. tuple_size {
        let letter = ALPHABET.chars().nth(idx).unwrap().to_string();
        let letter_lower = ALPHABET_LOWER.chars().nth(idx).unwrap().to_string();
        let letter = Ident::new(&letter, Span::call_site());
        let letter_lower = Ident::new(&letter_lower, Span::call_site());

        generics.extend(quote! { #letter, });
        generics_assign.extend(quote! { #letter: for<'a> SystemExtractor<'a, I>, });
        extractor.extend(quote! { let #letter_lower = #letter::extract(schedule_id, world, &mut inputs); });
        fields.extend(quote! { #letter_lower, });
    }

    quote! {
        impl<I, O, #generics> System<I, O> for fn(#generics) -> O
            where #generics_assign
        {
            fn execute<'a>(
                &mut self,
                schedule_id: ScheduleID,
                world: &'a World,
                inputs: I
            ) -> O {
                let mut inputs = Some(inputs);
                #extractor
                (self)(#fields)
            }
        }
    }.into()
}

#[proc_macro]
pub fn query_builder(item: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let mut stream = proc_macro::TokenStream::default();
    let input = parse_macro_input!(item as LitInt);
    let tuple_size: u32 = input.base10_parse().expect("Failed to parse input");
    
    // create generics
    let a_lifetime = LifetimeParam::new(Lifetime::new("'b", Span::call_site()));
    let mut generics = Generics::default();
    generics.params.push(GenericParam::Lifetime(a_lifetime.clone()));

    let mut tuple = Punctuated::new();
    let mut item_tuple = Punctuated::new();
    let mut iter_tuple = Punctuated::new();
    let mut builders = TokenStream::default();
    let mut nones_builder = TokenStream::default();
    let mut is_builder = TokenStream::default();
    let mut ids_builder = TokenStream::default();

    for bit in 0 .. tuple_size {
        let letter = ALPHABET.chars().nth(bit as usize).unwrap().to_string();
        let letter_lower = ALPHABET_LOWER.chars().nth(bit as usize).unwrap().to_string();
        let ident = Ident::new(&letter, Span::call_site());
        let lower_ident = Ident::new(&letter_lower, Span::call_site());
        let bit_lit = LitInt::new(&bit.to_string(), Span::call_site());

        // create type param bounds (ComponentMeta + 'static)
        let mut bounds = Punctuated::<TypeParamBound, Plus>::new();
        bounds.push(parse_quote!(ComponentQueryPart));

        tuple.push(syn::Type::Verbatim(quote! { #ident }));
        builders.extend(quote! { 
            #ident::append_req_mask(&mut req_builder);
        });
        nones_builder.extend(quote! { let mut #lower_ident = None; });
        is_builder.extend(quote! { if comp.lock_ref().get_bit_mask() == #ident::id() { #lower_ident = Some(comp); } });
        iter_tuple.push(syn::Type::Verbatim(quote! { #ident::extract(mutexs[#bit_lit]) }));
        item_tuple.push(syn::Type::Verbatim(quote! { #ident::Output }));
        ids_builder.extend(quote! { #ident::id(), });

        // add type param to primary generic
        generics.params.push(GenericParam::Type(TypeParam {
            attrs: Vec::new(),
            ident,
            colon_token: Some(Colon::default()),
            bounds,
            eq_token: None,
            default: None,
        }));
    }
    
    // create tuple
    let tuple = TypeTuple {
        paren_token: Paren::default(),
        elems: tuple
    };
    let item_tuple = TypeTuple {
        paren_token: Paren::default(),
        elems: item_tuple
    };
    let iter_tuple = TypeTuple {
        paren_token: Paren::default(),
        elems: iter_tuple
    };

    stream.extend(proc_macro::TokenStream::from(
        quote::quote! {
            impl #generics QueryCreator for Query<#a_lifetime, #tuple, #item_tuple> {
                type InputTypes = #item_tuple;

                fn new<'a, D: WorldDatabase>(db: &'a D) -> QueryIter<'a, #item_tuple> {
                    let mut req_builder = MaskBuilder::new();
                    #builders
                    let req_mask = req_builder.build();

                    let search_ids = [#ids_builder];

                    let iter = db.query(req_mask.clone())
                        .flat_map(move |(comp_mask, chunk)| {
                            let mut ctx: Option<ExtractContext> = None;
                            chunk.map(move |entity| {
                                let (mutexs, new_ctx) = extract_comps(entity, &search_ids, &ctx);
                                if let Some(new) = new_ctx { ctx = Some(new); }
                                (entity.0, comp_mask, #iter_tuple)
                            })
                        });
                    QueryIter(Box::new(iter))
                }
            }
        }
    ));

    return stream;
}
