use serde::Serialize;

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvEvent {
    Call { call: usize, arg: usize },
}
