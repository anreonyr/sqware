#![allow(dead_code)]
extern crate alloc;
#[path = "../../../common/path.rs"]
pub mod path;
#[path = "../../../wire/message.rs"]
pub mod message;
#[path = "../../../wire/ok.rs"]
pub mod ok;
#[path = "../frame.rs"]
pub mod frame;
#[path = "../grant.rs"]
pub mod grant;
mod common {
    pub use crate::path;
}

#[cfg(test)]
mod generation;

#[cfg(test)]
mod tests {
    use super::{frame::*, grant::*, message::Message, ok::OK};
    use env::{PieToken, TaskId, wire::Span};

    // Fixed little-endian samples independent of the codec under test.
    const BUILD_BYTES: &[u8] = &[
        1,
        8, 7, 6, 5, 4, 3, 2, 1,
        24, 23, 22, 21, 20, 19, 18, 17,
        40, 39, 38, 37, 36, 35, 34, 33,
        56, 55, 54, 53, 52, 51, 50, 49,
        2,
        72, 71, 70, 69, 68, 67, 66, 65,
        88, 87, 86, 85, 84, 83, 82, 81,
        104, 103, 102, 101, 100, 99, 98, 97,
    ];
    const CLAIM_BYTES: &[u8] = &[
        2, 8, 7, 6, 5, 4, 3, 2, 1, 104, 103, 102, 101, 100, 99, 98, 97,
    ];
    const REPLY_BYTES: &[u8] = &[
        0, 24, 23, 22, 21, 20, 19, 18, 17, 8, 7, 6, 5, 4, 3, 2, 1,
    ];
    fn token(raw: u64) -> PieToken {
        PieToken::from_bytes(&raw.to_le_bytes()).unwrap()
    }
    fn build() -> Ask {
        let mut args = [0; 64];
        args[0] = 0x4142434445464748;
        args[1] = 0x5152535455565758;
        Ask {
            op: 1, image: token(0x0102030405060708),
            offset: 0x1112131415161718, len: 0x2122232425262728,
            stack: 0x3132333435363738, count: 2, args,
            back: token(0x6162636465666768),
        }
    }

    #[test]
    fn identifiers_limits_and_marks_are_fixed() {
        assert_eq!((BUILD, CLAIM, OK), (1, 2, 0));
        assert_eq!((MAX_ARGS, MAX_IMAGE, CLAIM_MS), (64, 16_777_216, 3000));
        assert_eq!((Ask::LEN, Claim::LEN, Said::LEN), (554, 17, 17));
        assert_eq!(DIR.as_str(), "svc/sys/loader");
        assert_eq!(BACK.get(), 0xc1bc7bad2c8f22ca);
        assert_eq!(IMAGE.get(), 0x280b1d49733b5ae2);
        assert_eq!(Grant::Build.mark().get(), 0xa1c2ab202871949a);
        assert_eq!(Grant::Build.at(), 1);
        assert_eq!(Grant::from_action(1), Some(Grant::Build));
        assert_eq!(Grant::from_action(0), None);
        assert_eq!(Grant::from_action(2), None);
        assert_eq!(grant_of(env::Mark::new(0xa1c2ab202871949a)), Some(Grant::Build));
        assert_eq!(grant_of(BACK), None);
        assert_eq!(grant_of(IMAGE), None);
    }

    #[test]
    fn error_codes_and_unknown_fallback_are_fixed() {
        use Fail::*;
        assert_eq!(fail_to_code(None), 0);
        assert_eq!(code_to_fail(0), None);
        for (code, fail) in [(1, Unknown), (2, BadImage), (3, Full),
            (4, NotReady), (5, Bad), (6, Denied)] {
            assert_eq!(fail_to_code(Some(fail)), code);
            assert_eq!(code_to_fail(code), Some(fail));
        }
        for code in 7..=255 {
            assert_eq!(code_to_fail(code), Some(Bad));
        }
    }

    #[test]
    fn build_matches_fixed_bytes_in_both_directions() {
        let mut out = [0; 554];
        let mut ask = build();
        ask.args[2..].fill(u64::MAX);
        let n = ask.store_at(&mut out, 0).unwrap();
        assert_eq!(n, 58);
        assert_eq!(&out[..n], BUILD_BYTES);
        let Some(Wire::Build(decoded)) = Wire::take(BUILD_BYTES) else { panic!("build"); };
        assert_eq!(decoded.image.get(), 0x0102030405060708);
        assert_eq!(decoded.offset, 0x1112131415161718);
        assert_eq!(decoded.len, 0x2122232425262728);
        assert_eq!(decoded.stack, 0x3132333435363738);
        assert_eq!(decoded.count, 2);
        assert_eq!(decoded.args, build().args);
        assert_eq!(decoded.back.get(), 0x6162636465666768);
        assert_eq!(Grant::for_wire(&Wire::Build(decoded)), Grant::Build);
    }

