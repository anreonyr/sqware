use serde::Serialize;

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BootEvent {
    Launch { hart: usize },
    Done { hart: usize },
}
