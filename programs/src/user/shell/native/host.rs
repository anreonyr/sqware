use super::super::{
    adapt::{
        controller::Controller,
        ports::{Buffer, LocalPort},
    },
    core::{
        Command, CommandId, Direction, Endpoint, JobId, JobStatus, Link, MemberResult, Plan,
        PortDecl,
    },
};
use super::objects::{Object, Objects};
use alloc::{collections::VecDeque, format, string::String, vec, vec::Vec};
use env::Wait;
use lisp::{
    Arity, Call, Diagnostic, Engine, ForeignId, HostError, ReadState, Reader, RootValue, Source,
    Step, ValueKind,
};
const MEMORY: usize = 4 * 1024 * 1024;
const SPECS: &[(&str, Arity)] = &[
    (
        "buffer",
        Arity {
            min: 1,
            max: Some(2),
        },
    ),
    ("terminal", Arity::fixed(1)),
    ("bytes", Arity::fixed(1)),
    (
        "read",
        Arity {
            min: 1,
            max: Some(3),
        },
    ),
    ("write", Arity::fixed(2)),
    (
        "wait",
        Arity {
            min: 1,
            max: Some(2),
        },
    ),
    ("close", Arity::fixed(1)),
    ("command", Arity::fixed(2)),
    ("connect", Arity::fixed(2)),
    ("prepare", Arity::fixed(1)),
    ("start", Arity::fixed(1)),
    ("pause", Arity::fixed(1)),
    ("resume", Arity::fixed(1)),
    ("cancel", Arity::fixed(1)),
    ("foreground", Arity::fixed(1)),
    ("background", Arity::fixed(1)),
    ("jobs", Arity::fixed(0)),
    ("status", Arity::fixed(1)),
];
enum Goal {
    Prepared,
    Running,
    Paused,
    Ended,
    Wait,
}
enum Waiting {
    Job {
        id: JobId,
        goal: Goal,
        deadline: u64,
        value: Option<RootValue>,
    },
    Read {
        port: u64,
        count: usize,
        deadline: u64,
    },
    Port {
        port: u64,
        deadline: u64,
    },
}
struct Effect {
    call: Call,
    waiting: Waiting,
}
enum Outcome {
    Ready(RootValue),
    Pending(Waiting),
}
pub struct Host {
    pub objects: Objects,
    pub controller: Controller,
    pending: Option<Effect>,
    pub input: VecDeque<u8>,
    pub eof: bool,
    pub output: VecDeque<u8>,
    next_command: u64,
}
impl Host {
    pub fn new(controller: Controller) -> Self {
        Self {
            objects: Objects::default(),
            controller,
            pending: None,
            input: VecDeque::new(),
            eof: false,
            output: VecDeque::new(),
            next_command: 0,
        }
    }
    pub fn install(engine: &mut Engine) -> Result<(), Diagnostic> {
        for (operation, (name, arity)) in SPECS.iter().enumerate() {
            engine.register(name, operation, *arity)?;
        }
        let source = include_str!("stdlib.lisp");
        let mut at = 0;
        loop {
            match Reader::default().read(Source::new("<shell-stdlib>", &source[at..]), true)? {
                ReadState::Complete { form, consumed } => {
                    engine.start(form)?;
                    loop {
                        match engine.step(1024) {
                            Step::Done(_) => break,
                            Step::Failed(error) => return Err(error),
                            Step::Yielded => {}
                            Step::Request(_) => unreachable!(),
                        }
                    }
                    at += consumed;
                }
                ReadState::End => return Ok(()),
                ReadState::More => unreachable!(),
            }
        }
    }
    fn foreign(
        &mut self,
        engine: &mut Engine,
        object: Object,
        kind: &str,
    ) -> Result<RootValue, String> {
        let id = self.objects.insert(object).map_err(String::from)?;
        match engine.foreign(id, kind) {
            Ok(value) => Ok(value),
            Err(error) => {
                self.objects.remove(id);
                Err(error.render())
            }
        }
    }
    fn id(&self, engine: &Engine, value: &RootValue) -> Result<ForeignId, String> {
        engine
            .foreign_id(value)
            .map_err(|e| e.render())?
            .filter(|id| self.objects.get(*id).is_some())
            .ok_or("expected live shell object".into())
    }
    fn job_id(&self, engine: &Engine, value: &RootValue) -> Result<JobId, String> {
        match self
            .objects
            .get(self.id(engine, value)?)
            .ok_or("missing job")?
        {
            Object::Job(id) => Ok(*id),
            _ => Err("expected job".into()),
        }
    }
    fn port_key(&self, engine: &Engine, value: &RootValue) -> Result<u64, String> {
        let id = self.id(engine, value)?;
        if matches!(self.objects.get(id), Some(Object::Port(_))) {
            Ok(Objects::key(id))
        } else {
            Err("expected port".into())
        }
    }
    fn direction(engine: &Engine, value: &RootValue) -> Result<Direction, String> {
        match engine.text(value).map_err(|e| e.render())? {
            Some("read") => Ok(Direction::Read),
            Some("write") => Ok(Direction::Write),
            _ => Err("direction must be read or write".into()),
        }
    }
    fn list(engine: &Engine, value: &RootValue) -> Result<Vec<RootValue>, String> {
        let mut cursor = value.clone();
        let mut values = Vec::new();
        loop {
            if engine.kind(&cursor).map_err(|e| e.render())? == ValueKind::Nil {
                return Ok(values);
            }
            let (first, rest) = engine
                .pair(&cursor)
                .map_err(|e| e.render())?
                .ok_or("expected proper list")?;
            if values.len() >= 4096 {
                return Err("list limit exceeded".into());
            }
            values.push(first);
            cursor = rest;
        }
    }
    fn endpoint(&self, engine: &Engine, value: &RootValue) -> Result<Endpoint, String> {
        if engine.kind(value).map_err(|e| e.render())? == ValueKind::Foreign {
            return Ok(Endpoint::Port {
                id: self.port_key(engine, value)?,
            });
        }
        let parts = Self::list(engine, value)?;
        if parts.len() != 2 {
            return Err("command endpoint requires command and port name".into());
        }
        let Object::Command(command) = self
            .objects
            .get(self.id(engine, &parts[0])?)
            .ok_or("missing command")?
        else {
            return Err("expected command endpoint".into());
        };
        let port = engine
            .text(&parts[1])
            .map_err(|e| e.render())?
            .ok_or("port name must be a symbol or string")?;
        Ok(Endpoint::Command {
            id: command.id,
            port: String::from(port),
        })
    }
    fn deadline(value: Option<&RootValue>) -> Result<u64, String> {
        let within = if let Some(value) = value {
            Wait::AtMost(
                usize::try_from(value.integer().ok_or("timeout must be an integer")?)
                    .map_err(|_| "negative timeout")?,
            )
        } else {
            Wait::Forever
        };
        Ok(ipc::time::deadline(within))
    }
    fn tagged(
        engine: &mut Engine,
        tag: &str,
        value: Option<RootValue>,
    ) -> Result<RootValue, String> {
        let mut values = vec![engine.symbol(tag).map_err(|e| e.render())?];
        if let Some(value) = value {
            values.push(value);
        }
        engine.list(&values).map_err(|e| e.render())
    }
    fn status(&self, engine: &mut Engine, id: JobId) -> Result<RootValue, String> {
        let job = self.controller.job(id).ok_or("unknown job")?;
        let label = engine
            .symbol(job.model.status.name())
            .map_err(|e| e.render())?;
        let mut results = Vec::new();
        for result in &job.model.results {
            results.push(match result {
                Some(MemberResult::Exited(reason)) => engine.integer(*reason as i64),
                Some(MemberResult::NotStarted) => {
                    engine.symbol("not-started").map_err(|e| e.render())?
                }
                None => engine.boolean(false),
            });
        }
        let results = engine.list(&results).map_err(|e| e.render())?;
        let mut values = vec![label, results];
        if job.model.status == JobStatus::Paused {
            values.push(
                engine
                    .symbol(match job.model.pause_reason {
                        super::super::core::PauseReason::Requested => "requested",
                        super::super::core::PauseReason::BackgroundRead => "background-read",
                    })
                    .map_err(|e| e.render())?,
            );
        }
        engine.list(&values).map_err(|e| e.render())
    }
    fn dispatch(&mut self, engine: &mut Engine, call: &Call) -> Result<Outcome, String> {
        let name = SPECS
            .get(call.operation)
            .ok_or("unknown native operation")?
            .0;
        let args = &call.arguments;
        let ready = |value| Ok(Outcome::Ready(value));
        match name {
            "buffer" => {
                let direction = Self::direction(engine, &args[0])?;
                let bytes = if let Some(value) = args.get(1) {
                    engine
                        .byte_slice(value)
                        .map_err(|e| e.render())?
                        .ok_or("buffer content must be bytes")?
                        .to_vec()
                } else {
                    Vec::new()
                };
                if self.objects.used() + bytes.len() > MEMORY {
                    return Err("buffer limit exceeded".into());
                }
                let buffer = match direction {
                    Direction::Read => Buffer::Read { bytes, at: 0 },
                    Direction::Write => Buffer::Write { bytes },
                };
                ready(self.foreign(
                    engine,
                    Object::Port(LocalPort {
                        buffer,
                        closed: false,
                        bound: None,
                    }),
                    "port",
                )?)
            }
            "terminal" => {
                let direction = Self::direction(engine, &args[0])?;
                ready(self.foreign(
                    engine,
                    Object::Port(LocalPort {
                        buffer: Buffer::Terminal(direction),
                        closed: false,
                        bound: None,
                    }),
                    "port",
                )?)
            }
            "bytes" => {
                let port = self
                    .objects
                    .port(self.port_key(engine, &args[0])?)
                    .ok_or("missing port")?;
                let bytes = port.bytes().ok_or("port is not a memory buffer")?;
                ready(engine.bytes(bytes).map_err(|e| e.render())?)
            }
            "read" => {
                let port = self.port_key(engine, &args[0])?;
                let count = match args.get(1) {
                    Some(value) => {
                        usize::try_from(value.integer().ok_or("read count must be an integer")?)
                            .map_err(|_| "negative read count")?
                    }
                    None => 256,
                };
                if count > MEMORY {
                    return Err("read count limit exceeded".into());
                }
                let local = self.objects.port(port).ok_or("missing port")?;
                if local.bound.is_some() || local.direction() != Direction::Read {
                    return Err("port is bound or not readable".into());
                }
                if count == 0 {
                    let empty = engine.bytes(&[]).map_err(|e| e.render())?;
                    return ready(Self::tagged(engine, "data", Some(empty))?);
                }
                Ok(Outcome::Pending(Waiting::Read {
                    port,
                    count,
                    deadline: Self::deadline(args.get(2))?,
                }))
            }
            "write" => {
                let key = self.port_key(engine, &args[0])?;
                let available = MEMORY
                    .saturating_sub(self.objects.used())
                    .saturating_sub(self.output.len());
                let bytes = engine
                    .byte_slice(&args[1])
                    .map_err(|e| e.render())?
                    .ok_or("write requires bytes; encode strings explicitly")?;
                let local = self.objects.port_mut(key).ok_or("missing port")?;
                if local.bound.is_some() {
                    return Err("port is connected to a job".into());
                }
                let n = local
                    .write(bytes, &mut self.output, available)
                    .map_err(String::from)?;
                ready(engine.integer(n as i64))
            }
            "close" => {
                let key = self.port_key(engine, &args[0])?;
                self.objects.port_mut(key).ok_or("missing port")?.closed = true;
                ready(engine.nil())
            }
            "wait" => {
                let deadline = Self::deadline(args.get(1))?;
                let id = self.id(engine, &args[0])?;
                match self.objects.get(id).ok_or("missing object")? {
                    Object::Job(job) => Ok(Outcome::Pending(Waiting::Job {
                        id: *job,
                        goal: Goal::Wait,
                        deadline,
                        value: None,
                    })),
                    Object::Port(_) => Ok(Outcome::Pending(Waiting::Port {
                        port: Objects::key(id),
                        deadline,
                    })),
                    _ => Err("wait requires port or job".into()),
                }
            }
            "command" => {
                let name = engine
                    .text(&args[0])
                    .map_err(|e| e.render())?
                    .ok_or("command name must be a symbol or string")?;
                let image = self
                    .controller
                    .catalogue
                    .images
                    .iter()
                    .find(|image| image.name == name)
                    .ok_or("command image not supplied")?;
                let arguments = Self::list(engine, &args[1])?;
                if arguments.len() > shell_api::MAX_ARGS {
                    return Err("too many program arguments".into());
                }
                let mut strings = Vec::new();
                let mut size = 0;
                for value in arguments {
                    if engine.kind(&value).map_err(|e| e.render())? != ValueKind::String {
                        return Err("program arguments must be strings".into());
                    }
                    let arg = engine
                        .text(&value)
                        .map_err(|e| e.render())?
                        .ok_or("expected string")?;
                    size += arg.len();
                    if size > shell_api::MAX_SIZE {
                        return Err("program argument limit exceeded".into());
                    }
                    strings.push(String::from(arg));
                }
                self.next_command = self
                    .next_command
                    .checked_add(1)
                    .ok_or("command identifiers exhausted")?;
                let command = Command {
                    id: CommandId(self.next_command),
                    image: image.name.clone(),
                    args: strings,
                    ports: image
                        .ports
                        .iter()
                        .map(|(name, direction)| PortDecl {
                            name: name.clone(),
                            direction: *direction,
                        })
                        .collect(),
                };
                ready(self.foreign(engine, Object::Command(command), "command")?)
            }
            "connect" => {
                let mut commands = Vec::new();
                for value in Self::list(engine, &args[0])? {
                    let Object::Command(command) = self
                        .objects
                        .get(self.id(engine, &value)?)
                        .ok_or("missing command")?
                    else {
                        return Err("expected command list".into());
                    };
                    commands.push(command.clone());
                }
                let mut links = Vec::new();
                for value in Self::list(engine, &args[1])? {
                    let ends = Self::list(engine, &value)?;
                    if ends.len() != 2 {
                        return Err("link requires source and sink".into());
                    }
                    links.push(Link {
                        source: self.endpoint(engine, &ends[0])?,
                        sink: self.endpoint(engine, &ends[1])?,
                    });
                }
                let plan = Plan::connect(commands, links, |key| {
                    self.objects.port(key).map(LocalPort::direction)
                })
                .map_err(|e| format!("invalid plan: {e:?}"))?;
                ready(self.foreign(
                    engine,
                    Object::Plan {
                        plan,
                        roots: args.to_vec(),
                    },
                    "plan",
                )?)
            }
            "prepare" => {
                let Object::Plan { plan, .. } = self
                    .objects
                    .get(self.id(engine, &args[0])?)
                    .ok_or("missing plan")?
                else {
                    return Err("prepare requires a plan".into());
                };
                let plan = plan.clone();
                let id = self
                    .controller
                    .prepare(plan, args[0].clone(), &mut self.objects)
                    .map_err(String::from)?;
                let value = match self.foreign(engine, Object::Job(id), "job") {
                    Ok(value) => value,
                    Err(error) => {
                        let _ = self.controller.cancel(id);
                        return Err(error);
                    }
                };
                Ok(Outcome::Pending(Waiting::Job {
                    id,
                    goal: Goal::Prepared,
                    deadline: u64::MAX,
                    value: Some(value),
                }))
            }
            "start" | "pause" | "resume" | "cancel" => {
                let id = self.job_id(engine, &args[0])?;
                let goal = match name {
                    "start" => {
                        self.controller.start(id).map_err(String::from)?;
                        Goal::Running
                    }
                    "pause" => {
                        self.controller
                            .pause(id, super::super::core::PauseReason::Requested)
                            .map_err(String::from)?;
                        Goal::Paused
                    }
                    "resume" => {
                        self.controller.resume(id).map_err(String::from)?;
                        Goal::Running
                    }
                    _ => {
                        self.controller.cancel(id).map_err(String::from)?;
                        Goal::Ended
                    }
                };
                Ok(Outcome::Pending(Waiting::Job {
                    id,
                    goal,
                    deadline: ipc::time::deadline(Wait::AtMost(5000)),
                    value: Some(args[0].clone()),
                }))
            }
            "foreground" | "background" => {
                let id = self.job_id(engine, &args[0])?;
                if name == "foreground" {
                    self.controller.foreground(id)
                } else {
                    self.controller.background(id)
                }
                .map_err(String::from)?;
                ready(args[0].clone())
            }
            "jobs" => {
                let ids: Vec<_> = self.controller.jobs().collect();
                let mut values = Vec::new();
                for id in ids {
                    values.push(self.foreign(engine, Object::Job(id), "job")?);
                }
                ready(engine.list(&values).map_err(|e| e.render())?)
            }
            "status" => ready(self.status(engine, self.job_id(engine, &args[0])?)?),
            _ => Err("unknown shell function".into()),
        }
    }
    pub fn request(&mut self, engine: &mut Engine, call: Call) -> Result<(), Diagnostic> {
        match self.dispatch(engine, &call) {
            Ok(Outcome::Ready(value)) => engine.resume(call.id, Ok(value)),
            Ok(Outcome::Pending(waiting)) => {
                self.pending = Some(Effect { call, waiting });
                Ok(())
            }
            Err(error) => engine.resume(call.id, Err(HostError(error))),
        }
    }
    fn poll(
        &mut self,
        engine: &mut Engine,
        waiting: &Waiting,
    ) -> Result<Option<RootValue>, String> {
        match waiting {
            Waiting::Read {
                port,
                count,
                deadline,
            } => {
                let local = self.objects.port_mut(*port).ok_or("missing port")?;
                match local
                    .read(*count, &mut self.input, self.eof)
                    .map_err(String::from)?
                {
                    Some(bytes) if bytes.is_empty() => Self::tagged(engine, "eof", None).map(Some),
                    Some(bytes) => {
                        let data = engine.bytes(&bytes).map_err(|e| e.render())?;
                        Self::tagged(engine, "data", Some(data)).map(Some)
                    }
                    None if env::chrono::clock() >= *deadline => {
                        Self::tagged(engine, "pending", None).map(Some)
                    }
                    None => Ok(None),
                }
            }
            Waiting::Port { port, deadline } => {
                let local = self.objects.port(*port).ok_or("missing port")?;
                if local.closed {
                    return Err("port closed".into());
                }
                if local.bound.is_some() {
                    return Err("port is connected to a job".into());
                }
                let ready = match &local.buffer {
                    Buffer::Read { .. } => true,
                    Buffer::Write { .. } => true,
                    Buffer::Terminal(Direction::Read) => !self.input.is_empty() || self.eof,
                    Buffer::Terminal(Direction::Write) => true,
                };
                if ready || env::chrono::clock() >= *deadline {
                    Ok(Some(engine.boolean(ready)))
                } else {
                    Ok(None)
                }
            }
            Waiting::Job {
                id,
                goal,
                deadline,
                value,
            } => {
                let job = self.controller.job(*id).ok_or("unknown job")?;
                let status = job.model.status;
                if status == JobStatus::Failed {
                    return Err(job.error.clone().unwrap_or("job failed".into()));
                }
                let ready = match goal {
                    Goal::Prepared => status == JobStatus::Prepared,
                    Goal::Running => {
                        matches!(status, JobStatus::Running | JobStatus::Paused)
                            || status.terminal()
                    }
                    Goal::Paused => status == JobStatus::Paused || status.terminal(),
                    Goal::Ended => status.terminal(),
                    Goal::Wait => status.terminal() || status == JobStatus::Paused,
                };
                if ready {
                    match value {
                        Some(value) => Ok(Some(value.clone())),
                        None => self.status(engine, *id).map(Some),
                    }
                } else if env::chrono::clock() >= *deadline {
                    if matches!(goal, Goal::Wait) {
                        Self::tagged(engine, "pending", None).map(Some)
                    } else {
                        Err("job transition timed out".into())
                    }
                } else {
                    Ok(None)
                }
            }
        }
    }
    pub fn advance(&mut self, engine: &mut Engine) -> Result<(), Diagnostic> {
        self.controller.advance(&mut self.objects);
        if let Some(effect) = self.pending.take() {
            match self.poll(engine, &effect.waiting) {
                Ok(Some(value)) => engine.resume(effect.call.id, Ok(value))?,
                Ok(None) => self.pending = Some(effect),
                Err(error) => engine.resume(effect.call.id, Err(HostError(error)))?,
            }
        }
        for id in engine.take_released() {
            self.objects.remove(id);
        }
        self.controller.reclaim(&self.objects);
        Ok(())
    }
    pub fn awaiting_input(&self) -> bool {
        matches!(self.pending.as_ref().map(|effect| &effect.waiting), Some(Waiting::Read { port, .. } | Waiting::Port { port, .. }) if matches!(self.objects.port(*port).map(|port| &port.buffer), Some(Buffer::Terminal(Direction::Read))))
    }
    pub fn cancel_evaluation(&mut self, engine: &mut Engine) {
        if let Some(Effect {
            waiting:
                Waiting::Job {
                    id,
                    goal: Goal::Prepared,
                    ..
                },
            ..
        }) = self.pending.take()
        {
            let _ = self.controller.cancel(id);
        }
        self.input.clear();
        self.eof = false;
        engine.cancel();
    }
}
