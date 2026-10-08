#![no_std]
#![no_main]
extern crate alloc;
extern crate programs;
use env::wire::Span as _;
use env::{Mark, PieToken, TaskId, Wait};
use programs::harness::probe::hierarchy::{ANSWER, COMMAND};
use ipc::session::{Session, establish};
use system_client::control::publication::Client;
use system_api::control::Object;
use system_api::control::Target;
use system_api::identity::Selector;
use system_client::identity::Query;
use system_client::identity::SelfOps;
use env::pie;
use ::resource::raw::{Hole, reserve};
use system_api::operator::Fail;
use system_api::operator::Permit;
use system_client::operator as operator;
use system_client::operator::Face;
const WAIT: Wait = Wait::AtMost(3000);
#[programs::entry]
fn main() -> programs::Report<'static> {
    let control = env::unit::sire();
    let me = env::unit::self_id();
    let command = establish::claim(control, COMMAND, WAIT).unwrap();
    let answer = establish::claim(control, ANSWER, WAIT).unwrap();
    let _ready = establish::Held(
        establish::endpoint(control, Mark::of(programs::unit::READY), Wait::POLL).unwrap(),
    );
    let session = Session::open(control, operator::BERTH, WAIT)
        .unwrap_or_else(|_| panic!("hierarchy operator session"));
    let tree = Face::of(session);
    let authority = system_client::identity::authority().unwrap();
    let query = Query::discover(&tree, authority, WAIT).unwrap();
    let self_ops = SelfOps::discover(&tree, authority, WAIT).unwrap();
    let client = Client::injected().unwrap();
    let mut resource = PieToken::NONE;
    let mut acquired = PieToken::NONE;
    let mut c = None;
    let mut target = None;
    let mut road = None;
    loop {
        let mut bytes = [0; 9];
        let (n, sender) = Hole::from_raw(command)
            .pull(&mut bytes, Wait::Forever)
            .unwrap();
        assert_eq!(n, 9);
        assert_eq!(sender, control);
        let task = TaskId::new(u64::from_le_bytes(bytes[1..].try_into().unwrap()) as usize);
        match bytes[0] {
            1 => {
                let Object::Principal(p) = client
                    .reference(&tree, authority, 1, "named-subject", WAIT)
                    .unwrap()
                else {
                    panic!("principal ref type");
                };
                let Object::Coalition(coalition) = client
                    .reference(&tree, authority, 2, "named-league", WAIT)
                    .unwrap()
                else {
                    panic!("coalition ref type");
                };
                assert_eq!(
                    query.resolve(me, WAIT).unwrap().unwrap().current.principal,
                    p
                );
                assert_eq!(query.heir(p, p, WAIT), Ok(true));
                assert_eq!(query.amid(p, coalition, WAIT), Ok(true));
                assert_eq!(
                    query
                        .resolve(task, WAIT)
                        .unwrap()
                        .unwrap()
                        .current
                        .principal,
                    p
                );
                assert!(
                    Client::reference_direct(
                        control,
                        authority,
                        pie::unseal_hole(system_api::control::publication::REF).unwrap(),
                        1,
                        "named-subject",
                        WAIT
                    )
                    .is_err(),
                    "same mark from untrusted owner"
                );
                assert!(
                    client
                        .publish(
                            Target::IdentityName {
                                object: Object::Principal(p),
                                name: "forged".into()
                            },
                            pie::unseal_hole(Mark::of("forged")).unwrap(),
                            Permit::Public,
                            WAIT
                        )
                        .is_err()
                );
                resource = pie::unseal_hole(Mark::of("hierarchy-resource")).unwrap();
                let proxy = Target::RuntimeResource {
                    task,
                    kind: "test".into(),
                    name: "member".into(),
                };
                let id = client
                    .publish(
                        proxy.clone(),
                        resource,
                        Permit::Identity(Selector::MemberOf(coalition)),
                        WAIT,
                    )
                    .unwrap();
                assert_eq!(
                    client.publish(
                        proxy.clone(),
                        resource,
                        Permit::Identity(Selector::MemberOf(coalition)),
                        WAIT
                    ),
                    Ok(id)
                );
                let base = client.runtime(task, WAIT).unwrap();
                let resource_road = base.try_join("test").unwrap().try_join("member").unwrap();
                acquired = tree
                    .tile(&resource_road, WAIT)
                    .unwrap()
                    .token(WAIT)
                    .unwrap();
                // Directory ownership is the real target, resource ownership remains the service.
                assert_eq!(reserve(acquired).unwrap().1, me);
                assert_eq!(
                    client.unpublish(
                        Target::RuntimeResource {
                            task: control,
                            kind: "test".into(),
                            name: "member".into()
                        },
                        WAIT
                    ),
                    Err(Fail::Unknown)
                );
                let own = Target::RuntimeResource {
                    task: me,
                    kind: "hole".into(),
                    name: "same-subject".into(),
                };
                client
                    .publish(own, resource, Permit::Identity(Selector::Exact(p)), WAIT)
                    .unwrap();
                let ownroad = client
                    .runtime(me, WAIT)
                    .unwrap()
                    .try_join("hole")
                    .unwrap()
                    .try_join("same-subject")
                    .unwrap();
                assert!(tree.tile(&ownroad, WAIT).unwrap().token(WAIT).is_ok());
                {
                    use system_api::control::publication::BACK;
use system_api::control::publication::ENTRY;
use system_api::control::publication::Frame;
                    use ::resource::port::{self, Access, Policy};
                    let abandoned = Target::RuntimeResource {
                        task: me,
                        kind: "public".into(),
                        name: "abandoned".into(),
                    };
                    let seed = port::ship(
                        resource,
                        control,
                        Access::FETCH | Access::STORE,
                        Policy::VEST,
                    )
                    .unwrap()
                    .seed();
                    let closed = pie::unseal_hole(BACK).unwrap();
                    let reply = port::ship(
                        closed,
                        control,
                        Access::STORE,
                        Policy::NONE,
                    )
                    .unwrap()
                    .seed();
                    pie::seal(closed).unwrap();
                    let mut frame = Frame::new(1, abandoned.clone(), (seed, Permit::Public));
                    frame.back = reply;
                    ipc::hand::Sender::<Frame>::from_raw(
                        establish::find(control, ENTRY).unwrap(),
                    )
                    .send_within(frame, WAIT)
                    .unwrap_or_else(|_| panic!("abandoned request admission"));
                    client.runtime(me, WAIT).unwrap();
                    assert!(
                        pie::revoke(control, seed).is_err(),
                        "invalid reply channel must release transferred source"
                    );
                    client
                        .publish(abandoned.clone(), resource, Permit::Public, WAIT)
                        .unwrap();
                    client.unpublish(abandoned, WAIT).unwrap();
                    let _ = pie::release(closed);
                    programs::debug::put(
                        "hierarchy: abandoned request with closed reply channel drops its borrowed source and leaves no claim",
                    );
                }
                let full = |i| Target::RuntimeResource {
                    task,
                    kind: "full".into(),
                    name: alloc::format!("f{i}"),
                };
                for i in 0..32 {
                    client
                        .publish(full(i), resource, Permit::Public, WAIT)
                        .unwrap();
                }
                assert!(
                    client
                        .publish(full(32), resource, Permit::Public, WAIT)
                        .is_err(),
                    "full Pane must reject publication"
                );
                client.unpublish(full(0), WAIT).unwrap();
                client
                    .publish(full(32), resource, Permit::Public, WAIT)
                    .unwrap();
                for i in 1..33 {
                    client.unpublish(full(i), WAIT).unwrap();
                }
                programs::debug::put(
                    "hierarchy: full mount failure leaves no registry claim; retry after freeing capacity succeeds",
                );
                c = Some(coalition);
                target = Some(proxy);
                road = Some(resource_road);
                programs::debug::put(
                    "hierarchy: named full IDs -> Identity lineage/member queries -> MemberOf Find allowed",
                );
            }
            2 => {
                let road = road.as_ref().unwrap();
                assert_eq!(
                    tree.tile(road, WAIT).unwrap().token(WAIT),
                    Err(Fail::Denied)
                );
                self_ops.waive(WAIT).unwrap();
                assert_eq!(
                    tree.tile(road, WAIT).unwrap().token(WAIT),
                    Err(Fail::Denied)
                );
                client
                    .unpublish(target.as_ref().unwrap().clone(), WAIT)
                    .unwrap();
                assert!(matches!(tree.root().tile(road, WAIT), Err(Fail::Unknown)));
                Hole::from_raw(acquired).push(b"kept", WAIT).unwrap();
                let mut read = [0; 4];
                assert_eq!(
                    Hole::from_raw(resource).pull(&mut read, WAIT).unwrap(),
                    (4, me)
                );
                assert_eq!(&read, b"kept");
                pie::seal(resource).unwrap();
                assert!(
                    reserve(acquired).is_err(),
                    "resource close must invalidate delivered capability"
                );
                let other = pie::unseal_hole(Mark::of("hierarchy-resource")).unwrap();
                client
                    .publish(
                        target.as_ref().unwrap().clone(),
                        other,
                        Permit::Identity(Selector::MemberOf(c.unwrap())),
                        WAIT,
                    )
                    .unwrap();
                resource = other;
                programs::debug::put(
                    "hierarchy: expel -> Find denied -> waive still denied; unpublish keeps delivered capability; close invalidates it",
                );
            }
            3 => {
                assert!(client.runtime(task, WAIT).is_err());
                assert!(matches!(
                    tree.root().tile(road.as_ref().unwrap(), WAIT),
                    Err(Fail::Unknown)
                ));
                assert!(
                    client
                        .publish(
                            target.as_ref().unwrap().clone(),
                            resource,
                            Permit::Identity(Selector::MemberOf(c.unwrap())),
                            WAIT
                        )
                        .is_err()
                );
                assert!(
                    reserve(resource).is_ok(),
                    "target exit must not close proxy service resource"
                );
                programs::debug::put(
                    "hierarchy: actual team/task proxy registration and child-first target exit cleanup passed",
                );
            }
            5 => {
                use system_api::control::publication::BACK;
use system_api::control::publication::REF;
use system_api::control::publication::Reply;
                use ::resource::port::{self, Access, Policy};
                let fake = establish::find(control, REF).unwrap();
                assert_eq!(
                    Client::reference_direct(control, authority, fake, 1, "wrong-authority", WAIT),
                    Err(Fail::Unjudged)
                );
                let back = pie::unseal_hole(BACK).unwrap();
                let from = me.get();
                let helper = execution::unit::task::spawn(move || {
                    let back = establish::claim(TaskId::new(from), BACK, WAIT).unwrap();
                    let reply = Reply {
                        status: 0,
                        kind: 1,
                        task: TaskId::new(0),
                        number: 1,
                    };
                    let mut bytes = [0; Reply::LEN];
                    let n = reply.store_at(&mut bytes, 0).unwrap();
                    Hole::from_raw(back).push(&bytes[..n], WAIT).unwrap();
                });
                port::ship(
                    back,
                    helper.id(),
                    Access::STORE,
                    Policy::NONE,
                )
                .unwrap();
                let mut encoded = [0; Reply::LEN];
                let (n, actual) = Hole::from_raw(back).pull(&mut encoded, WAIT).unwrap();
                assert_eq!(actual, helper.id());
                assert_eq!(
                    Reply::from_sender(control, actual, &encoded[..n]),
                    Err(Fail::Denied)
                );
                helper.join();
                let _ = pie::seal(back);
                let _ = pie::release(back);
                programs::debug::put(
                    "hierarchy: real ref IPC rejects wrong authority; shared reply validator rejects actual forged sender",
                );
            }
            4 => {
                assert!(
                    client
                        .reference(&tree, authority, 1, "named-subject", WAIT)
                        .is_err()
                );
                assert!(
                    client
                        .reference(&tree, authority, 2, "named-league", WAIT)
                        .is_err()
                );
                let Object::Principal(p) = client
                    .reference(&tree, authority, 1, "system-dependent", WAIT)
                    .unwrap()
                else {
                    panic!("static alias type");
                };
                assert_eq!(
                    p,
                    query.resolve(me, WAIT).unwrap().unwrap().origin.principal
                );
                programs::debug::put(
                    "hierarchy: fresh static ref has new authority; dynamic names did not rebind",
                );
            }
            _ => panic!("unknown hierarchy command"),
        }
        Hole::from_raw(answer).push(&bytes[..1], WAIT).unwrap();
    }
}
