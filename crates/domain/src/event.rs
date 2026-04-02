use crate::{
    assistant_mode::{AssistantMode, ModeSwitchTrigger},
    message::Message,
    session::SessionId,
};

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
    AssistantModeSwitched {
        session_id: SessionId,
        from_mode: AssistantMode,
        to_mode: AssistantMode,
        trigger: ModeSwitchTrigger,
    },
}
