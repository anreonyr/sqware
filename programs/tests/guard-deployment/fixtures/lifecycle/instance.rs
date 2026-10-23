pub struct Instance {
    pub service: crate::service::Service,
    pub launched: bool,
    pub identity_install: Option<u64>,
}

impl Instance {
    pub fn new(service: crate::service::Service) -> Self {
        Self {
            service,
            launched: false,
            identity_install: None,
        }
    }
}
