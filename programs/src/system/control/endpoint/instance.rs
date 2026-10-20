use super::request::{self as answer, Inbox};
use crate::system::app::Fault as Fail;
use crate::system::assembly::Jobs;
use crate::system::control::instance::Command;
use crate::system::control::unit::Control;
use ::schedule::{Progress, Res, ResMut};
use env::Wait;
use ipc::rpc::{self, request::Receiver as RequestReceiver};
use system_api::control as call;
use system_api::control::Call as ControlContract;

pub fn answer(
    mut control: ResMut<Control>,
    mut inbox: ResMut<Inbox>,
    mut jobs: ResMut<Jobs>,
) -> Result<Progress, Fail> {
    let count = inbox.0.len();
    for _ in 0..count {
        let incoming = inbox.0.pop_front().ok_or(Fail::Room)?;
        let command = match &incoming.wire {
            call::frame::Wire::EmbarkInstance(task) => Command::Embark(*task),
            call::frame::Wire::DebarkInstance(task) => Command::Debark(*task),
            call::frame::Wire::RuinInstance(task) => Command::Ruin(*task),
            call::frame::Wire::StateInstance(task) => Command::State(*task),
            _ => {
                inbox.0.push_back(incoming);
                continue;
            }
        };
        let task = match command {
            Command::Embark(task)
            | Command::Debark(task)
            | Command::Ruin(task)
            | Command::State(task) => task,
        };
        let Some(reference) = control.instance(task).map(|instance| instance.reference) else {
            answer::reply(
                incoming.reply,
                call::frame::said_status(call::frame::fail_to_code(Some(call::Fail::Unknown))),
            );
            continue;
        };
        if !incoming.reply.is_alive() {
            drop(incoming.reply);
            continue;
        }
        if let Err((fail, incoming)) = jobs.submit_command(incoming, reference) {
            answer::reply(
                incoming.reply,
                call::frame::said_status(call::frame::fail_to_code(Some(match fail {
                    crate::system::control::unit::verdict::Fail::Full => call::Fail::Full,
                    crate::system::control::unit::verdict::Fail::Unknown => call::Fail::Unknown,
                    crate::system::control::unit::verdict::Fail::BadImage => call::Fail::BadImage,
                    crate::system::control::unit::verdict::Fail::NotReady => call::Fail::NotReady,
                }))),
            );
        }
    }
    let commands = jobs.take_commands(16);
    for mut work in commands {
        let command = match &work.wire {
            call::frame::Wire::EmbarkInstance(task) => Command::Embark(*task),
            call::frame::Wire::DebarkInstance(task) => Command::Debark(*task),
            call::frame::Wire::RuinInstance(task) => Command::Ruin(*task),
            call::frame::Wire::StateInstance(task) => Command::State(*task),
            _ => {
                finish_command(
                    &mut jobs,
                    work,
                    Err(crate::system::assembly::JobFailure::Command(
                        call::Fail::Denied,
                    )),
                );
                continue;
            }
        };
        let task = match command {
            Command::Embark(task)
            | Command::Debark(task)
            | Command::Ruin(task)
            | Command::State(task) => task,
        };
        if control.instance(task).map(|instance| instance.reference) != Some(work.reference) {
            finish_command(
                &mut jobs,
                work,
                Err(crate::system::assembly::JobFailure::Command(
                    call::Fail::Unknown,
                )),
            );
            continue;
        }
        let result = control.command_instance(work.from, command);
        match result {
            Ok(Some(crate::system::control::unit::table::State::Starting))
                if matches!(command, Command::Embark(_)) =>
            {
                let reference = work.reference;
                let deadline = *work.activation_deadline.get_or_insert_with(|| {
                    env::chrono::clock() + crate::support::timing::BOOT_MS as u64 * 1_000_000
                });
                let readiness = crate::system::control::instance::activation::bounded_ready(
                    jobs.poll_instance_ready(reference),
                    env::chrono::clock(),
                    deadline,
                );
                match readiness {
                    Ok(true) => finish_activation(
                        &mut jobs,
                        ActivationFinish {
                            control: &mut control,
                            work,
                            result: Ok(crate::system::assembly::JobOutcome::Activate(reference)),
                        },
                    ),
                    Ok(false) => jobs.requeue_command(work),
                    Err(fail) => {
                        finish_activation(
                            &mut jobs,
                            ActivationFinish {
                                control: &mut control,
                                work,
                                result: Err(crate::system::assembly::JobFailure::Command(
                                    match fail {
                                        crate::system::control::unit::verdict::Fail::Unknown => {
                                            call::Fail::Unknown
                                        }
                                        crate::system::control::unit::verdict::Fail::BadImage => {
                                            call::Fail::BadImage
                                        }
                                        crate::system::control::unit::verdict::Fail::Full => {
                                            call::Fail::Full
                                        }
                                        crate::system::control::unit::verdict::Fail::NotReady => {
                                            call::Fail::NotReady
                                        }
                                    },
                                )),
                            },
                        );
                    }
                }
            }
            Ok(Some(state)) => {
                let outcome = if let Command::State(task) = command {
                    let mut said = call::frame::said_state(answer::wire_state(state));
                    if let Some(item) = control.instance(task) {
                        said.task = item.task;
                        said.team = item.team.unwrap_or(env::TeamId::new(0));
                        said.completed = item.reason.is_some();
                        said.reason = item.reason.unwrap_or(0) as u64;
                    }
                    crate::system::assembly::JobOutcome::Command(said)
                } else {
                    match command {
                        Command::Embark(_) => {
                            crate::system::assembly::JobOutcome::Activate(work.reference)
                        }
                        Command::Debark(_) => {
                            crate::system::assembly::JobOutcome::Debark(work.reference)
                        }
                        Command::Ruin(_) => {
                            crate::system::assembly::JobOutcome::Retire(work.reference)
                        }
                        Command::State(_) => unreachable!(),
                    }
                };
                finish_command(&mut jobs, work, Ok(outcome));
            }
            Ok(None) => jobs.requeue_command(work),
            Err(fail) => finish_command(
                &mut jobs,
                work,
                Err(crate::system::assembly::JobFailure::Command(fail)),
            ),
        }
    }
    Ok(Progress::Done)
}

