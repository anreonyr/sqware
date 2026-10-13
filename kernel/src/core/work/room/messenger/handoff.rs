pub enum Handoff<T> {
    Resume(T),
    Switch(usize),
}
