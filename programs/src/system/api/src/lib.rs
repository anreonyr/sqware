#![no_std]

pub mod identity;

#[mold::interface(id = "sqware.system.loader.v1")]
pub mod loader {
    use env::{PieToken, TaskId};
    pub use wire::OK;

    pub const DIR: &str = "svc/sys/loader";
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
        #[grant(code = 1, key = "build", legacy = "loader-entry-build")]
        Build,
    }

    #[requests]
    pub enum Wire {
        #[operation(code = 1, grant = Build, frame = Ask)]
        Build {
            image: PieToken,
            offset: u64,
            len: u64,
            stack: u64,
            count: u8,
            #[frame(count = count, fill = 0)]
            args: [u64; MAX_ARGS],
            back: PieToken,
        },
        #[operation(code = 2, grant = Build, frame = Claim)]
        Claim {
            task: TaskId,
            back: PieToken,
        },
    }

    #[reply]
    #[derive(Clone, Copy, Debug)]
    pub struct Said {
        pub status: u8,
        pub team: u64,
        pub task: TaskId,
    }

    #[derive(Clone, Copy, PartialEq, Eq, Debug, mold::WireCodes)]
    #[wire(fallback = Bad)]
    pub enum Fail {
        #[code(1)] Unknown,
        #[code(2)] BadImage,
        #[code(3)] Full,
        #[code(4)] NotReady,
        #[code(5)] Bad,
        #[code(6)] Denied,
    }
}
