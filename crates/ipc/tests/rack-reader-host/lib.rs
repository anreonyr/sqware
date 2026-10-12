#![allow(dead_code)]

extern crate self as env;
extern crate self as resource;

pub use abi::{
    Bit, MailCondition, MailFail, MailResult, PieFail, PieResult, PieToken, Source, TaskId, Wait,
    make_fail,
};

pub mod chrono {
    pub fn clock() -> u64 {
        crate::test_backend::now()
    }
}

pub mod dock {
    use crate::{PieResult, PieToken};

    #[derive(Clone, Copy)]
    pub struct View(usize);
    impl View {
        pub fn new(base: usize) -> Self {
            Self(base)
        }
        pub fn base(self) -> usize {
            self.0
        }
    }

    pub struct Dock(View);
    impl Dock {
        pub fn open(_: PieToken) -> PieResult<Self> {
            Ok(Self(View::new(0)))
        }
        pub fn view(&self) -> View {
            self.0
        }
    }
}

mod test_backend {
    use super::Wait;
    use std::{
        cell::{Cell, RefCell},
        collections::VecDeque,
        vec::Vec,
    };

    #[derive(Clone)]
    pub struct Step {
        pub advance_ns: u64,
        pub ready: bool,
        pub push: Option<Vec<u8>>,
    }

    thread_local! {
        static NOW: Cell<u64> = const { Cell::new(0) };
        static QUEUE: RefCell<VecDeque<Vec<u8>>> = const { RefCell::new(VecDeque::new()) };
        static STEPS: RefCell<VecDeque<Step>> = const { RefCell::new(VecDeque::new()) };
        static WAITS: RefCell<Vec<Wait>> = const { RefCell::new(Vec::new()) };
    }

    pub fn reset(script: impl IntoIterator<Item = Step>) {
        NOW.with(|now| now.set(0));
        QUEUE.with(|queue| queue.borrow_mut().clear());
        STEPS.with(|steps| *steps.borrow_mut() = script.into_iter().collect());
        WAITS.with(|waits| waits.borrow_mut().clear());
    }
    pub fn now() -> u64 {
        NOW.with(Cell::get)
    }
    pub fn waits() -> Vec<Wait> {
        WAITS.with(|waits| waits.borrow().clone())
    }
    pub fn pop_frame() -> Option<Vec<u8>> {
        QUEUE.with(|queue| queue.borrow_mut().pop_front())
    }
    pub fn wait(wait: Wait) -> bool {
        WAITS.with(|waits| waits.borrow_mut().push(wait));
        let step = STEPS
            .with(|steps| steps.borrow_mut().pop_front())
            .expect("scripted wait step");
        NOW.with(|now| now.set(now.get().saturating_add(step.advance_ns)));
        if let Some(frame) = step.push {
            QUEUE.with(|queue| queue.borrow_mut().push_back(frame));
        }
        step.ready
    }
}

mod bell {
    use crate::{Bit, MailResult, PieToken, Source, Wait, test_backend};

    pub(crate) struct Bell;
    impl Bell {
        pub(crate) fn from_raw(_: PieToken, _: Bit) -> Self {
            Self
        }
        pub(crate) fn token(&self) -> PieToken {
            PieToken::NONE
        }
        pub(crate) fn source(&self) -> Source {
            Source::Mail {
                pie: PieToken::NONE,
                condition: crate::MailCondition::Signal(Bit::FIRST),
            }
        }
        pub(crate) fn ring(&self) -> MailResult<()> {
            Ok(())
        }
        pub(crate) fn hush(&self) -> MailResult<()> {
            Ok(())
        }
        pub(crate) fn wait(&self, wait: Wait) -> MailResult<bool> {
            Ok(test_backend::wait(wait))
        }
    }
}

mod ring {
    use crate::{dock::View, test_backend};
    use wire::Message;

    pub(crate) const SLOT: usize = 512;
    pub(crate) struct Ring;
    static RING: Ring = Ring;
    pub(crate) const fn exact<M: Message>() {}
    pub(crate) fn ring(_: View) -> &'static Ring {
        &RING
    }
    pub(crate) fn cursor(_: &Ring) -> u64 {
        1
    }
    pub(crate) fn lost(_: &Ring) -> u64 {
        0
    }
    pub(crate) fn pop(
        _: &Ring,
        next: &mut u64,
        _: &mut u64,
        out: &mut [u8],
    ) -> Result<Option<usize>, usize> {
        let Some(frame) = test_backend::pop_frame() else {
            return Ok(None);
        };
        if frame.len() > out.len() {
            *next = next.saturating_add(1);
            return Err(frame.len());
        }
        out[..frame.len()].copy_from_slice(&frame);
        *next = next.saturating_add(1);
        Ok(Some(frame.len()))
    }
}

const SPACE_BIT: Bit = Bit::FIRST;
#[path = "../../src/rack/reader.rs"]
mod reader;
#[path = "../../src/time.rs"]
mod time;

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec::Vec;
    use wire::Message;

    struct ByteMessage;
    impl Message for ByteMessage {
        type In = u8;
        type Buf = [u8; 1];
        const EMPTY: Self::Buf = [0];
        fn store(&self, out: &mut [u8]) -> Option<usize> {
            *out.first_mut()? = 1;
            Some(1)
        }
        fn fetch(bytes: &[u8]) -> Option<u8> {
            (bytes.len() == 1).then(|| bytes[0])
        }
    }

    fn reader() -> reader::Reader<ByteMessage> {
        reader::Reader::of(dock::View::new(0), PieToken::NONE)
    }

    fn step(ms: u64, ready: bool, push: Option<Vec<u8>>) -> test_backend::Step {
        test_backend::Step {
            advance_ns: ms * 1_000_000,
            ready,
            push,
        }
    }

    #[test]
    fn early_false_wake_does_not_end_recv_before_data_arrives() {
        test_backend::reset([step(50, false, None), step(900, false, Some(vec![42]))]);
        let mut reader = reader();
        assert_eq!(reader.recv(Wait::AtMost(1000)).unwrap(), 42);
        assert_eq!(
            test_backend::waits(),
            [Wait::AtMost(1000), Wait::AtMost(950)]
        );
        assert_eq!(test_backend::now(), 950_000_000);
    }

    #[test]
    fn false_wakes_keep_one_deadline_and_stop_at_expiry() {
        test_backend::reset([step(50, false, None), step(950, false, None)]);
        let mut reader = reader();
        assert!(matches!(
            reader.recv(Wait::AtMost(1000)),
            Err(reader::RecvFail::Empty)
        ));
        assert_eq!(
            test_backend::waits(),
            [Wait::AtMost(1000), Wait::AtMost(950)]
        );
        assert_eq!(test_backend::now(), 1_000_000_000);
    }

    #[test]
    fn poll_never_enters_wait_after_its_ring_checks() {
        test_backend::reset([]);
        let mut reader = reader();
        assert!(matches!(
            reader.recv(Wait::POLL),
            Err(reader::RecvFail::Empty)
        ));
        assert!(test_backend::waits().is_empty());
    }
}
