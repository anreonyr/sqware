#![allow(dead_code)]
extern crate alloc;

#[cfg(test)]
mod tests {
    use account_api::Request as AccountRequest;
    use alloc::string::String;
    use env::{Mark, PieToken, TaskId};
    use system_api::control::{
        Grant,
        frame::{self, Fail, Req, Request, Said, State, Wire},
    };
    use wire::message::Message;

    fn token(raw: u64) -> PieToken {
        PieToken::from_bytes(&raw.to_le_bytes()).unwrap()
    }

    #[test]
    fn construction_inputs_preserve_identity_arguments_and_reject_partial_frames() {
        use system_api::{control::construction as c, identity::{PrincipalId, Subject}, loader};
        let mut args = [0; loader::MAX_ARGS];
        args[..2].copy_from_slice(&[7, 11]);
        for input in [
            c::Input::Image(c::Image { image: token(4), offset: 17, len: 4096, stack: 8192, count: 2, args }),
            c::Input::Unit(c::UnitRef { name: String::from("shell"), stack: 8192, count: 2, args }),
        ] {
            for op in [c::BUILD, c::INSPECT, c::CANCEL] {
                let request = c::Request {
                    op, id: c::CreateId { value: 41 }, input: c::InputOption(Some(input.clone())),
                    owner: TaskId::new(8),
                    subject: Subject::new(PrincipalId::new(TaskId::new(2), 9), &[]).unwrap(),
                    back: token(5),
                };
                let mut bytes = c::Request::EMPTY;
                let len = request.store(&mut bytes).unwrap();
                let decoded = c::Request::fetch(&bytes[..len]).unwrap();
                assert_eq!(decoded.op, op);
                assert_eq!(decoded.id, request.id);
                assert_eq!(decoded.owner, request.owner);
                assert_eq!(decoded.subject, request.subject);
                assert_eq!(decoded.input.0, Some(input.clone()));
                assert_eq!(c::Call::back(&decoded), token(5));
                for end in 0..len {
                    assert!(c::Request::fetch(&bytes[..end]).is_none(), "accepted prefix {end}/{len}");
                }
                assert!(c::Request::fetch(&bytes[..len + 1]).is_none());
                bytes[10] = 99;
                assert!(c::Request::fetch(&bytes[..len]).is_none());
            }
        }
        for op in [c::INSPECT, c::CANCEL] {
            let request = c::Request {
                op, id: c::CreateId { value: 41 }, input: c::InputOption(None),
                owner: TaskId::new(8),
                subject: Subject::new(PrincipalId::new(TaskId::new(2), 9), &[]).unwrap(),
                back: token(5),
            };
            let mut bytes = c::Request::EMPTY;
            let len = request.store(&mut bytes).unwrap();
            let decoded = c::Request::fetch(&bytes[..len]).unwrap();
            assert_eq!(decoded.op, op);
            assert_eq!(decoded.id.value, 41);
            assert!(decoded.input.0.is_none());
            assert_eq!(c::Call::back(&decoded), token(5));
            bytes[9] = 2;
            assert!(c::Request::fetch(&bytes[..len]).is_none());
        }
    }

    #[test]
    fn construction_count_rejects_overflow_and_keeps_maximum_arguments() {
        use env::wire::Span;
        use system_api::{control::construction as c, loader};
        let args = core::array::from_fn(|i| i as u64 + 1);
        let mut bytes = [0; 1600];
        for input in [
            c::Input::Image(c::Image { image: token(4), offset: 0, len: 4096, stack: 0, count: loader::MAX_ARGS as u8, args }),
            c::Input::Unit(c::UnitRef { name: String::from("shell"), stack: 0, count: loader::MAX_ARGS as u8, args }),
        ] {
            let len = input.store_at(&mut bytes, 0).unwrap();
            assert_eq!(c::Input::fetch_at(&bytes[..len], 0), Some((input.clone(), len)));
            let mut invalid = input;
            match &mut invalid { c::Input::Image(value) => value.count += 1, c::Input::Unit(value) => value.count += 1 }
            assert!(invalid.store_at(&mut bytes, 0).is_none());
        }
        assert!(c::Input::fetch_at(&[99], 0).is_none());
    }

