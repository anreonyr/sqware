use super::*;

impl RuntimeJob {
    pub fn test_running(id: JobId, task: TaskId, team: env::TeamId, root: RootValue) -> Self {
        let mut model = Job::prepare(id, 1);
        model.prepared().unwrap();
        model.start().unwrap();
        model.acknowledge(0).unwrap();
        Self {
            model,
            error: None,
            plan: Plan { commands: Vec::new(), links: Vec::new() },
            _root: root,
            members: vec![Member {
                built: Some(system_api::loader::Built { task, team }),
                bindings: Vec::new(),
                manifest: None,
                ran: true,
                dead: false,
                reason: None,
                stopping: false,
            }],
            pipes: Vec::new(),
            pumps: Vec::new(),
            work: None,
            build_index: 1,
            link_index: 0,
            manifest_index: 1,
            binding: None,
            pump_index: 0,
            input: VecDeque::new(),
            output: VecDeque::new(),
            eof: false,
            released: false,
        }
    }

    pub fn test_push_pump(&mut self, local: u64) {
        self.pumps.push(Pump {
            local, port: crate::pipe_client::Port, done: false,
            pending: Vec::new(), at: 0,
        });
    }

    pub fn test_member_dead(&self, index: usize) -> bool {
        self.members[index].dead
    }
}
