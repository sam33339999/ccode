use crate::{message::Message, session::SessionId};

#[derive(Debug, Clone)]
pub enum DomainEvent {
    SessionCreated {
        session_id: SessionId,
    },
    MessageAdded {
        session_id: SessionId,
        message: Message,
    },
    SessionDeleted {
        session_id: SessionId,
    },
}
