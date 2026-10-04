//! Static local account verifier; authentication material stays in Login.
use argon2::{Argon2, PasswordHash, PasswordVerifier};
pub const ACCOUNT: &str = "anran";
const VERIFIER: &str = "$argon2id$v=19$m=19456,t=2,p=1$c3F3YXJlLWFucmFuLXYxIQ$bhc/b4L3SCjUpCf71KjiSpoAQqK5zzB2yq+FK9ikois";
pub fn verify(account: &[u8], password: &[u8]) -> bool {
    let Ok(hash) = PasswordHash::new(VERIFIER) else {
        return false;
    };
    let password_matches = Argon2::default().verify_password(password, &hash).is_ok();
    account == ACCOUNT.as_bytes() && password_matches
}
