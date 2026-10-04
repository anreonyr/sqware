use crate::system::run::bootstrap::Boot;
use crate::system::{
    boot,
    control::{
        core::{publication::Publications, verdict},
        serve::{
            answer, frame, lifecycle, living::Living, material::Supplies, publication, resource,
            start::Images, unit::Control, watch,
        },
    },
    identity::serve::{install::Roster, names},
    life,
    operator::serve::install::Tree,
};
use alloc::{collections::VecDeque, vec::Vec};
use env::pie;
use protocol::common::schedule::{Dispatch, Resources as Registry};
use runtime::core::res::bell::Bell;
pub fn resources(boot: Boot) -> Result<Registry<'static>, &'static str> {
    let status = boot::status();
    let entry = pie::unseal_hole(protocol::system::control::publication::ENTRY)
        .map_err(|_| "publication entry")?;
    let mut resources = Registry::new();
    macro_rules! put {
        ($value:expr) => {
            resources
                .insert($value)
                .map_err(|_| "system resource capacity")?
        };
    }
    put!(crate::system::identity::serve::revision::Epoch::new());
    put!(crate::system::identity::serve::revision::Changed(
        Bell::unseal().map_err(|_| "identity change bell")?
    ));
    put!(Control::new(status.clone()));
    put!(super::account::Accounts::new(boot.catalog)?);
    crate::system::loader::serve::install::resources(&mut resources)?;
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
    put!(resource::Runtimes {
        requests: Vec::new(),
        seen: 0,
        checked: Vec::new()
    });
    put!(names::Names::new());
    put!(names::Registrations {
        requests: Vec::new(),
        seen: 0,
        dirty: false
    });
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
        quiet: env::chrono::clock(),
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
