use super::material::Supplies;
use super::resource::Resources;
use super::start::Images;
use super::unit::Control;
use crate::service::hub::bridge::Activation;
use crate::system::control::core::publication::Publications;
use crate::system::control::core::unit::State;
use crate::system::identity::serve::install::Roster;
use crate::system::identity::serve::names::Names;
use crate::system::operator::serve::install::Tree;
use env::{PieToken, Wait};
use protocol::{communication::hand::Sender, debug, system::control as ccall};
use runtime::env::mail::{self, HolePie};
pub(crate) fn serve_face(
    control: &mut Control,
    roster: &Roster,
    supplies: &mut Supplies,
    activation: &mut Option<Activation>,
    entry: PieToken,
    publication: &mut Publications,
    resources: &mut Resources,
    names: &mut Names,
    tree: &mut Tree,
    images: &Images,
    grant: ccall::Grant,
    face: PieToken,
    buf: &mut [u8],
) {
    let hole = HolePie::from_token(face);
    while let Ok((len, from)) = hole.pull(buf, Wait::POLL) {
        let Some((ask, back)) = ccall::frame::Wire::take(&buf[..len]) else {
            continue;
        };
        if !matches!(
            mail::reserve(back),
            Ok((_vestor, owner, mark)) if owner == from && mark == ccall::BACK
        ) {
            continue;
        }
        if let Some(wire) = &ask {
            let asked = ccall::Grant::for_wire(wire);
            if asked != grant {
                debug!("control: face={} denied as={}", grant.name(), asked.name());
                {
                    let mut tx = Sender::<ccall::frame::Said>::from_token(back);
                    let _ = tx.send(ccall::frame::said_status(ccall::frame::DENIED));
                }
                let _ = mail::release(back);
                continue;
            }
        }
        let said = answer(
            control,
            roster,
            supplies,
            activation,
            entry,
            publication,
            resources,
            names,
            tree,
            images,
            from,
            ask,
        );
        {
            let mut tx = Sender::<ccall::frame::Said>::from_token(back);
            let _ = tx.send(said);
        }
        let _ = mail::release(back);
    }
}
fn answer(
    control: &mut Control,
    roster: &Roster,
    supplies: &mut Supplies,
    activation: &mut Option<Activation>,
    entry: PieToken,
    publication: &mut Publications,
    resources: &mut Resources,
    names: &mut Names,
    tree: &mut Tree,
    images: &Images,
    from: env::TaskId,
    ask: Option<ccall::frame::Wire>,
) -> ccall::frame::Said {
    let code = |fail: crate::system::control::core::verdict::Fail| {
        ccall::frame::fail_to_code(Some(wire_fail(fail)))
    };
    let Some(ask) = ask else {
        return ccall::frame::said_status(ccall::frame::BAD);
    };
    let supplies_machine = supplies.machine;
    match ask {
        ccall::frame::Wire::Mint(name) => match control.mint(name, images) {
            Ok(()) => ccall::frame::said_status(ccall::frame::OK),
            Err(fail) => ccall::frame::said_status(code(fail)),
        },
        ccall::frame::Wire::Start(name) => match control.release(
            name,
            from,
            roster,
            activation,
            supplies,
            |control, activation| {
                crate::system::run::cycle::poll(
                    control,
                    roster,
                    &supplies_machine,
                    activation,
                    entry,
                    publication,
                    resources,
                    names,
                    tree,
                )
            },
        ) {
            Ok(service) => ccall::frame::said_task(service.0),
            Err(fail) => ccall::frame::said_status(code(fail)),
        },
        ccall::frame::Wire::Stop(name) => match control.stop(name, roster, activation) {
            Ok(()) => ccall::frame::said_status(ccall::frame::OK),
            Err(fail) => ccall::frame::said_status(code(fail)),
        },
        ccall::frame::Wire::State(name) => match control.state(name) {
            Ok(state) => ccall::frame::said_state(wire_state(state)),
            Err(fail) => ccall::frame::said_status(code(fail)),
        },
    }
}
fn wire_fail(fail: crate::system::control::core::verdict::Fail) -> ccall::Fail {
    use crate::system::control::core::verdict::Fail as Model;
    match fail {
        Model::Unknown => ccall::Fail::Unknown,
        Model::BadImage => ccall::Fail::BadImage,
        Model::Full => ccall::Fail::Full,
        Model::NotReady => ccall::Fail::NotReady,
    }
}
fn wire_state(state: State) -> ccall::State {
    match state {
        State::NeverStarted => ccall::State::NeverStarted,
        State::Starting => ccall::State::Starting,
        State::Ready => ccall::State::Ready,
        State::Stopping => ccall::State::Stopping,
        State::Dead => ccall::State::Dead,
    }
}
