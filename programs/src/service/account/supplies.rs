use alloc::{string::String, vec::Vec};
use env::{PieToken, TaskId, Wait};
use pipe_api::Direction;
use shell_api::{Catalogue, Image};
use wire::Message;
const WAIT: Wait = Wait::AtMost(5000);
pub const COMMANDS: &[(&str, &str, &[(&str, Direction)])] = &[
    (
        "cat",
        "account-cat",
        &[("source", Direction::Read), ("copy", Direction::Write)],
    ),
    ("emit", "account-emit", &[("records", Direction::Write)]),
    (
        "upper",
        "account-upper",
        &[("source", Direction::Read), ("result", Direction::Write)],
    ),
    ("fail", "account-fail", &[]),
    ("spin", "account-spin", &[]),
    ("workers", "account-workers", &[("ticks", Direction::Write)]),
];
struct Cached {
    name: &'static str,
    root: PieToken,
    length: usize,
    ports: &'static [(&'static str, Direction)],
    _load: ipc::session::establish::Held,
}
impl Drop for Cached {
    fn drop(&mut self) {
        let _ = env::pie::release(self.root, env::ReleaseMode::Revoke);
    }
}
struct Delivery {
    mailbox: PieToken,
    watch: PieToken,
}
impl Drop for Delivery {
    fn drop(&mut self) {
        let _ = env::pie::release(self.mailbox, env::ReleaseMode::Revoke);
        let _ = env::pie::release(self.watch, env::ReleaseMode::Revoke);
    }
}
pub struct Supplies {
    images: Vec<Cached>,
    deliveries: Vec<Delivery>,
}
impl Supplies {
    pub fn receive(supervisor: TaskId) -> Result<Self, &'static str> {
        let mut images = Vec::new();
        for &(name, channel, ports) in COMMANDS {
            let (bytes, load) = receive(supervisor, env::Mark::of(channel))?;
            let size = bytes.len().div_ceil(env::PAGE_SIZE) * env::PAGE_SIZE;
            let root = env::pie::unseal(env::UnsealArgs::Pole { size, shared: true })
                .map_err(|_| "account image cache")?;
            let image = Cached {
                name,
                root,
                length: bytes.len(),
                ports,
                _load: load,
            };
            let (address, mapped) =
                resource::raw::open(root).map_err(|_| "account image cache mapping")?;
            if mapped < bytes.len() {
                return Err("account image cache size");
            }
            // SAFETY: this newly created writable mapping covers the copied ELF bytes.
            unsafe {
                core::ptr::copy_nonoverlapping(bytes.as_ptr(), address as *mut u8, bytes.len());
            }
            env::pie::shut(root).map_err(|_| "account image cache unmap")?;
            env::pie::narrow(root, env::Permission::FETCH | env::Permission::VEST)
                .map_err(|_| "account image cache freeze")?;
            images.push(image);
        }
        Ok(Self {
            images,
            deliveries: Vec::new(),
        })
    }
    pub fn send(&mut self, task: TaskId) -> Result<(), &'static str> {
        self.sweep();
        self.deliveries
            .try_reserve(1)
            .map_err(|_| "account catalogue capacity")?;
        let mailbox = env::pie::unseal(env::UnsealArgs::hole(shell_api::CATALOGUE))
            .map_err(|_| "account catalogue mailbox")?;
        let watch = match env::pie::unseal(env::UnsealArgs::Tole { shared: false }) {
            Ok(watch) => watch,
            Err(_) => {
                let _ = env::pie::release(mailbox, env::ReleaseMode::Revoke);
                return Err("account catalogue watch");
            }
        };
        let delivery = Delivery { mailbox, watch };
        env::pie::accord(mailbox, task, env::Permission::FETCH, shell_api::CATALOGUE)
            .map_err(|_| "account catalogue grant")?;
        env::pie::accord(
            watch,
            task,
            env::Permission::FETCH | env::Permission::ONLY,
            shell_api::CATALOGUE,
        )
        .map_err(|_| "account catalogue lifetime")?;
        let mut images = Vec::new();
        for image in &self.images {
            let seed = env::pie::accord(
                image.root,
                task,
                env::Permission::FETCH | env::Permission::VEST,
                crate::unit::IMAGE_MARK,
            )
            .map_err(|_| "account command image grant")?;
            images.push(Image {
                name: String::from(image.name),
                seed,
                length: image.length,
                ports: image
                    .ports
                    .iter()
                    .map(|(name, direction)| (String::from(*name), *direction))
                    .collect(),
            });
        }
        let bytes = Catalogue { images }
            .encode()
            .map_err(|_| "account catalogue encoding")?;
        resource::raw::Hole::from_raw(mailbox)
            .push(&bytes, Wait::POLL)
            .map_err(|_| "account catalogue delivery")?;
        self.deliveries.push(delivery);
        Ok(())
    }
    pub fn sweep(&mut self) {
        self.deliveries.retain(|delivery| {
            let held = matches!(env::mail::wait(delivery.watch, env::MailCondition::Pull, Wait::POLL), Err(error) if error.source == env::MailFail::HandedOver);
            let read = matches!(env::mail::wait(delivery.mailbox, env::MailCondition::Empty, Wait::POLL), Ok(true)); held && !read
        });
    }
}
fn receive(
    supervisor: TaskId,
    channel: env::Mark,
) -> Result<(Vec<u8>, ipc::session::establish::Held), &'static str> {
    let load = ipc::session::establish::Held(
        ipc::session::establish::endpoint(supervisor, channel, Wait::POLL)
            .map_err(|_| "account command channel")?,
    );
    let mut bytes = [0; crate::unit::ImageSupplyFrame::LEN];
    let (length, from) = resource::raw::Hole::from_raw(load.0.rx())
        .pull(&mut bytes, WAIT)
        .map_err(|_| "account command receive")?;
    let frame =
        crate::unit::ImageSupplyFrame::fetch(&bytes[..length]).ok_or("account command frame")?;
    let info = resource::raw::inspect(frame.seed).map_err(|_| "account command source")?;
    if from != supervisor
        || !info.alive
        || info.owner != supervisor
        || info.vestor != supervisor
        || info.mark != crate::unit::IMAGE_MARK
    {
        return Err("account command source");
    }
    let dock = resource::dock::Dock::open(frame.seed).map_err(|_| "account command mapping")?;
    let view = dock.view();
    let len = usize::try_from(frame.length).map_err(|_| "account command length")?;
    if len > view.size() {
        return Err("account command range");
    }
    // SAFETY: the read-only granted image is bounded by its held mapping.
    let bytes = unsafe { core::slice::from_raw_parts(view.base() as *const u8, len) }.to_vec();
    dock.shut().map_err(|_| "account command unmap")?;
    let _ = env::pie::release(frame.seed, env::ReleaseMode::Revoke);
    Ok((bytes, load))
}
