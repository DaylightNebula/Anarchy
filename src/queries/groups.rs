use anyhow::bail;
use mutual::SharedData;

use crate::*;

pub trait QueryGroup {
    type Output;

    fn req_mut() -> bool;
    fn req_comps() -> ComponentIDGroup;
    fn from_comps(comps: DynComponents) -> anyhow::Result<Self::Output>;
}

impl <QC: QueryComponent> QueryGroup for QC {
    type Output = QC::Output;

    fn req_mut() -> bool { QC::req_mut() }
    fn req_comps() -> ComponentIDGroup { Box::new([QC::req_comp()]) }
    fn from_comps(comps: DynComponents) -> anyhow::Result<Self::Output> {
        if comps.is_empty() { bail!("components given to group empty") }
        let found = comps.iter()
            .filter(|b| b.lock_ref().get_id() == QC::req_comp())
            .next();
        QC::extract(found)
    }
}
