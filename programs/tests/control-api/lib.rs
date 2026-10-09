#![allow(dead_code)]
extern crate alloc;

#[cfg(test)]
mod tests {
    use alloc::{string::String, vec};
    use env::{Mark, PieToken, TaskId};
    use account_api::Request as AccountRequest;
    use system_api::control::{
        frame::{self, Fail, Req, Request, Said, State, Wire},
        publication::{self, Frame, Object, Reply, Scope, Target},
        Grant,
    };
    use wire::message::Message;

    fn token(raw: u64) -> PieToken { PieToken::from_bytes(&raw.to_le_bytes()).unwrap() }

    #[test]
    fn construction_envelope_preserves_image_owner_and_subject_and_rejects_partial_frames() {
        use system_api::{control::construction, identity::{PrincipalId, Subject}, loader};
        let mut args = [0; loader::MAX_ARGS];
        args[..2].copy_from_slice(&[7, 11]);
        let request = construction::Request {
            image: loader::Ask { op: loader::BUILD, image: token(4), offset: 0, len: 4096,
                stack: 0, count: 2, args, back: token(5) },
            owner: TaskId::new(8),
            subject: Subject::new(PrincipalId::new(TaskId::new(2), 9), &[]).unwrap(),
        };
        let mut bytes = construction::Request::EMPTY;
        let len = request.store(&mut bytes).unwrap();
        assert_eq!(len, 58 + 8 + 17);
        let decoded = construction::Request::fetch(&bytes[..len]).unwrap();
        assert_eq!(decoded.owner, request.owner);
        assert_eq!(decoded.subject, request.subject);
        assert_eq!(decoded.image.image, request.image.image);
        assert_eq!(decoded.image.back, request.image.back);
        assert_eq!(&decoded.image.args[..2], &[7, 11]);
        assert_eq!(construction::Call::back(&decoded), token(5));
        for end in 0..len { assert!(construction::Request::fetch(&bytes[..end]).is_none()); }
        assert!(construction::Request::fetch(&bytes[..len + 1]).is_none());
    }

    #[test]
    fn actions_states_grants_and_marks_keep_their_fixed_values() {
        assert_eq!(
            [State::NeverStarted.code(), State::Starting.code(), State::Ready.code(),
                State::Stopping.code(), State::Dead.code(), State::Debarked.code()],
            [0, 1, 2, 3, 4, 5],
        );
        assert_eq!(frame::fail_to_code(Some(Fail::Bad)), 5);
        assert_eq!(Grant::ALL.map(Grant::at), [1, 2, 3, 4, 5]);
        assert_eq!(Grant::ALL.map(Grant::name), ["state", "mint", "embark", "debark", "ruin"]);
        assert_eq!(Grant::ALL.map(Grant::mark), [
            Mark::of("control-entry-state"), Mark::of("control-entry-mint"),
            Mark::of("control-entry-embark"), Mark::of("control-entry-debark"),
            Mark::of("control-entry-ruin"),
        ]);
        assert_eq!(frame::MINT, 1);
        assert_eq!(frame::EMBARK, 2);
        assert_eq!(frame::DEBARK, 3);
        assert_eq!(frame::STATE, 4);
        assert_eq!(frame::RUIN, 5);
        assert_eq!(system_api::control::marks::DECLARATIONS.len(), 7);
        assert_eq!(env::marks::conflict(system_api::control::REGISTRY), None);
    }

    #[test]
    fn request_message_keeps_fixed_bytes_and_all_nine_shapes() {
        let back = token(0x0102_0304_0506_0708);
        let request = Request(Req::Mint(String::from("abc")), back);
        let mut bytes = Request::EMPTY;
        let len = request.store(&mut bytes).unwrap();
        assert_eq!(&bytes[..len], &[1, 3, b'a', b'b', b'c', 8, 7, 6, 5, 4, 3, 2, 1]);
        assert_eq!(Request::fetch(&bytes[..len]), Some((Some(Wire::Mint(String::from("abc"))), back)));

        let task = TaskId::new(0x1122_3344);
        let requests = [
            (1, Req::Mint(String::from("x"))), (2, Req::Embark(String::from("x"))),
            (3, Req::Debark(String::from("x"))), (5, Req::Ruin(String::from("x"))),
            (4, Req::State(String::from("x"))), (6, Req::EmbarkInstance(task)),
            (7, Req::DebarkInstance(task)), (8, Req::RuinInstance(task)), (9, Req::StateInstance(task)),
        ];
        for (op, req) in requests {
            let mut bytes = Request::EMPTY;
            let n = Request(req.clone(), back).store(&mut bytes).unwrap();
            assert_eq!(bytes[0], op);
            assert_eq!(Request::fetch(&bytes[..n]).unwrap().1, back);
            assert!(Request::fetch(&bytes[..n + 1]).is_none());
        }
        let unknown = [99, 1, b'x', 8, 7, 6, 5, 4, 3, 2, 1];
        assert_eq!(Request::fetch(&unknown), Some((None, token(0x0102_0304_0506_0708))));
    }

