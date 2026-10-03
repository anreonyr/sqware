use super::{bootstrap, scene};
use crate::system::control::{
    core::publication::Publications,
    serve::{
        self,
        material::Supplies,
        resource::Resources,
        start::{self, Images},
        unit::Control,
        watch::Watch,
    },
};
use crate::system::identity::serve::{install::Roster, names::Names};
use crate::system::operator::serve::install::Tree;
use crate::system::{boot, life};
use protocol::debug;
use runtime::env::{mail, room};

pub fn run() -> Result<(), env::Reason> {
    let boot = bootstrap::take().map_err(|e| e.code())?;
    let list = scene::programs(&boot.catalog).map_err(|_| start::E_PROGRAM)?;
    if list.is_empty() {
        return Err(start::E_PROGRAM);
    }
    let status = crate::system::boot::start().map_err(|_| start::E_TABLE)?;
    let result = (|| {
        let entry = mail::unseal_hole(protocol::system::control::publication::ENTRY)
            .map_err(|_| start::E_TABLE)?;
        let images = Images {
            catalog: boot.catalog,
            entry,
        };
        let mut supplies = Supplies::new(boot.machine, boot.accounts);
        let mut control = Control::new(status.clone());
        let mut roster = Roster::default();
        let mut tree = Tree::default();
        let mut watch = Watch::new().map_err(|_| start::E_TABLE)?;
        let mut publications = Publications::new();
        let mut resources = Resources::new();
        let mut names = Names::new();
        let mut activation = None;
        boot::install(
            &status,
            &mut roster,
            &mut tree,
            &mut publications,
            entry,
            &mut names,
            &mut watch,
        )
        .map_err(|why| {
            debug::put(why);
            start::E_TABLE
        })?;
        serve::run(
            &mut watch,
            &mut control,
            &roster,
            &mut supplies,
            &mut activation,
            &images,
            &mut publications,
            &mut resources,
            &mut names,
            &mut tree,
            &list,
        )
        .map_err(|_| 9usize)?;
        life::stop(&status).map_err(|_| 9usize)
    })();
    if result.is_err() {
        let _ = room::doom(status.control);
    }
    result
}
