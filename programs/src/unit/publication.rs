#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PublishScope {
    Driver,
    Hub,
    Fixture,
    Terminal,
}

#[derive(Clone, Copy)]
pub struct PublishEntry {
    pub name: &'static str,
}

impl PublishEntry {
    pub(crate) const fn from_names<const N: usize>(names: [&'static str; N]) -> [Self; N] {
        let mut entries = [Self { name: "" }; N];
        let mut index = 0;
        while index < N {
            entries[index] = Self { name: names[index] };
            index += 1;
        }
        entries
    }
}

#[derive(Clone, Copy)]
pub enum Publish {
    Entries {
        scope: PublishScope,
        group: &'static str,
        road: &'static str,
        entries: &'static [PublishEntry],
        public: bool,
    },
    Namespace {
        scope: PublishScope,
        group: &'static str,
        road: &'static str,
        public: bool,
    },
    Devices,
}
