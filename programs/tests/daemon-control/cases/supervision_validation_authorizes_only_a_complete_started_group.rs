use crate::support::{programmed, target};
use crate::system::control::supervision;
use crate::system::control::unit::Input;
use crate::unit::Dependency;

static AFTER_SCENE: &[Dependency] = &[Dependency::Target("scene")];

#[test]
fn supervision_validation_authorizes_only_a_complete_started_group() {
    let main = programmed("main", Some("scene"), Some(AFTER_SCENE));
    let worker = programmed("worker", None, None);
    let inputs = [Input { program: main }, Input { program: worker }];
    let members: &'static [&'static str] = &["worker"];
    assert_eq!(supervision::validate(&inputs, target(members), &[]), Ok(()));

    let missing = programmed("main", Some("missing"), Some(AFTER_SCENE));
    assert_eq!(
        supervision::validate(
            &[Input { program: missing }, Input { program: worker }],
            target(members),
            &[],
        ),
        Err("supervision group missing")
    );

    let unordered = programmed("main", Some("scene"), None);
    assert_eq!(
        supervision::validate(
            &[Input { program: unordered }, Input { program: worker }],
            target(members),
            &[],
        ),
        Err("supervision ordering missing")
    );

    let self_group: &'static [&'static str] = &["main"];
    assert_eq!(
        supervision::validate(&inputs, target(self_group), &[]),
        Err("supervision includes self")
    );

    let guarded: &'static [&'static str] = &["worker"];
    assert_eq!(
        supervision::validate(&inputs, target(guarded), &["worker"]),
        Err("supervision includes guarded target")
    );

    let duplicate: &'static [&'static str] = &["worker", "worker"];
    assert_eq!(
        supervision::validate(&inputs, target(duplicate), &[]),
        Err("duplicate supervision member")
    );

    let absent: &'static [&'static str] = &["ghost"];
    assert_eq!(
        supervision::validate(&inputs, target(absent), &[]),
        Err("supervision member not started")
    );

    let second = programmed("second", Some("scene"), Some(AFTER_SCENE));
    assert_eq!(
        supervision::validate(
            &[
                Input { program: main },
                Input { program: worker },
                Input { program: second },
            ],
            target(members),
            &[],
        ),
        Err("multiple supervision declarations")
    );
}
