//! Control-owned admission policy for the two construction inputs.

use env::TaskId;
use system_api::identity::Subject;

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Construction {
    pub selector: Selector,
    pub owner: Owner,
    pub identity: Identity,
    pub arguments: Arguments,
    pub stack: Range,
    pub max_image: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Selector {
    Unit(&'static str),
    Image,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Owner {
    Caller,
    Selected,
    Exact(TaskId),
    NamedUnit(&'static str),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Identity {
    Caller,
    Selected,
    Exact(Subject),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Arguments {
    pub min: u8,
    pub max: u8,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Range {
    pub min: u64,
    pub max: u64,
}

impl Construction {
    pub const fn unit(name: &'static str) -> Self {
        Self {
            selector: Selector::Unit(name),
            owner: Owner::Selected,
            identity: Identity::Selected,
            arguments: Arguments { min: 0, max: 64 },
            stack: Range {
                min: 0,
                max: 1 << 20,
            },
            max_image: 0,
        }
    }

    pub const fn image() -> Self {
        Self {
            selector: Selector::Image,
            owner: Owner::Caller,
            identity: Identity::Caller,
            arguments: Arguments { min: 0, max: 64 },
            stack: Range {
                min: 0,
                max: 1 << 20,
            },
            max_image: system_api::loader::MAX_IMAGE,
        }
    }

    pub const fn unit_owned_by(name: &'static str, owner: &'static str) -> Self {
        let mut policy = Self::unit(name);
        policy.owner = Owner::NamedUnit(owner);
        policy
    }

    pub fn valid(self) -> bool {
        self.arguments.min <= self.arguments.max
            && self.arguments.max as usize <= system_api::loader::MAX_ARGS
            && self.stack.min <= self.stack.max
            && self.stack.max <= 1 << 20
            && match self.selector {
                Selector::Unit(name) => system_api::operator::name::valid(name),
                Selector::Image => {
                    self.max_image > 0 && self.max_image <= system_api::loader::MAX_IMAGE
                }
            }
            && match self.owner {
                Owner::NamedUnit(name) => system_api::operator::name::valid(name),
                Owner::Caller | Owner::Selected | Owner::Exact(_) => true,
            }
    }
}
