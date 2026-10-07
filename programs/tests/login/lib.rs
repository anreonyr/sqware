#![allow(dead_code)]
extern crate alloc;
#[cfg(test)]
#[path = "../../src/user/login/auth.rs"]
mod auth;
#[cfg(test)]
mod frame { pub use system_api::control::account::Request; }
#[cfg(test)]
mod tests {
    use crate::frame::Request;
    use env::PieToken;
    use env::wire::Span as _;
    #[test]
    fn account_password_requires_both_matching() {
        assert!(super::auth::verify(b"anran", b"sqware"));
        assert!(!super::auth::verify(b"anran", b"wrong"));
        assert!(!super::auth::verify(b"unknown", b"sqware"));
        assert!(!super::auth::verify(b"anran", b""));
    }
    #[test]
    fn requests_reject_invalid_shapes_and_trailing_bytes() {
        let cases = [
            ("anran", true),
            ("", false),
            ("../anran", false),
            ("other", true),
        ];
        for (account, valid) in cases {
            let request = Request {
                account: account.into(),
                back: PieToken::NONE,
            };
            let mut bytes = [0; Request::LEN + 1];
            let n = request.store_at(&mut bytes, 0).unwrap();
            assert_eq!(Request::take(&bytes[..n]).is_some(), valid);
            assert!(Request::take(&bytes[..n + 1]).is_none());
            for end in 0..n {
                assert!(Request::take(&bytes[..end]).is_none());
            }
        }
    }
}
