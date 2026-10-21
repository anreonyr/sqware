#![no_std]

extern crate alloc;

pub mod control;
pub mod identity;
pub mod namespace;
pub mod operator;
pub mod retirement;
pub mod supply;

#[mold::interface(id = "sqware.system.loader.v1")]
pub mod loader {
    use env::{PieToken, TaskId};
    #[derive(Clone, Copy, Debug)]
    pub struct Built {
        pub team: env::TeamId,
        pub task: TaskId,
    }
    #[derive(Clone, Copy, Debug, env::Frame)]
    #[frame(len = 8)]
    pub struct StartupMetadata {
        /// Owner-local Resolve carrier already accorded by System to the instance owner.
        pub resolve: PieToken,
    }
    pub use wire::OK;

    pub const DIR: &str = "loader";
    pub const CLAIM_MS: usize = 3000;
    pub const MAX_ARGS: usize = 64;
    pub const MAX_IMAGE: usize = 16 * 1024 * 1024;

    #[channels]
    pub enum Channel {
        #[channel(key = "reply", legacy = "loader-back")]
        Back,
        #[channel(key = "image", legacy = "loader-image")]
        Image,
    }

    #[grants]
    pub enum Grant {
        #[grant(code = 1, key = "claim", legacy = "loader-entry-claim")]
        Claim,
    }

    #[requests]
    pub enum Wire {
        #[operation(code = 1, grant = Claim, frame = Claim)]
        Claim { task: TaskId, back: PieToken },
    }

    #[reply]
    #[derive(Clone, Copy, Debug)]
    pub struct Said {
        pub status: u8,
        pub state: u8,
        pub team: u64,
        pub task: TaskId,
        pub startup: StartupMetadata,
    }

    impl Said {
        pub fn from_result(result: Result<Built, crate::control::Fail>) -> Self {
            match result {
                Ok(built) => Self {
                    status: wire::OK,
                    state: crate::control::construction::COMPLETED,
                    task: built.task,
                    team: built.team.get() as u64,
                    startup: StartupMetadata {
                        resolve: PieToken::NONE,
                    },
                },
                Err(fail) => Self {
                    status: crate::control::frame::fail_to_code(Some(fail)),
                    state: crate::control::construction::FAILED,
                    task: TaskId::new(0),
                    team: 0,
                    startup: StartupMetadata {
                        resolve: PieToken::NONE,
                    },
                },
            }
        }
    }

    #[derive(Clone, Copy, PartialEq, Eq, Debug, mold::WireCodes)]
    #[wire(fallback = Bad)]
    pub enum Fail {
        #[code(1)]
        Unknown,
        #[code(2)]
        BadImage,
        #[code(3)]
        Full,
        #[code(4)]
        NotReady,
        #[code(5)]
        Bad,
        #[code(6)]
        Denied,
    }
    fn reply_to(request: &Wire) -> PieToken {
        match request {
            Wire::Claim(claim) => claim.back,
        }
    }

    #[mold::contract(request = Wire, response = Said, mark = BACK, back = reply_to)]
    pub struct Call;
}
