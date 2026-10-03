use super::schedule;
use crate::system::run::{
    bootstrap::{self, Boot},
    scene,
};
use crate::system::{
    boot,
    control::{
        core::{publication::Publications, verdict},
        serve::{
            self, answer, frame, lifecycle,
            living::Living,
            material::Supplies,
            publication, resource,
            start::{self, Images},
            unit::Control,
            watch,
        },
    },
    identity::serve::{install::Roster, names},
    life,
    operator::serve::install::Tree,
};
use alloc::{collections::VecDeque, vec::Vec};
use protocol::common::schedule::{Cursor, Dispatch, Progress, Resources as Registry};
pub fn resources(boot: Boot) -> Result<Registry<'static>, &'static str> {
    let status = boot::status();
    let entry = runtime::env::mail::unseal_hole(protocol::system::control::publication::ENTRY)
        .map_err(|_| "publication entry")?;
    let mut resources = Registry::new();
    macro_rules! put {
        ($value:expr) => {
            resources
                .insert($value)
                .map_err(|_| "system resource capacity")?
        };
    }
    put!(Control::new(status.clone()));
    put!(status);
    put!(boot::Faces(Vec::new()));
    put!(boot::Mounts(Vec::new()));
    put!(Images {
        catalog: boot.catalog,
        entry
    });
    put!(boot.machine);
    put!(Supplies::new(boot.machine, boot.accounts));
    put!(Roster::default());
    put!(Tree::default());
    put!(crate::system::operator::serve::install::Connections(
        Vec::new()
    ));
    put!(watch::Watch::new().map_err(|_| "control watch")?);
    put!(Publications::new());
    put!(resource::Resources::new());
    put!(resource::Runtimes(Vec::new()));
    put!(names::Names::new());
    put!(names::Registrations(Vec::new()));
    put!(None::<crate::service::hub::bridge::Activation>);
    put!(Living::new());
    put!(lifecycle::Operations::new());
    put!(lifecycle::Active(None));
    put!(Dispatch::<lifecycle::Key, verdict::Fail>::new());
    put!(frame::Startup {
        list: Vec::new(),
        at: 0,
        eligible: false
    });
    put!(frame::Flow {
        settling: false,
        forced: false,
        done: false
    });
    put!(frame::Activity {
        owed: 0,
        quiet: runtime::env::chrono::clock(),
        walking: false
    });
    put!(frame::Bound(env::Wait::POLL));
    put!(frame::Shutoff(None));
    put!(answer::Inbox(VecDeque::new()));
    put!(answer::Buffer(alloc::vec![0; runtime::PAGE_SIZE]));
    put!(watch::Interests {
        tokens: Vec::new(),
        subs: Vec::new(),
        armed: false
    });
    put!(publication::Inbox(VecDeque::new()));
    put!(publication::Request(None));
    put!(publication::Outcome(None));
    put!(publication::Decision::Unset);
    put!(publication::Kind { road: None });
    put!(Dispatch::<u8, &'static str>::new());
    put!(life::Deadline(0));
    Ok(resources)
}
pub fn run() -> Result<(), env::Reason> {
    let boot = bootstrap::take().map_err(|e| e.code())?;
    let list = scene::programs(&boot.catalog).map_err(|_| start::E_PROGRAM)?;
    if list.is_empty() {
        return Err(start::E_PROGRAM);
    }
    let resources = resources(boot).map_err(|_| start::E_TABLE)?;
    resources
        .write::<frame::Startup>()
        .map_err(|_| start::E_TABLE)?
        .list = list;
    let mut startup = schedule::startup().map_err(|_| start::E_TABLE)?;
    let mut frame = schedule::frame().map_err(|_| start::E_TABLE)?;
    let mut shutdown = schedule::shutdown().map_err(|_| start::E_TABLE)?;
    let result = (|| {
        let mut starting = Cursor::default();
        let mut cursor = Cursor::default();
        loop {
            startup
                .advance(&mut starting, &resources)
                .map_err(|error| {
                    protocol::debug::put(&alloc::format!("system: startup {:?}", error));
                    start::E_TABLE
                })?;
            if resources
                .read::<serve::frame::Flow>()
                .map_err(|_| start::E_TABLE)?
                .done
            {
                break;
            }
            if frame.advance(&mut cursor, &resources).map_err(|error| {
                protocol::debug::put(&alloc::format!("system: frame {:?}", error));
                9usize
            })? == Progress::Done
            {
                cursor.reset();
            }
        }
        let mut cursor = Cursor::default();
        while shutdown
            .advance(&mut cursor, &resources)
            .map_err(|_| 9usize)?
            == Progress::Pending
        {}
        Ok(())
    })();
    if result.is_err() {
        let _ = runtime::env::room::doom(runtime::env::unit::self_id());
    }
    result
}