    #[test]
    fn actions_states_grants_and_marks_keep_their_fixed_values() {
        assert_eq!(
            [
                State::NeverStarted.code(),
                State::Starting.code(),
                State::Ready.code(),
                State::Stopping.code(),
                State::Dead.code(),
                State::Debarked.code()
            ],
            [0, 1, 2, 3, 4, 5],
        );
        assert_eq!(frame::fail_to_code(Some(Fail::Bad)), 5);
        assert_eq!(Grant::ALL.map(Grant::at), [1, 2, 3, 4, 5]);
        assert_eq!(
            Grant::ALL.map(Grant::name),
            ["state", "mint", "embark", "debark", "ruin"]
        );
        assert_eq!(
            Grant::ALL.map(Grant::mark),
            [
                Mark::of("control-entry-state"),
                Mark::of("control-entry-mint"),
                Mark::of("control-entry-embark"),
                Mark::of("control-entry-debark"),
                Mark::of("control-entry-ruin"),
            ]
        );
        assert_eq!(frame::MINT, 1);
        assert_eq!(frame::EMBARK, 2);
        assert_eq!(frame::DEBARK, 3);
        assert_eq!(frame::STATE, 4);
        assert_eq!(frame::RUIN, 5);
        assert_eq!(system_api::control::marks::DECLARATIONS.len(), 4);
        assert_eq!(env::marks::conflict(system_api::control::REGISTRY), None);
    }

    #[test]
    fn request_message_keeps_fixed_bytes_and_all_nine_shapes() {
        let back = token(0x0102_0304_0506_0708);
        let request = Request(Req::Mint(String::from("abc")), back);
        let mut bytes = Request::EMPTY;
        let len = request.store(&mut bytes).unwrap();
        assert_eq!(
            &bytes[..len],
            &[1, 3, b'a', b'b', b'c', 8, 7, 6, 5, 4, 3, 2, 1]
        );
        assert_eq!(
            Request::fetch(&bytes[..len]),
            Some((Some(Wire::Mint(String::from("abc"))), back))
        );

        let task = TaskId::new(0x1122_3344);
        let requests = [
            (1, Req::Mint(String::from("x"))),
            (2, Req::Embark(String::from("x"))),
            (3, Req::Debark(String::from("x"))),
            (5, Req::Ruin(String::from("x"))),
            (4, Req::State(String::from("x"))),
            (6, Req::EmbarkInstance(task)),
            (7, Req::DebarkInstance(task)),
            (8, Req::RuinInstance(task)),
            (9, Req::StateInstance(task)),
        ];
        for (op, req) in requests {
            let mut bytes = Request::EMPTY;
            let n = Request(req.clone(), back).store(&mut bytes).unwrap();
            assert_eq!(bytes[0], op);
            assert_eq!(Request::fetch(&bytes[..n]).unwrap().1, back);
            assert!(Request::fetch(&bytes[..n + 1]).is_none());
        }
        let unknown = [99, 1, b'x', 8, 7, 6, 5, 4, 3, 2, 1];
        assert_eq!(
            Request::fetch(&unknown),
            Some((None, token(0x0102_0304_0506_0708)))
        );
    }

