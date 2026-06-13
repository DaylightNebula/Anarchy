use crate::{CastableSharedData, Component, ComponentID, ComponentMeta, MaskBuilder, MutCastGuard, RefCastGuard, RelaxedMutex};

/// A standard trait to allow something to be extracted from an entities component.
pub trait ComponentQueryPart {
    type Output;
    fn id() -> ComponentID;
    fn append_req_mask(builder: &mut MaskBuilder);
    fn append_opt_mask(builder: &mut MaskBuilder);
    fn extract<'a>(mutex: Option<&'a RelaxedMutex<Box<dyn Component + 'static>>>) -> Self::Output;
}

/// Extract an immutable reference to a component from an `Entity`.
impl <A: ComponentMeta + 'static> ComponentQueryPart for &A {
    type Output = RefCastGuard<Box<dyn Component>, A>;
    fn id() -> ComponentID { A::bit_mask() }
    fn append_req_mask(builder: &mut MaskBuilder) { builder.insert::<A>(); }
    fn append_opt_mask(_builder: &mut MaskBuilder) {}
    fn extract<'a>(mutex: Option<&'a RelaxedMutex<Box<dyn Component + 'static>>>) -> Self::Output { 
        mutex
            .expect("Query did not follow mask")
            .lock_cast_ref() 
    }
}

/// Extract a mutable reference to a component from an `Entity`.
impl <A: ComponentMeta + 'static> ComponentQueryPart for &mut A {
    type Output = MutCastGuard<Box<dyn Component>, A>;
    fn id() -> ComponentID { A::bit_mask() }
    fn append_req_mask(builder: &mut MaskBuilder) { builder.insert::<A>(); }
    fn append_opt_mask(_builder: &mut MaskBuilder) {}
    fn extract<'a>(mutex: Option<&'a RelaxedMutex<Box<dyn Component + 'static>>>) -> Self::Output { 
        mutex
            .expect("Query did not follow mask")
            .lock_cast_mut() 
    }
}

/// Extract an option to an immutable reference to a component from an `Entity`.
impl <A: ComponentMeta + 'static> ComponentQueryPart for Option<&A> {
    type Output = Option<RefCastGuard<Box<dyn Component>, A>>;
    fn id() -> ComponentID { A::bit_mask() }
    fn append_req_mask(_builder: &mut MaskBuilder) {}
    fn append_opt_mask(builder: &mut MaskBuilder) { builder.insert::<A>(); }
    fn extract<'a>(mutex: Option<&'a RelaxedMutex<Box<dyn Component + 'static>>>) -> Self::Output { 
        mutex.map(|a| a.lock_cast_ref())
    } 
}

/// Extract an option to a mutable reference to a component from an `Entity`.
impl <A: ComponentMeta + 'static> ComponentQueryPart for Option<&mut A> {
    type Output = Option<MutCastGuard<Box<dyn Component>, A>>;
    fn id() -> ComponentID { A::bit_mask() }
    fn append_req_mask(_builder: &mut MaskBuilder) {}
    fn append_opt_mask(builder: &mut MaskBuilder) { builder.insert::<A>(); }
    fn extract<'a>(mutex: Option<&'a RelaxedMutex<Box<dyn Component + 'static>>>) -> Self::Output { 
        mutex.map(|a| a.lock_cast_mut())
    }
}
