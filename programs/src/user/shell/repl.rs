use super::{
    adapt::{
        controller::Controller,
        waiting::{Interests, Waiting},
    },
    native::Host,
};
use alloc::{format, string::String, vec::Vec};
use env::{TaskId, Wait};
use lisp::{Engine, Limits, ReadState, Reader, Source, Step};
use terminal_client::{Io, Read};
const WAIT: Wait = Wait::AtMost(5000);
fn catalogue(system: TaskId) -> Result<shell_api::Catalogue, &'static str> {
    use wire::Message;
    let mut images = Vec::new();
    for command in super::commands::COMMANDS {
        let channel = ipc::session::establish::claim(
            system,
            crate::unit::descriptor_mark(command.binding.slot),
            WAIT,
        )
        .map_err(|_| "shell resource channel")?;
        let mut frame_bytes = crate::unit::SupplyFrame::EMPTY;
        let (n, from) = resource::raw::Hole::from_raw(channel)
            .pull(&mut frame_bytes, WAIT)
            .map_err(|_| "shell resource receive")?;
        if from != system {
            return Err("shell resource source");
        }
        let frame =
            crate::unit::SupplyFrame::fetch(&frame_bytes[..n]).ok_or("shell resource frame")?;
        let description = &frame.description[..usize::from(frame.length)];
        let encoded_length: [u8; 8] = description
            .get(..8)
            .ok_or("shell resource descriptor")?
            .try_into()
            .map_err(|_| "shell resource descriptor")?;
        let length = usize::try_from(u64::from_le_bytes(encoded_length))
            .map_err(|_| "shell resource length")?;
        if length == 0 || length > 16 * 1024 * 1024 {
            return Err("shell resource length");
        }
        let seed = resource::raw::pies()
            .find(|info| {
                info.alive
                    && info.owner == system
                    && info.vestor == system
                    && info.kind == env::PieKind::Pole
                    && info.mark == command.binding.slot
                    && info.permission.contains(command.binding.permission)
            })
            .map(|info| info.token)
            .ok_or("shell image resource")?;
        images.push(shell_api::Image {
            name: String::from(command.name),
            seed,
            mark: command.binding.slot,
            length,
            ports: command
                .ports
                .iter()
                .map(|(name, direction)| (String::from(*name), *direction))
                .collect(),
        });
    }
    Ok(shell_api::Catalogue { images })
}
fn queue(host: &mut Host, bytes: &[u8]) {
    if host.output.len() + bytes.len() <= 4 * 1024 * 1024 {
        host.output.extend(bytes);
    }
}
pub fn run() -> Result<(), &'static str> {
    let args = execution::boot::args::args();
    let terminal = TaskId::new(*args.first().ok_or("shell terminal owner")?);
    let login = env::unit::sire();
    let system = super::startup::receive(login)?;
    let io = Io::injected(terminal).map_err(|_| "shell terminal")?;
    let loader_info = resource::raw::pies()
        .find(|info| {
            info.alive
                && info.owner == system
                && info.vestor == system
                && info.mark == system_api::loader::Grant::Claim.mark()
                && info.kind == env::PieKind::Hole
                && info.permission.contains(env::Permission::STORE)
        })
        .ok_or("shell claim grant")?;
    let loader = loader_info.token;
    let construction = resource::raw::pies()
        .find(|info| {
            info.alive
                && info.mark == system_api::control::construction::ENTRY
                && info.owner == system
                && info.vestor == system
                && info.kind == env::PieKind::Hole
                && info.permission.contains(env::Permission::STORE)
        })
        .ok_or("shell construction grant")?
        .token;
    let catalogue = catalogue(system)?;
    let session = ipc::session::Session::open(system, system_client::operator::BERTH, WAIT)
        .map_err(|_| "shell operator")?;
    let tree = system_client::operator::Face::of(session);
    let pipe = pipe_client::Client::find(&tree, WAIT).map_err(|_| "shell pipe service")?;
    let service = tree
        .resolve(system_api::namespace::SERVICE, WAIT)
        .map_err(|_| "shell service namespace")?;
    let lifecycle = system_client::control::Face::of(
        tree.tile_from(service, system_api::control::INSTANCE, WAIT)
            .and_then(|tile| tile.token(WAIT))
            .map_err(|_| "shell instance endpoint")?,
    )
    .map_err(|_| "shell lifecycle")?;
    let mut engine = Engine::new(Limits::default()).map_err(|_| "shell language allocation")?;
    Host::install(&mut engine).map_err(|_| "shell standard library")?;
    let grant = system_api::identity::Grant::Resolve;
    let authority = system_client::identity::authority().ok_or("shell identity authority")?;
    let entry = resource::raw::pies()
        .find(|info| {
            info.alive
                && info.owner == authority
                && info.vestor == login
                && info.mark == grant.mark()
                && info.kind == env::PieKind::Hole
                && info.permission.contains(env::Permission::STORE)
        })
        .ok_or("shell identity resolver")?;
    let identity_authority = entry.owner;
    let resolver = system_client::identity::Face::direct(entry.owner, grant, entry.token)
        .map_err(|_| "shell identity resolver")?;
    let system_api::identity::Reply::Binding(Some(binding)) = resolver
        .call(
            system_api::identity::Wire::Resolve(env::unit::self_id()),
            WAIT,
        )
        .map_err(|_| "shell identity")?
    else {
        return Err("shell identity missing");
    };
    let mut host = Host::new(Controller::new(
        catalogue,
        pipe,
        construction,
        loader,
        binding.current,
        identity_authority,
        lifecycle,
    ));
    queue(&mut host, b"Shuyra Lisp Shell\n");
    queue(
        &mut host,
        format!(
            "shell: task={} principal={}:{}\n",
            env::unit::self_id().get(),
            binding.current.principal.authority.get(),
            binding.current.principal.slot
        )
        .as_bytes(),
    );
    let mut waiting = Waiting::new()?;
    let mut input = Vec::new();
    let mut prompt = false;
    let mut exiting = false;
    let mut suspended = None;
    loop {
        let mut progressed = false;
        let mut output_blocked = false;
        for _ in 0..4 {
            let Some(event) = io
                .read_with(Wait::POLL)
                .map_err(|_| "shell terminal input")?
            else {
                break;
            };
            progressed = true;
            match event {
                Read::Data(bytes) => {
                    if host.controller.foreground.is_some() {
                        host.controller.input(bytes.bytes(), false);
                    } else if host.awaiting_input() {
                        host.input.extend(bytes.bytes());
                    } else {
                        if input.len() + bytes.bytes().len() > Limits::default().input {
                            input.clear();
                            queue(&mut host, b"input limit exceeded\n");
                        } else {
                            input.extend_from_slice(bytes.bytes());
                        }
                        prompt = false;
                    }
                }
                Read::Eof => {
                    if host.controller.foreground.is_some() {
                        host.controller.input(&[], true);
                    } else if host.awaiting_input() {
                        host.eof = true;
                    } else {
                        exiting = true;
                        input.clear();
                        host.cancel_evaluation(&mut engine);
                        host.controller.cancel_all();
                    }
                }
                Read::Interrupt => {
                    suspended = None;
                    input.clear();
                    if let Some(id) = host.controller.foreground {
                        host.cancel_evaluation(&mut engine);
                        prompt = false;
                        let _ = host.controller.cancel(id);
                    } else {
                        host.cancel_evaluation(&mut engine);
                        queue(&mut host, b"\n");
                        prompt = false;
                    }
                }
                Read::Suspend => {
                    if let Some(id) = host.controller.foreground {
                        input.clear();
                        host.cancel_evaluation(&mut engine);
                        suspended = Some(id);
                        prompt = false;
                        let _ = host
                            .controller
                            .pause(id, super::core::PauseReason::Requested);
                    }
                }
            }
        }
        if let Err(error) = host.advance(&mut engine) {
            queue(&mut host, format!("{}\n", error.render()).as_bytes());
            prompt = false;
        }
        if let Some(id) = suspended {
            if host.controller.job(id).is_some_and(|job| {
                job.model.status == super::core::JobStatus::Paused || job.model.status.terminal()
            }) {
                match host.status(&mut engine, id) {
                    Ok(value) => match engine.display(&value) {
                        Ok(text) => queue(&mut host, format!("{text}\n").as_bytes()),
                        Err(error) => queue(&mut host, format!("{}\n", error.render()).as_bytes()),
                    },
                    Err(error) => queue(&mut host, format!("{error}\n").as_bytes()),
                }
                suspended = None;
            }
        }
        if engine.is_idle() && !exiting && host.controller.foreground.is_none() {
            if input.is_empty() {
                if !prompt {
                    queue(&mut host, b"lisp> ");
                    prompt = true;
                }
            } else {
                match core::str::from_utf8(&input) {
                    Ok(text) => match Reader::default().read(Source::new("<repl>", text), false) {
                        Ok(ReadState::Complete { form, consumed }) => {
                            progressed = true;
                            input.drain(..consumed);
                            if let Err(error) = engine.start(form) {
                                queue(&mut host, format!("{}\n", error.render()).as_bytes());
                            }
                            prompt = false;
                        }
                        Ok(ReadState::End) => {
                            input.clear();
                        }
                        Ok(ReadState::More) => {
                            if input.last() == Some(&b'\n') && !prompt {
                                queue(&mut host, b"....> ");
                                prompt = true;
                            }
                        }
                        Err(error) => {
                            input.clear();
                            queue(&mut host, format!("{}\n", error.render()).as_bytes());
                            prompt = false;
                        }
                    },
                    Err(error) if error.error_len().is_none() => {}
                    Err(_) => {
                        input.clear();
                        queue(&mut host, b"invalid UTF-8 input\n");
                        prompt = false;
                    }
                }
            }
        }
        if !exiting {
            match engine.step(1024) {
                Step::Yielded => {}
                Step::Request(call) => {
                    if let Err(error) = host.request(&mut engine, call) {
                        queue(&mut host, format!("{}\n", error.render()).as_bytes());
                        prompt = false;
                    }
                }
                Step::Done(value) => {
                    match engine.display(&value) {
                        Ok(text) => queue(&mut host, format!("{text}\n").as_bytes()),
                        Err(error) => queue(&mut host, format!("{}\n", error.render()).as_bytes()),
                    }
                    prompt = false;
                }
                Step::Failed(error) => {
                    queue(&mut host, format!("{}\n", error.render()).as_bytes());
                    prompt = false;
                }
            }
        }
        let (first, second) = host.output.as_slices();
        let output = if first.is_empty() { second } else { first };
        if !output.is_empty() {
            let n = output.len().min(256);
            if io
                .try_write(&output[..n])
                .map_err(|_| "shell terminal output")?
            {
                for _ in 0..n {
                    host.output.pop_front();
                }
                progressed = true;
            } else {
                output_blocked = true;
            }
        } else if let Some((id, bytes)) = host.controller.output() {
            let n = bytes.len();
            if io.try_write(bytes).map_err(|_| "shell job output")? {
                host.controller.consume_output(id, n);
                progressed = true;
            } else {
                output_blocked = true;
            }
        }
        if exiting
            && host.controller.settled()
            && host.output.is_empty()
            && host.controller.output().is_none()
        {
            io.drain().map_err(|_| "shell output drain")?;
            return Ok(());
        }
        if engine.is_runnable()
            || progressed
            || !exiting && engine.is_idle() && host.controller.foreground.is_none() && !prompt
        {
            env::room::starve();
            continue;
        }
        let mut interests = Interests::new();
        let [input, output, control] = io.raw_channels();
        interests.source(env::ToleEventSource::Hole {
            token: input,
            condition: env::HoleCondition::Pull,
        });
        interests.source(env::ToleEventSource::Hole {
            token: control,
            condition: env::HoleCondition::Pull,
        });
        if output_blocked {
            interests.source(env::ToleEventSource::Hole {
                token: output,
                condition: env::HoleCondition::Push,
            });
        }
        interests.budget(host.remaining());
        host.controller.interests(&host.objects, &mut interests);
        if interests.runnable {
            env::room::starve();
            continue;
        }
        waiting.apply(&interests)?;
        waiting.await_(interests.within)?;
    }
}