    #[test]
    fn named_task_query_has_fixed_opcode_name_and_return_token() {
        let back = token(0x0102_0304_0506_0708);
        let request = Request(Req::Task(String::from("login")), back);
        let mut bytes = Request::EMPTY;
        let len = request.store(&mut bytes).unwrap();
        assert_eq!(
            &bytes[..len],
            &[10, 5, b'l', b'o', b'g', b'i', b'n', 8, 7, 6, 5, 4, 3, 2, 1]
        );
        let decoded = Request::fetch(&bytes[..len]).unwrap();
        assert_eq!(decoded, (Some(Wire::Task(String::from("login"))), back));
        assert_eq!(system_api::control::Call::back(&decoded), back);
        assert_eq!(Grant::for_wire(decoded.0.as_ref().unwrap()), Grant::State);
        for end in 0..len {
            assert!(Request::fetch(&bytes[..end]).is_none());
        }
        assert!(Request::fetch(&bytes[..len + 1]).is_none());
        let response = frame::said_task(TaskId::new(0x0102_0304_0506_0708));
        let mut answer = Said::EMPTY;
        assert_eq!(response.store(&mut answer), Some(27));
        assert_eq!(answer, [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 8, 7, 6, 5, 4, 3, 2, 1, 0, 0, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn said_and_account_frames_keep_layout_and_validation_boundary() {
        let said = Said { reason: 0, completed: false,
            status: 0,
            a: 3,
            task: TaskId::new(0x0102_0304_0506_0708),
            team: env::TeamId::new(0x1112_1314_1516_1718),
        };
        let mut bytes = Said::EMPTY;
        let n = said.store(&mut bytes).unwrap();
        assert_eq!(n, Said::LEN);
        assert_eq!(&bytes[..n], &[0, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 8, 7, 6, 5, 4, 3, 2, 1, 24, 23, 22, 21, 20, 19, 18, 17]);
        assert_eq!(Said::fetch(&bytes[..n]), Some(said));

        let account = AccountRequest {
            op: account_api::CREATE,
            id: system_api::control::construction::CreateId { value: 13 },
            account: String::from("ab"),
            back: token(0x0102_0304_0506_0708),
        };
        let mut bytes = AccountRequest::EMPTY;
        let n = account.store(&mut bytes).unwrap();
        assert_eq!(&bytes[..n], &[1, 13, 0, 0, 0, 0, 0, 0, 0, 2, b'a', b'b', 8, 7, 6, 5, 4, 3, 2, 1]);
        let (decoded, exact) = AccountRequest::fetch(&bytes[..n]).unwrap();
        assert!(exact);
        assert_eq!(decoded.op, account_api::CREATE);
        assert_eq!(decoded.id.value, 13);
        assert_eq!(decoded.account, "ab");
        assert_eq!(decoded.back, account.back);
        let (decoded, exact) = AccountRequest::fetch(&bytes[..n + 1]).unwrap();
        assert!(!exact);
        assert_eq!(decoded.back, account.back);

        let invalid = AccountRequest {
            op: account_api::CREATE,
            id: account.id,
            account: String::from("."),
            back: account.back,
        };
        let mut bytes = AccountRequest::EMPTY;
        let n = invalid.store(&mut bytes).unwrap();
        assert!(AccountRequest::fetch(&bytes[..n]).is_some());
        assert!(AccountRequest::take(&bytes[..n]).is_none());
    }

    #[test]
    fn identity_exports_preserve_actual_entry_revision_and_validation_boundaries() {
        use system_api::identity::{export, IdentityObject, IdentityName, PrincipalId, OperatorEntryRef};
        let request = export::Request {
            object: IdentityObject::Principal(PrincipalId::new(TaskId::new(77), 19)),
            alias: IdentityName::new("anran").unwrap(),
            binding: IdentityName::new("create").unwrap(),
            entry: OperatorEntryRef { operator: TaskId::new(81), root: false, id: 0, revision: 123 },
            back: token(9),
        };
        assert!(request.valid(), "Node(0) is a real direct entry");
        let mut bytes = [0; export::Request::LEN + 1];
        let n = request.store(&mut bytes).unwrap();
        let (decoded, exact) = export::Request::fetch(&bytes[..n]).unwrap();
        assert!(exact && decoded.valid());
        assert_eq!(decoded.object, request.object);
        assert_eq!(decoded.entry, request.entry);
        assert_eq!(decoded.alias, request.alias);
        assert_eq!(export::Call::back(&(decoded, exact)), request.back);
        let (_, exact) = export::Request::fetch(&bytes[..n + 1]).unwrap();
        assert!(!exact, "trailing data is rejected by export admission");
        for end in 0..n { assert!(export::Request::fetch(&bytes[..end]).is_none()); }
        let mut invalid = request;
        invalid.entry.root = true;
        invalid.entry.id = 1;
        assert!(!invalid.valid());
        invalid.entry.root = false;
        invalid.entry.operator = TaskId::new(0);
        assert!(!invalid.valid());
        assert!(IdentityName::new("bad/name").is_none());
        let reply = export::Response::accepted();
        let mut bytes = [0; export::Response::LEN + 1];
        let n = reply.store(&mut bytes).unwrap();
        assert_eq!(export::Response::fetch(&bytes[..n]).unwrap().status, wire::OK);
        assert!(export::Response::fetch(&bytes[..n + 1]).is_none());
        for end in 0..n { assert!(export::Response::fetch(&bytes[..end]).is_none()); }
    }
}
