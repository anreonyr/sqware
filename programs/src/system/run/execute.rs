use super::{install, schedule};
use crate::system::control::serve::start;
use super::frame;
use crate::system::run::{bootstrap, scene};
use runtime::schedule::{Cursor, Progress};
pub fn run() -> Result<(), env::Reason> {
    let boot = bootstrap::take().map_err(|e| e.code())?;
    let list = scene::programs(&boot.catalog).map_err(|_| start::E_PROGRAM)?;
    if list.is_empty() {
        return Err(start::E_PROGRAM);
    }
    let resources = install::resources(boot).map_err(|_| start::E_TABLE)?;
    resources
        .write::<frame::Startup>()
        .map_err(|_| start::E_TABLE)?
        .list = list;
    let mut startup = schedule::startup().map_err(|_| start::E_TABLE)?;
    let mut frame = schedule::frame().map_err(|_| start::E_TABLE)?;
    let mut shutdown = schedule::shutdown().map_err(|_| start::E_TABLE)?;
    startup.prepare(&resources);
    frame.prepare(&resources);
    shutdown.prepare(&resources);
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
                .read::<frame::Flow>()
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
        let _ = env::room::doom(env::unit::self_id());
    }
    result
}