    #[test]
    fn claim_matches_fixed_bytes_and_shares_build_grant() {
        let claim = Claim { op: 2, task: TaskId::new(0x0102030405060708),
            back: token(0x6162636465666768) };
        let mut out = [0; 17];
        assert_eq!(claim.store_at(&mut out, 0), Some(17));
        assert_eq!(out.as_slice(), CLAIM_BYTES);
        let Some(Wire::Claim(decoded)) = Wire::take(CLAIM_BYTES) else { panic!("claim"); };
        assert_eq!(decoded.task.get(), 0x0102030405060708);
        assert_eq!(decoded.back.get(), 0x6162636465666768);
        assert_eq!(Grant::for_wire(&Wire::Claim(decoded)), Grant::Build);
    }

    #[test]
    fn replies_match_fixed_bytes_for_success_and_errors() {
        for status in 0..=6 {
            let said = Said { status, team: 0x1112131415161718,
                task: TaskId::new(0x0102030405060708) };
            let mut expected = REPLY_BYTES.to_vec();
            expected[0] = status;
            let mut out = [0; 17];
            assert_eq!(said.store(&mut out), Some(17));
            assert_eq!(out.as_slice(), expected);
            let decoded = Said::fetch(&expected).unwrap();
            assert_eq!((decoded.status, decoded.team, decoded.task),
                (status, said.team, said.task));
        }
        let mut failure = [0; 17];
        failure[0] = 4;
        let said = Said { status: 4, team: 0, task: TaskId::new(0) };
        let mut out = [0; 17];
        assert_eq!(said.store(&mut out), Some(17));
        assert_eq!(out, failure);
        assert_eq!(Said::fetch(&failure).unwrap().task.get(), 0);
    }

    #[test]
    fn requests_and_replies_reject_truncation_and_trailing_bytes() {
        for sample in [BUILD_BYTES, CLAIM_BYTES] {
            for len in 0..sample.len() {
                assert!(Wire::take(&sample[..len]).is_none(), "request length {len}");
            }
            let mut extra = sample.to_vec();
            extra.push(0);
            assert!(Wire::take(&extra).is_none());
        }
        for len in 0..REPLY_BYTES.len() {
            assert!(Said::fetch(&REPLY_BYTES[..len]).is_none());
        }
        let mut extra = REPLY_BYTES.to_vec();
        extra.push(0);
        assert!(Said::fetch(&extra).is_none());
        assert!(build().store_at(&mut [0; 57], 0).is_none());
    }

    #[test]
    fn unknown_request_operations_are_rejected() {
        for op in [0, 3, 127, 255] {
            for sample in [BUILD_BYTES, CLAIM_BYTES] {
                let mut bytes = sample.to_vec();
                bytes[0] = op;
                assert!(Wire::take(&bytes).is_none());
            }
        }
    }

    #[test]
    fn generated_dispatch_encoding_checks_the_selected_operation() {
        let mut out = [0; 554];
        let n = Wire::Build(build()).store(&mut out).unwrap();
        assert_eq!(&out[..n], BUILD_BYTES);
        let mut mismatched = build();
        mismatched.op = 2;
        assert!(Wire::Build(mismatched).store(&mut out).is_none());
        let claim = Claim { op: 2, task: TaskId::new(0x0102030405060708),
            back: token(0x6162636465666768) };
        let n = Wire::Claim(claim).store(&mut out).unwrap();
        assert_eq!(&out[..n], CLAIM_BYTES);
    }

    #[test]
    fn argument_count_boundaries_preserve_layout() {
        for count in [0, 64] {
            let mut ask = build();
            ask.count = count;
            for (i, arg) in ask.args.iter_mut().enumerate() { *arg = i as u64; }
            let mut expected = BUILD_BYTES[..34].to_vec();
            expected[33] = count;
            for i in 0..count { expected.extend_from_slice(&(i as u64).to_le_bytes()); }
            expected.extend_from_slice(&[104, 103, 102, 101, 100, 99, 98, 97]);
            let mut out = [0; 554];
            let n = ask.store_at(&mut out, 0).unwrap();
            assert_eq!(n, 42 + count as usize * 8);
            assert_eq!(&out[..n], expected);
            let Some(Wire::Build(decoded)) = Wire::take(&expected) else { panic!("count"); };
            assert_eq!(decoded.count, count);
            assert_eq!(&decoded.args[..count as usize], &ask.args[..count as usize]);
            assert!(decoded.args[count as usize..].iter().all(|&arg| arg == 0));
        }
        let mut ask = build();
        ask.count = 65;
        assert!(ask.store_at(&mut [0; 554], 0).is_none());
        let mut oversized = vec![0; 562];
        oversized[0] = 1;
        oversized[33] = 65;
        assert!(Wire::take(&oversized).is_none());
    }
}
