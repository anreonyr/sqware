use crate::unit::Binding;
use pipe_api::Direction;

pub struct Command {
    pub name: &'static str,
    pub binding: Binding,
    pub ports: &'static [(&'static str, Direction)],
}

const CAT_PORTS: &[(&str, Direction)] = &[("source", Direction::Read), ("copy", Direction::Write)];
const EMIT_PORTS: &[(&str, Direction)] = &[("records", Direction::Write)];
const UPPER_PORTS: &[(&str, Direction)] =
    &[("source", Direction::Read), ("result", Direction::Write)];
const EMPTY: &[(&str, Direction)] = &[];
const WORKERS_PORTS: &[(&str, Direction)] = &[("ticks", Direction::Write)];

const fn binding(resource: &'static str, slot: &'static str) -> Binding {
    Binding {
        source: system_api::supply::Key::of("system/initrd"),
        resource: system_api::supply::Key::of(resource),
        slot: env::Mark::of(slot),
        permission: env::Permission::FETCH,
    }
}

pub(crate) const CAT: Command = Command {
    name: "cat",
    binding: binding("cat", "shell-cat"),
    ports: CAT_PORTS,
};
pub(crate) const EMIT: Command = Command {
    name: "emit",
    binding: binding("emit", "shell-emit"),
    ports: EMIT_PORTS,
};
pub(crate) const UPPER: Command = Command {
    name: "upper",
    binding: binding("upper", "shell-upper"),
    ports: UPPER_PORTS,
};
pub(crate) const FAIL: Command = Command {
    name: "fail",
    binding: binding("fail", "shell-fail"),
    ports: EMPTY,
};
pub(crate) const SPIN: Command = Command {
    name: "spin",
    binding: binding("spin", "shell-spin"),
    ports: EMPTY,
};
pub(crate) const WORKERS: Command = Command {
    name: "workers",
    binding: binding("workers", "shell-workers"),
    ports: WORKERS_PORTS,
};
pub const COMMANDS: &[Command] = &[CAT, EMIT, UPPER, FAIL, SPIN, WORKERS];
pub const BINDINGS: &[Binding] = &[
    CAT.binding,
    EMIT.binding,
    UPPER.binding,
    FAIL.binding,
    SPIN.binding,
    WORKERS.binding,
];
