//! Tagged request/reply dispatch; field layout belongs to Frame and Span.

use super::super::grant::Grant;
use super::{
    MAX_FRAME, OK,
    data::{Back, Optional, Task, valid_selector},
    vocab::*,
};
use crate::wire::message::Message;
use env::{PieToken, wire::Span};

#[derive(env::Frame)]
struct Header {
    back: Back,
    action: u8,
}
#[derive(env::Frame)]
struct ReplyHeader {
    status: u8,
    kind: u8,
}
#[derive(env::Frame)]
struct Restricted {
    parent: Task,
    subject: Subject,
}

fn put<T: Span>(value: &T, bytes: &mut [u8], at: &mut usize) -> Option<()> {
    *at = value.store_at(bytes, *at)?;
    Some(())
}
fn get<T: Span>(bytes: &[u8], at: &mut usize) -> Option<T> {
    let (value, next) = T::fetch_at(bytes, *at)?;
    *at = next;
    Some(value)
}

impl Span for Install {
    const MAX: Option<usize> = env::wire::total(&[<u8 as Span>::MAX, Some(Restricted::LEN)]);
    fn store_at(&self, bytes: &mut [u8], mut at: usize) -> Option<usize> {
        match *self {
            Self::Authorized(subject) => {
                put(&0u8, bytes, &mut at)?;
                put(&subject, bytes, &mut at)?;
            }
            Self::Inherit { parent } => {
                put(&1u8, bytes, &mut at)?;
                put(&Task::new(parent)?, bytes, &mut at)?;
            }
            Self::Restrict { parent, subject } => {
                put(&2u8, bytes, &mut at)?;
                at = Restricted {
                    parent: Task::new(parent)?,
                    subject,
                }
                .store_at(bytes, at)?;
            }
        }
        Some(at)
    }
    fn fetch_at(bytes: &[u8], mut at: usize) -> Option<(Self, usize)> {
        let install = match get::<u8>(bytes, &mut at)? {
            0 => Self::Authorized(get(bytes, &mut at)?),
            1 => Self::Inherit {
                parent: get::<Task>(bytes, &mut at)?.0,
            },
            2 => {
                let (frame, next) = Restricted::fetch_at(bytes, at)?;
                at = next;
                Self::Restrict {
                    parent: frame.parent.0,
                    subject: frame.subject,
                }
            }
            _ => return None,
        };
        Some((install, at))
    }
}

impl Wire {
    pub fn store(self, back: PieToken, bytes: &mut [u8]) -> Option<usize> {
        self.validate()?;
        if back.get() == 0 {
            return None;
        }
        let mut at = Header {
            back: Back(back),
            action: Grant::of_wire(&self),
        }
        .store_at(bytes, 0)?;
        match self {
            Self::Resolve(t) | Self::Unbind(t) => put(&Task::new(t)?, bytes, &mut at)?,
            Self::Matches(t, selector) => {
                valid_selector(selector)?;
                put(&Task::new(t)?, bytes, &mut at)?;
                put(&selector, bytes, &mut at)?;
            }
            Self::Same(a, b) => {
                put(&Task::new(a)?, bytes, &mut at)?;
                put(&Task::new(b)?, bytes, &mut at)?;
            }
            Self::Sire(p) | Self::Derive(p) => put(&p, bytes, &mut at)?,
            Self::Heir(a, b) => {
                put(&a, bytes, &mut at)?;
                put(&b, bytes, &mut at)?;
            }
            Self::Amid(p, c) => {
                put(&p, bytes, &mut at)?;
                put(&c, bytes, &mut at)?;
            }
            Self::Members(c, cursor) => {
                put(&c, bytes, &mut at)?;
                put(&Optional(cursor), bytes, &mut at)?;
            }
            Self::Memberships(p, cursor) => {
                put(&p, bytes, &mut at)?;
                put(&Optional(cursor), bytes, &mut at)?;
            }
            Self::Adopt(subject) | Self::Restrict(subject) => put(&subject, bytes, &mut at)?,
            Self::Admit(c, p) | Self::Expel(c, p) => {
                put(&c, bytes, &mut at)?;
                put(&p, bytes, &mut at)?;
            }
            Self::Bind(task, install) => {
                put(&Task::new(task)?, bytes, &mut at)?;
                put(&install, bytes, &mut at)?;
            }
            Self::Waive | Self::Found => {}
        }
        (at <= MAX_FRAME).then_some(at)
    }

