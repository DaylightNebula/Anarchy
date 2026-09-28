use anyhow::*;
use mutual::*;

use crate::*;

pub trait QueryComponent {
    type Output;

    fn req_mut() -> bool;
    fn req_comp() -> ComponentID;
    fn extract(comps: Option<&RelaxedMutex<DynComponent>>) -> anyhow::Result<Self::Output>;
}

impl <A: ComponentMeta + Component> QueryComponent for &A {
    type Output = Ref<A>;

    fn req_mut() -> bool {
        false
    }

    fn req_comp() -> ComponentID {
        A::id()
    }

    fn extract(comp: Option<&RelaxedMutex<DynComponent>>) -> anyhow::Result<Self::Output> {
        let comp = comp.context("no component given to required query component")?.lock_ref();
        if comp.get_id() != A::id() { bail!("incorrect component") }
        Ok(Ref::new(comp, |comp| comp.downcast_ref().unwrap()))
    }
}

impl <A: ComponentMeta + Component> QueryComponent for &mut A {
    type Output = Mut<A>;

    fn req_mut() -> bool {
        true
    }

    fn req_comp() -> ComponentID {
        A::id()
    }

    fn extract(comp: Option<&RelaxedMutex<DynComponent>>) -> anyhow::Result<Self::Output> {
        let comp = comp.context("no component given to required query component")?.lock_mut();
        if comp.get_id() != A::id() { bail!("incorrect component") }
        Ok(Mut::new(
            comp, 
            |comp| comp.as_any().downcast_ref().unwrap(),
            |comp| comp.as_any_mut().downcast_mut().unwrap()
        ))
    }
}

impl <A: ComponentMeta + Component> QueryComponent for Option<&A> {
    type Output = Option<Ref<A>>;

    fn req_mut() -> bool {
        false
    }

    fn req_comp() -> ComponentID {
        A::id()
    }

    fn extract(comp: Option<&RelaxedMutex<DynComponent>>) -> anyhow::Result<Self::Output> {
        let Some(comp) = comp else { return Ok(None) };
        let comp = comp.lock_ref();
        if comp.get_id() != A::id() { bail!("incorrect component") }
        Ok(Some(Ref::new(comp, |comp| comp.downcast_ref().unwrap())))
    }
}

impl <A: ComponentMeta + Component> QueryComponent for Option<&mut A> {
    type Output = Option<Mut<A>>;

    fn req_mut() -> bool {
        true
    }

    fn req_comp() -> ComponentID {
        A::id()
    }

    fn extract(comp: Option<&RelaxedMutex<DynComponent>>) -> anyhow::Result<Self::Output> {
        let Some(comp) = comp else { return Ok(None) };
        let comp = comp.lock_mut();
        if comp.get_id() != A::id() { bail!("incorrect component") }
        Ok(Some(Mut::new(
            comp, 
            |comp| comp.as_any().downcast_ref().unwrap(),
            |comp| comp.as_any_mut().downcast_mut().unwrap()
        )))
    }
}