fn finish_command(
    jobs: &mut Jobs,
    work: crate::system::assembly::Command,
    result: Result<crate::system::assembly::JobOutcome, crate::system::assembly::JobFailure>,
) {
    let id = work.id;
    let expected = work.reference;
    jobs.complete_command(work, result);
    let Some(completion) = jobs.take(id) else {
        return;
    };
    let said = match completion.result.outcome {
        Ok(crate::system::assembly::JobOutcome::Command(said)) => said,
        Ok(
            crate::system::assembly::JobOutcome::Activate(reference)
            | crate::system::assembly::JobOutcome::Retire(reference)
            | crate::system::assembly::JobOutcome::Debark(reference),
        ) if reference == expected => call::frame::said_status(call::frame::OK),
        Ok(_) => call::frame::said_status(call::frame::fail_to_code(Some(call::Fail::Unknown))),
        Err(crate::system::assembly::JobFailure::Command(fail)) => {
            call::frame::said_status(call::frame::fail_to_code(Some(fail)))
        }
        Err(_) => call::frame::said_status(call::frame::FULL),
    };
    let crate::system::assembly::JobReply::Command { reply, waiters } = completion.result.reply
    else {
        return;
    };
    reply_command(reply, waiters, said);
}

struct ActivationFinish<'a> {
    control: &'a mut Control,
    work: crate::system::assembly::Command,
    result: Result<crate::system::assembly::JobOutcome, crate::system::assembly::JobFailure>,
}

fn finish_activation(jobs: &mut Jobs, finish: ActivationFinish<'_>) {
    let ActivationFinish {
        control,
        work,
        result,
    } = finish;
    let id = work.id;
    let expected = work.reference;
    jobs.complete_command(work, result);
    let Some(completion) = jobs.take(id) else {
        return;
    };
    let said = match completion.result.outcome {
        Ok(crate::system::assembly::JobOutcome::Activate(reference)) if reference == expected => {
            if control.activate_instance(reference) {
                call::frame::said_status(call::frame::OK)
            } else {
                call::frame::said_status(call::frame::fail_to_code(Some(call::Fail::Unknown)))
            }
        }
        Err(crate::system::assembly::JobFailure::Command(fail)) => {
            control.fail_activation(expected);
            call::frame::said_status(call::frame::fail_to_code(Some(fail)))
        }
        _ => call::frame::said_status(call::frame::fail_to_code(Some(call::Fail::Unknown))),
    };
    let crate::system::assembly::JobReply::Command { reply, waiters } = completion.result.reply
    else {
        return;
    };
    reply_command(reply, waiters, said);
}

fn reply_command(
    reply: Option<rpc::reply::Sender<call::frame::Said>>,
    waiters: alloc::vec::Vec<rpc::reply::Sender<call::frame::Said>>,
    said: call::frame::Said,
) {
    if let Some(reply) = reply {
        answer::reply(reply, said);
    }
    for waiter in waiters {
        answer::reply(waiter, said);
    }
}

pub fn receive(
    watch: Res<crate::system::control::namespace::Endpoints>,
    mut buffer: ResMut<answer::Buffer>,
    mut inbox: ResMut<Inbox>,
) -> Result<Progress, Fail> {
    let Some(entry) = watch.instance() else {
        return Ok(Progress::Done);
    };
    let receiver = RequestReceiver::<ControlContract>::from_raw(
        entry,
        ControlContract::BACK,
        ControlContract::back,
    );
    for _ in 0..16 {
        let request = match receiver.receive(&mut buffer.0, Wait::POLL) {
            Ok(request) => request,
            Err(rejected) => match rejected.fail {
                rpc::Fail::Receive(_) => break,
                _ => continue,
            },
        };
        let from = request.from;
        let (wire, _) = request.request;
        let reply = request.reply;
        match wire {
            Some(
                wire @ (call::frame::Wire::EmbarkInstance(_)
                | call::frame::Wire::DebarkInstance(_)
                | call::frame::Wire::RuinInstance(_)
                | call::frame::Wire::StateInstance(_)),
            ) => {
                if inbox.0.try_reserve(1).is_err() {
                    answer::reply(reply, call::frame::said_status(call::frame::FULL));
                } else {
                    inbox.0.push_back(answer::Incoming { wire, from, reply });
                }
            }
            _ => answer::reply(reply, call::frame::said_status(call::frame::DENIED)),
        }
    }
    Ok(Progress::Done)
}
