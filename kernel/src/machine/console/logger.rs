struct KernelLogger;
static LOGGER: KernelLogger = KernelLogger;

impl log::Log for KernelLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Error
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            super::_write(format_args!("[{}] {}\n", record.level(), record.args()));
        }
    }

    fn flush(&self) {}
}

pub fn init() {
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(log::LevelFilter::Error);
}
