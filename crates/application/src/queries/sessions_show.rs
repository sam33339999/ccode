use crate::error::AppError;
use ccode_domain::{message::Role, session::SessionId};
use ccode_ports::repositories::SessionRepository;

#[derive(Debug, Clone)]
pub struct SessionMessageView {
    pub id: String,
    pub role: String,
    pub content: String,
    pub created_at: u64,
}

#[derive(Debug, Clone)]
pub struct SessionView {
    pub id: String,
    pub created_at: u64,
    pub updated_at: u64,
    pub message_count: usize,
    pub messages: Vec<SessionMessageView>,
}

pub struct SessionsShowQuery<R> {
    repo: R,
}

impl<R: SessionRepository> SessionsShowQuery<R> {
    pub fn new(repo: R) -> Self {
        Self { repo }
    }

    pub async fn execute(&self, id: String) -> Result<Option<SessionView>, AppError> {
        let session = self.repo.find_by_id(&SessionId(id)).await?;
        Ok(session.map(|s| SessionView {
            id: s.id.0,
            created_at: s.created_at,
            updated_at: s.updated_at,
            message_count: s.messages.len(),
            messages: s
                .messages
                .into_iter()
                .map(|m| SessionMessageView {
                    id: m.id.0,
                    role: role_label(&m.role).to_string(),
                    content: m.content,
                    created_at: m.created_at,
                })
                .collect(),
        }))
    }
}

fn role_label(role: &Role) -> &'static str {
    match role {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use ccode_domain::{
        message::{Message, Role},
        session::{Session, SessionSummary},
    };
    use ccode_ports::PortError;

    struct MockRepo(Option<Session>);

    #[async_trait]
    impl SessionRepository for MockRepo {
        async fn list(&self, _limit: usize) -> Result<Vec<SessionSummary>, PortError> {
            Ok(vec![])
        }
        async fn find_by_id(&self, _id: &SessionId) -> Result<Option<Session>, PortError> {
            Ok(self.0.clone())
        }
        async fn save(&self, _session: &Session) -> Result<(), PortError> {
            Ok(())
        }
        async fn delete(&self, _id: &SessionId) -> Result<(), PortError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn returns_none_when_session_missing() {
        let result = SessionsShowQuery::new(MockRepo(None))
            .execute("missing".to_string())
            .await
            .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn maps_full_session_to_view() {
        let mut session = Session::new("s-1", 1000);
        session.add_message(Message::new("m1", Role::User, "hi", 2000), 2000);
        session.add_message(Message::new("m2", Role::Assistant, "hello", 3000), 3000);

        let result = SessionsShowQuery::new(MockRepo(Some(session)))
            .execute("s-1".to_string())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(result.id, "s-1");
        assert_eq!(result.message_count, 2);
        assert_eq!(result.messages[0].id, "m1");
        assert_eq!(result.messages[0].role, "user");
        assert_eq!(result.messages[1].role, "assistant");
    }
}
