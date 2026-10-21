pub enum RoomEvent {
    Wait { tid: usize, key: usize },
}

impl RoomEvent {
    #[allow(non_snake_case)]
    pub fn Room(event: Self) -> Self { event }
}
