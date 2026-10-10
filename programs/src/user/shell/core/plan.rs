use alloc::{collections::BTreeSet, string::String, vec, vec::Vec};
pub use stream::Direction;
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct CommandId(pub u64);
#[derive(Clone, Debug)]
pub struct PortDecl {
    pub name: String,
    pub direction: Direction,
}
#[derive(Clone, Debug)]
pub struct Command {
    pub id: CommandId,
    pub image: String,
    pub args: Vec<String>,
    pub ports: Vec<PortDecl>,
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Endpoint {
    Command { id: CommandId, port: String },
    Port { id: u64 },
}
#[derive(Clone, Debug)]
pub struct Link {
    pub source: Endpoint,
    pub sink: Endpoint,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlanFail {
    Empty,
    Duplicate,
    Missing,
    Direction,
    Cycle,
    Full,
}
#[derive(Clone, Debug)]
pub struct Plan {
    pub commands: Vec<Command>,
    pub links: Vec<Link>,
}
impl Plan {
    pub fn connect(
        commands: Vec<Command>,
        links: Vec<Link>,
        port: impl Fn(u64) -> Option<Direction>,
    ) -> Result<Self, PlanFail> {
        if commands.is_empty() && links.is_empty() {
            return Err(PlanFail::Empty);
        }
        if commands.len() > 64 || links.len() > 128 {
            return Err(PlanFail::Full);
        }
        let mut ids = BTreeSet::new();
        let mut expected = BTreeSet::new();
        for command in &commands {
            if !ids.insert(command.id) {
                return Err(PlanFail::Duplicate);
            }
            for declaration in &command.ports {
                if declaration.name.is_empty()
                    || !expected.insert(Endpoint::Command {
                        id: command.id,
                        port: declaration.name.clone(),
                    })
                {
                    return Err(PlanFail::Duplicate);
                }
            }
        }
        let direction = |endpoint: &Endpoint| -> Result<(Direction, bool), PlanFail> {
            match endpoint {
                Endpoint::Port { id } => port(*id)
                    .map(|direction| (direction, true))
                    .ok_or(PlanFail::Missing),
                Endpoint::Command { id, port } => commands
                    .iter()
                    .find(|c| c.id == *id)
                    .and_then(|c| c.ports.iter().find(|p| p.name == *port))
                    .map(|p| (p.direction, false))
                    .ok_or(PlanFail::Missing),
            }
        };
        let mut used = BTreeSet::new();
        let mut incoming = vec![0usize; commands.len()];
        let mut outgoing = vec![Vec::new(); commands.len()];
        for link in &links {
            let (source, external) = direction(&link.source)?;
            if source
                != if external {
                    Direction::Read
                } else {
                    Direction::Write
                }
            {
                return Err(PlanFail::Direction);
            }
            let (sink, external) = direction(&link.sink)?;
            if sink
                != if external {
                    Direction::Write
                } else {
                    Direction::Read
                }
            {
                return Err(PlanFail::Direction);
            }
            if !used.insert(link.source.clone()) || !used.insert(link.sink.clone()) {
                return Err(PlanFail::Duplicate);
            }
            if let (Endpoint::Command { id: from, .. }, Endpoint::Command { id: to, .. }) =
                (&link.source, &link.sink)
            {
                let from = commands
                    .iter()
                    .position(|c| c.id == *from)
                    .ok_or(PlanFail::Missing)?;
                let to = commands
                    .iter()
                    .position(|c| c.id == *to)
                    .ok_or(PlanFail::Missing)?;
                outgoing[from].push(to);
                incoming[to] += 1;
            }
        }
        if !expected.iter().all(|endpoint| used.contains(endpoint)) {
            return Err(PlanFail::Missing);
        }
        let mut ready: Vec<usize> = incoming
            .iter()
            .enumerate()
            .filter_map(|(i, count)| (*count == 0).then_some(i))
            .collect();
        let mut visited = 0;
        while let Some(index) = ready.pop() {
            visited += 1;
            for &next in &outgoing[index] {
                incoming[next] -= 1;
                if incoming[next] == 0 {
                    ready.push(next);
                }
            }
        }
        if visited != commands.len() {
            return Err(PlanFail::Cycle);
        }
        Ok(Self { commands, links })
    }
}

impl Command {
    pub fn weight(&self) -> usize {
        core::mem::size_of::<Self>()
            + self.image.len()
            + self
                .args
                .iter()
                .map(|arg| arg.len() + core::mem::size_of::<String>())
                .sum::<usize>()
            + self
                .ports
                .iter()
                .map(|port| port.name.len() + core::mem::size_of::<PortDecl>())
                .sum::<usize>()
    }
}
impl Plan {
    pub fn weight(&self) -> usize {
        self.commands.iter().map(Command::weight).sum::<usize>()
            + self
                .links
                .iter()
                .map(|link| {
                    core::mem::size_of::<Link>()
                        + [&link.source, &link.sink]
                            .iter()
                            .map(|end| match end {
                                Endpoint::Command { port, .. } => port.len(),
                                _ => 0,
                            })
                            .sum::<usize>()
                })
                .sum::<usize>()
    }
}
