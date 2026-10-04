use crate::system::control::serve::living::Living;
use crate::system::operator::serve::install::Tree;
use crate::system::run::publication::book::Publications;
use protocol::common::schedule::{Progress, Res, ResMut};

pub(crate) fn retire(
    living: Res<Living>,
    mut publications: ResMut<Publications>,
    mut tree: ResMut<Tree>,
) -> Result<Progress, &'static str> {
    let tree = &mut *tree;
    let live = |task| living.contains(task);

    let mut at = 0;
    while at < publications.records.len() {
        let r = &publications.records[at];
        if r.installation.mount.is_none()
            || !live(r.source.publisher)
            || !live(r.installation.owner)
        {
            publications.remove(tree, at)?;
        } else {
            at += 1;
        }
    }
    Ok(Progress::Done)
}