    #[test]
    fn said_and_account_frames_keep_layout_and_validation_boundary() {
        let said = Said { status: 0, a: 3, task: TaskId::new(0x0102_0304_0506_0708) };
        let mut bytes = Said::EMPTY;
        let n = said.store(&mut bytes).unwrap();
        assert_eq!(n, Said::LEN);
        assert_eq!(&bytes[..n], &[0, 3, 8, 7, 6, 5, 4, 3, 2, 1]);
        assert_eq!(Said::fetch(&bytes[..n]), Some(said));

        let account = AccountRequest { account: String::from("ab"), back: token(0x0102_0304_0506_0708) };
        let mut bytes = AccountRequest::EMPTY;
        let n = account.store(&mut bytes).unwrap();
        assert_eq!(&bytes[..n], &[2, b'a', b'b', 8, 7, 6, 5, 4, 3, 2, 1]);
        let (decoded, exact) = AccountRequest::fetch(&bytes[..n]).unwrap();
        assert!(exact);
        assert_eq!(decoded.account, "ab");
        assert_eq!(decoded.back, account.back);
        let (decoded, exact) = AccountRequest::fetch(&bytes[..n + 1]).unwrap();
        assert!(!exact);
        assert_eq!(decoded.back, account.back);

        let invalid = AccountRequest { account: String::from("."), back: account.back };
        let mut bytes = AccountRequest::EMPTY;
        let n = invalid.store(&mut bytes).unwrap();
        assert!(AccountRequest::fetch(&bytes[..n]).is_some());
        assert!(AccountRequest::take(&bytes[..n]).is_none());
    }

    #[test]
    fn publication_frames_preserve_fixed_wire_bytes_and_target_validation() {
        let entry = token(0x0102_0304_0506_0708);
        let frame = Frame::new(publication::PUBLISH, Target::Service { scope: Scope(1), group: String::from("grp"), name: String::from("svc") }, (entry, system_api::operator::Permit::Public));
        assert_eq!(frame.target(), Some(Target::Service {
            scope: Scope(1), group: String::from("grp"), name: String::from("svc"),
        }));
        let mut bytes = Frame::EMPTY;
        let n = frame.store(&mut bytes).unwrap();
        let mut expected = vec![1, 1, 3, b'g', b'r', b'p', 3, b's', b'v', b'c'];
        expected.extend_from_slice(&[0; 16]);
        expected.extend_from_slice(&entry.to_bytes());
        expected.extend_from_slice(&[0; 18]);
        expected.extend_from_slice(&[0; 8]);
        assert_eq!(n, expected.len());
        assert_eq!(&bytes[..n], expected.as_slice());
        assert_eq!(Frame::fetch(&bytes[..n]), Some(frame.clone()));
        let mut invalid = frame;
        invalid.name = String::from("bad/name");
        let mut bytes = Frame::EMPTY;
        let n = invalid.store(&mut bytes).unwrap();
        let decoded = Frame::fetch(&bytes[..n]).unwrap();
        assert!(decoded.target().is_none());

        let reply = Reply { status: 0, kind: 2, task: TaskId::new(0x0102_0304_0506_0708), number: 0x1112_1314_1516_1718 };
        let mut bytes = Reply::EMPTY;
        let n = reply.store(&mut bytes).unwrap();
        assert_eq!(&bytes[..n], &[0, 2, 8, 7, 6, 5, 4, 3, 2, 1, 24, 23, 22, 21, 20, 19, 18, 17]);
        assert_eq!(Reply::fetch(&bytes[..n]), Some(reply));
        assert_eq!(reply.identity(TaskId::new(0x0102_0304_0506_0708)), Ok(Object::Coalition(
            system_api::identity::CoalitionId::new(TaskId::new(0x0102_0304_0506_0708), 0x1112_1314_1516_1718),
        )));
    }
}
