use crate::common::path::Path;
pub use system_api::loader::{
    Ask, Claim, Said, Wire, Fail, code_to_fail, fail_to_code,
    BACK, IMAGE, BUILD, CLAIM, CLAIM_MS, MAX_ARGS, MAX_IMAGE,
};
pub const DIR: &Path = Path::new(system_api::loader::DIR);
