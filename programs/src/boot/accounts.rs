use env::ledger::{args as boot_args, entry::Entries, manifest};
use env::{Name, PieToken};
use programs::debug;

pub struct Accounts {
    view: &'static [u8],
    entries: Entries<'static>,
}
impl Accounts {
    pub fn take() -> Option<Self> {
        let a = execution::boot::args::args();
        if a.len() < boot_args::LEN {
            return None;
        }
        // SAFETY: boot provides read-only mappings lasting for this team's lifetime.
        let view = unsafe {
            core::slice::from_raw_parts(a[boot_args::VIEW] as *const u8, a[boot_args::VIEW_LEN])
        };
        let ledger = unsafe {
            core::slice::from_raw_parts(a[boot_args::LEDGER] as *const u8, a[boot_args::LEDGER_LEN])
        };
        manifest::Entries::new(view)?;
        Some(Self {
            view,
            entries: Entries::new(ledger)?,
        })
    }
    pub fn view(&self) -> &'static [u8] {
        self.view
    }
    pub fn token(&self, name: Name) -> Option<PieToken> {
        self.entries.find(name).map(|entry| entry.token())
    }
    pub fn report(&self) {
        let (mut pages, mut traps, mut calls) = (0, 0, 0);
        for entry in self.entries.iter() {
            match entry.name() {
                Name::Page(_) => pages += 1,
                Name::Trap(_) => traps += 1,
                Name::Call(_) => calls += 1,
            }
        }
        debug!("boot: resources page={pages} trap={traps} call={calls}");
    }
}
