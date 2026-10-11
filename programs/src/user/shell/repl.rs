use super::{adapt::{controller::Controller, waiting::{Interests, Waiting}}, native::Host};
use alloc::{format, vec::Vec};
use env::{TaskId, Wait};
use lisp::{Engine, Limits, ReadState, Reader, Source, Step};
use terminal_client::{Io, Read};
const WAIT: Wait = Wait::AtMost(5000);
fn catalogue(owner: TaskId) -> Result<shell_api::Catalogue, &'static str> {
    let entry = resource::raw::pies()
        .find(|info| {
            info.alive
                && info.owner == owner
                && info.vestor == owner
                && info.kind == env::PieKind::Hole
                && info.mark == shell_api::CATALOGUE
        })
        .ok_or("shell catalogue missing")?;
    let mut bytes = alloc::vec![0; shell_api::MAX_SIZE];
    let (n, from) = resource::raw::Hole::from_raw(entry.token)
        .pull(&mut bytes, WAIT)
        .map_err(|_| "shell catalogue receive")?;
    if from != owner {
        return Err("shell catalogue source");
    }
    let catalogue =
        shell_api::Catalogue::decode(&bytes[..n]).map_err(|_| "shell catalogue format")?;
    for image in &catalogue.images {
        let info = resource::raw::inspect(image.seed).map_err(|_| "shell image capability")?;
        if !info.alive
            || info.owner != owner
            || info.vestor != owner
            || info.kind != env::PieKind::Pole
            || !info
                .permission
                .contains(env::Permission::FETCH | env::Permission::VEST)
        {
            return Err("shell image source");
        }
    }
    let _ = env::pie::release(entry.token, env::ReleaseMode::Revoke);
    Ok(catalogue)
}
fn queue(host: &mut Host, bytes: &[u8]) {
    if host.output.len() + bytes.len() <= 4 * 1024 * 1024 {
        host.output.extend(bytes);
    }
}
pub fn run() -> Result<(), &'static str> {
    let args = execution::boot::args::args();
    let terminal = TaskId::new(*args.first().ok_or("shell terminal owner")?);
    let account = TaskId::new(*args.get(1).ok_or("shell catalogue owner")?);
    let io = Io::injected(terminal).map_err(|_| "shell terminal")?;
    let catalogue = catalogue(account)?;
    let loader = resource::raw::pies()
        .find(|info| info.alive && info.mark == system_api::loader::Grant::Build.mark())
        .ok_or("shell loader grant")?;
    let sire = loader.owner;
    let loader = loader.token;
    let session = ipc::session::Session::open(sire, system_client::operator::BERTH, WAIT)
        .map_err(|_| "shell operator")?;
    let tree = system_client::operator::Face::of(session);
    let pipe = pipe_client::Client::find(&tree, WAIT).map_err(|_| "shell pipe service")?;
    let mut engine = Engine::new(Limits::default()).map_err(|_| "shell language allocation")?;
    Host::install(&mut engine).map_err(|_| "shell standard library")?;
    let mut host = Host::new(Controller::new(catalogue, pipe, loader));
    queue(&mut host, b"sqware Lisp Shell\n");
    let grant = system_api::identity::Grant::Resolve;
    let entry = resource::raw::pies()
        .find(|info| info.mark == grant.mark())
        .ok_or("shell identity resolver")?;
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
            } else { output_blocked = true; }
        } else if let Some((id, bytes)) = host.controller.output() {
            let n = bytes.len();
            if io.try_write(bytes).map_err(|_| "shell job output")? {
                host.controller.consume_output(id, n);
                progressed = true;
            } else { output_blocked = true; }
        }
        if exiting
            && host.controller.settled()
            && host.output.is_empty()
            && host.controller.output().is_none()
        {
            io.drain().map_err(|_| "shell output drain")?;
            return Ok(());
        }
        if engine.is_runnable() || progressed
            || !exiting && engine.is_idle() && host.controller.foreground.is_none() && !prompt {
            env::room::starve();
            continue;
        }
        let mut interests = Interests::new();
        let [input, output, control] = io.raw_channels();
        interests.source(env::Source::Mail { pie: input, condition: env::MailCondition::Pull });
        interests.source(env::Source::Mail { pie: control, condition: env::MailCondition::Pull });
        if output_blocked { interests.source(env::Source::Mail { pie: output, condition: env::MailCondition::Push }); }
        interests.budget(host.remaining());
        host.controller.interests(&host.objects, &mut interests);
        if interests.runnable { env::room::starve(); continue; }
        waiting.apply(&interests)?;
        waiting.await_(interests.within)?;
    }
}
