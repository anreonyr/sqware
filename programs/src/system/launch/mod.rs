use ::schedule::{BuildError, Plan, Progress, ResMut, Schedule};
use ipc::rpc::reply::Sender;
use system_api::control::Fail;
use system_api::loader::Said;
pub(crate) fn frame() -> Result<Plan<crate::system::app::Fault>, BuildError> {
    let mut frame = Schedule::sequence();
    frame.system("dispatch", dispatch)?;
    frame.plan("receive", crate::system::loader::frame()?)?;
    frame.build()
}

pub(crate) fn reply(back: Sender<Said>, result: Result<system_api::loader::Built, Fail>) -> bool {
    back.send(Said::from_result(result)).is_ok()
}

fn dispatch(
    mut inbox: ResMut<crate::system::control::Construction>,
    mut jobs: ResMut<crate::system::assembly::Jobs>,
    mut control: ResMut<crate::system::control::unit::Control>,
) -> Result<Progress, crate::system::app::Fault> {
    for (constructor_entry, mut request, from, back, approved) in inbox.drain() {
        if approved.is_none() {
            release_input(&request, from);
            reply(back, Err(Fail::Denied));
            continue;
        }
        if matches!(
            request.op,
            system_api::control::construction::INSPECT | system_api::control::construction::CANCEL
        ) {
            release_input(&request, from);
            let key = crate::system::assembly::CreateKey {
                entry: constructor_entry,
                caller: from,
                id: request.id,
            };
            let said = if request.op == system_api::control::construction::CANCEL {
                jobs.cancel_create(key, &mut control)
            } else {
                jobs.create_reply(key)
            };
            let _ = back.send(said);
            continue;
        }
        let key = crate::system::assembly::CreateKey {
            entry: constructor_entry,
            caller: from,
            id: request.id,
        };
        match jobs.admit_create(key, &request) {
            Ok(crate::system::assembly::CreateAdmission::Cancelled) => {
                release_input(&request, from);
                let _ = back.send(jobs.create_reply(key));
                continue;
            }
            Err(fail) => {
                release_input(&request, from);
                let _ = back.send(Said::from_result(Err(fail)));
                continue;
            }
            Ok(
                crate::system::assembly::CreateAdmission::New
                | crate::system::assembly::CreateAdmission::Duplicate,
            ) => {}
        }
        let Some(input) = request.input.0.take() else {
            let _ = back.send(Said::from_result(Err(Fail::Bad)));
            continue;
        };
        let _ = jobs.enqueue(crate::system::assembly::Request {
            image: crate::system::assembly::ImageRequest::new(input, from),
            delivery: crate::system::assembly::Delivery {
                owner: request.owner,
                identity: system_api::identity::Install::Authorized(request.subject),
                program: None,
                back: Some(back),
            },
            create: Some(key),
        });
    }
    Ok(Progress::Done)
}

fn release_input(request: &system_api::control::construction::Request, from: env::TaskId) {
    if let Some(system_api::control::construction::Input::Image(image)) = request.input.0.as_ref() {
        crate::system::loader::release_image(image, from);
    }
}
