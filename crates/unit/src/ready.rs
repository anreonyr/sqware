#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ready {
    OnSpawn,
    Signal(env::Mark),
}

pub const READY_MARK: env::Mark = env::Mark::of("ready");
