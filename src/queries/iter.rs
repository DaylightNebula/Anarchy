use crate::*;

pub struct QueryIter<QG: QueryGroup>(pub(crate) Query<QG>);
impl <QG: QueryGroup> Iterator for QueryIter<QG> {
    type Item = (u64, QG::Output);

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().ok().flatten()
    }
}