#![allow(dead_code)]
extern crate env as abi_env;

#[cfg(test)]
mod tests {
    use env::PieToken;
    use wire::message::Message;

    fn token(value: u64) -> PieToken {
        PieToken::from_bytes(&value.to_le_bytes()).unwrap()
    }

    #[test]
    fn typed_contracts_extract_return_tokens_for_all_control_faces() {
        let back = token(0x1234);
        let account = account_api::Request {
            op: account_api::CREATE,
            id: system_api::control::construction::CreateId { value: 1 },
            account: String::from("."),
            back,
        };
        let mut bytes = account_api::Request::EMPTY;
        let n = account.store(&mut bytes).unwrap();
        let decoded = account_api::Request::fetch(&bytes[..n]).unwrap();
        assert_eq!(account_api::Call::back(&decoded), back);
        assert!(
            decoded.1,
            "well-formed trailing boundary is retained for server validation"
        );
        assert_eq!(account_api::Call::BACK, account_api::BACK,);
        let decoded_with_tail = account_api::Request::fetch(&bytes[..n + 1]).unwrap();
        assert!(!decoded_with_tail.1);
        assert_eq!(account_api::Call::back(&decoded_with_tail), back);
    }
}
