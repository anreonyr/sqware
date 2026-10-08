use crate::Message;

/// A typed request and response pair over a byte transport.
pub trait Contract {
    type Request: Message;
    type Response: Message;
}