    /// An intact header with a malformed/unknown action keeps the caller's reply route.
    pub fn take(bytes: &[u8]) -> Option<(Option<Self>, PieToken)> {
        if bytes.len() > MAX_FRAME {
            return None;
        }
        let (header, mut at) = Header::fetch_at(bytes, 0)?;
        if header.back.0.get() == 0 {
            return None;
        }
        let decoded = (|| {
            let wire = match Grant::from_action(header.action)? {
                Grant::Resolve => Self::Resolve(get::<Task>(bytes, &mut at)?.0),
                Grant::Matches => {
                    Self::Matches(get::<Task>(bytes, &mut at)?.0, get(bytes, &mut at)?)
                }
                Grant::Same => Self::Same(
                    get::<Task>(bytes, &mut at)?.0,
                    get::<Task>(bytes, &mut at)?.0,
                ),
                Grant::Sire => Self::Sire(get(bytes, &mut at)?),
                Grant::Heir => Self::Heir(get(bytes, &mut at)?, get(bytes, &mut at)?),
                Grant::Amid => Self::Amid(get(bytes, &mut at)?, get(bytes, &mut at)?),
                Grant::Members => Self::Members(
                    get(bytes, &mut at)?,
                    get::<Optional<Cursor>>(bytes, &mut at)?.0,
                ),
                Grant::Memberships => Self::Memberships(
                    get(bytes, &mut at)?,
                    get::<Optional<Cursor>>(bytes, &mut at)?.0,
                ),
                Grant::Adopt => Self::Adopt(get(bytes, &mut at)?),
                Grant::Waive => Self::Waive,
                Grant::Restrict => Self::Restrict(get(bytes, &mut at)?),
                Grant::Derive => Self::Derive(get(bytes, &mut at)?),
                Grant::Found => Self::Found,
                Grant::Admit => Self::Admit(get(bytes, &mut at)?, get(bytes, &mut at)?),
                Grant::Expel => Self::Expel(get(bytes, &mut at)?, get(bytes, &mut at)?),
                Grant::Bind => Self::Bind(get::<Task>(bytes, &mut at)?.0, get(bytes, &mut at)?),
                Grant::Unbind => Self::Unbind(get::<Task>(bytes, &mut at)?.0),
            };
            wire.validate()?;
            (at == bytes.len()).then_some(wire)
        })();
        Some((decoded, header.back.0))
    }
}

pub struct Request(pub Wire, pub PieToken);

impl Message for Request {
    type In = (Option<Wire>, PieToken);
    type Buf = [u8; MAX_FRAME];
    const EMPTY: Self::Buf = [0; MAX_FRAME];

    fn store(&self, bytes: &mut [u8]) -> Option<usize> {
        self.0.store(self.1, bytes)
    }
    fn fetch(bytes: &[u8]) -> Option<Self::In> {
        Wire::take(bytes)
    }
}

impl Message for Reply {
    type In = Self;
    type Buf = [u8; MAX_FRAME];
    const EMPTY: Self::Buf = [0; MAX_FRAME];

    fn store(&self, bytes: &mut [u8]) -> Option<usize> {
        if let Self::Fail(fail) = *self {
            return fail_to_code(Some(fail)).store_at(bytes, 0);
        }
        let kind = match self {
            Self::Unit => 0,
            Self::Binding(_) => 1,
            Self::Principal(_) => 2,
            Self::Coalition(_) => 3,
            Self::Match(_) => 4,
            Self::Bool(_) => 5,
            Self::Members(_) => 6,
            Self::Memberships(_) => 7,
            Self::Fail(_) => return None,
        };
        let mut at = ReplyHeader { status: OK, kind }.store_at(bytes, 0)?;
        match *self {
            Self::Unit => {}
            Self::Binding(value) => put(&Optional(value), bytes, &mut at)?,
            Self::Principal(value) => put(&Optional(value), bytes, &mut at)?,
            Self::Coalition(value) => put(&value, bytes, &mut at)?,
            Self::Match(value) => put(&value, bytes, &mut at)?,
            Self::Bool(value) => put(&value, bytes, &mut at)?,
            Self::Members(value) => put(&value, bytes, &mut at)?,
            Self::Memberships(value) => put(&value, bytes, &mut at)?,
            Self::Fail(_) => return None,
        }
        (at <= MAX_FRAME).then_some(at)
    }

    fn fetch(bytes: &[u8]) -> Option<Self> {
        if bytes.len() > MAX_FRAME {
            return None;
        }
        let (status, end) = u8::fetch_at(bytes, 0)?;
        if status != OK {
            let fail = code_to_fail(status)?;
            return (end == bytes.len()).then_some(Self::Fail(fail));
        }
        let (header, mut at) = ReplyHeader::fetch_at(bytes, 0)?;
        let reply = match header.kind {
            0 => Self::Unit,
            1 => Self::Binding(get::<Optional<Binding>>(bytes, &mut at)?.0),
            2 => Self::Principal(get::<Optional<PrincipalId>>(bytes, &mut at)?.0),
            3 => Self::Coalition(get(bytes, &mut at)?),
            4 => Self::Match(get(bytes, &mut at)?),
            5 => Self::Bool(get(bytes, &mut at)?),
            6 => Self::Members(get(bytes, &mut at)?),
            7 => Self::Memberships(get(bytes, &mut at)?),
            _ => return None,
        };
        (at == bytes.len()).then_some(reply)
    }
}
