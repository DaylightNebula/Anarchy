use anarchy_macros::cge_builder;

use crate::{ComponentID, ComponentQueryPart, ExtractContext, MaskBuilder, QueryIter, WorldDatabase, extract_comps};

pub trait ComponentGroupExtractor {
    type GroupOutput;

    fn search_ids() -> Vec<ComponentID>;
    fn append_req_mask(builder: &mut MaskBuilder);
    fn append_opt_mask(builder: &mut MaskBuilder);

    fn new_iter<'a, D: WorldDatabase>(db: &'a D) -> QueryIter<'a, Self> where Self: Sized;
}

impl <A: ComponentQueryPart> ComponentGroupExtractor for A {
    type GroupOutput = A::Output;

    fn search_ids() -> Vec<ComponentID> { vec![ A::id() ] }

    fn append_req_mask(builder: &mut MaskBuilder) {
        A::append_req_mask(builder);
    }

    fn append_opt_mask(builder: &mut MaskBuilder) {
        A::append_opt_mask(builder);
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
                    let (comp, new_ctx) = { 
                        let (mut iter, new_ctx) = extract_comps(&*entity, &search_ids, &ctx);
                        let comp = A::extract(iter.next().flatten());
                        (comp, new_ctx)
                    };
                    if let Some(new) = new_ctx { ctx = Some(new); }
                    (entity.0, comp_mask.clone(), comp)
                })
            });
        QueryIter { 0: Box::new(iter) }
    }
}

impl <
    A: ComponentQueryPart,
    B: ComponentQueryPart
> ComponentGroupExtractor for (A, B) {
    type GroupOutput = (A::Output, B::Output);

    fn search_ids() -> Vec<ComponentID> { vec![ A::id(), B::id() ] }

    fn append_req_mask(builder: &mut MaskBuilder) {
        A::append_req_mask(builder);
        B::append_req_mask(builder);
    }

    fn append_opt_mask(builder: &mut MaskBuilder) {
        A::append_opt_mask(builder);
        B::append_opt_mask(builder);
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
                        let a = A::extract(iter.next().flatten());
                        let b = B::extract(iter.next().flatten());
                        ((a, b), new_ctx)
                    };
                    if let Some(new) = new_ctx { ctx = Some(new); }
                    (entity.0, comp_mask.clone(), extract)
                })
            });
        QueryIter { 0: Box::new(iter) }
    }
}

cge_builder!(3);
cge_builder!(4);
cge_builder!(5);
cge_builder!(6);
cge_builder!(7);
cge_builder!(8);
cge_builder!(9);
cge_builder!(10);
cge_builder!(11);
cge_builder!(12);
cge_builder!(13);
cge_builder!(14);
cge_builder!(15);
cge_builder!(16);
cge_builder!(17);
cge_builder!(18);
cge_builder!(19);
cge_builder!(20);
