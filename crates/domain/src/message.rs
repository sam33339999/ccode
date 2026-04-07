use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MessageId(pub String);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    pub media_type: String,
    pub data: AttachmentData,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AttachmentData {
    Base64(String),
    Url(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: MessageId,
    pub role: Role,
    pub content: String,
    /// Unix timestamp (ms)
    pub created_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachments: Option<Vec<Attachment>>,
}

impl Message {
    pub fn new(
        id: impl Into<String>,
        role: Role,
        content: impl Into<String>,
        created_at: u64,
    ) -> Self {
        Self {
            id: MessageId(id.into()),
            role,
            content: content.into(),
            created_at,
            tool_calls: None,
            tool_call_id: None,
            attachments: None,
        }
    }

    pub fn new_tool_result(
        id: impl Into<String>,
        tool_call_id: impl Into<String>,
        content: impl Into<String>,
        created_at: u64,
    ) -> Self {
        Self {
            id: MessageId(id.into()),
            role: Role::Tool,
            content: content.into(),
            created_at,
            tool_calls: None,
            tool_call_id: Some(tool_call_id.into()),
            attachments: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Attachment, AttachmentData, Message, Role};
    use serde_json::json;

    #[test]
    fn deserializes_legacy_message_without_attachments() {
        let legacy = json!({
            "id": "m1",
            "role": "user",
            "content": "hello",
            "created_at": 1
        });

        let message: Message = serde_json::from_value(legacy).expect("deserialize legacy message");
        assert!(message.attachments.is_none());
    }

    #[test]
    fn serializes_attachment_and_skips_none() {
        let mut message = Message::new("m1", Role::User, "hello", 1);
        message.attachments = Some(vec![
            Attachment {
                media_type: "image/png".to_string(),
                data: AttachmentData::Base64("YmFzZTY0".to_string()),
            },
            Attachment {
                media_type: "image/webp".to_string(),
                data: AttachmentData::Url("https://example.com/a.webp".to_string()),
            },
        ]);

        let with_attachments = serde_json::to_value(&message).expect("serialize message");
        assert!(with_attachments.get("attachments").is_some());

        message.attachments = None;
        let without_attachments = serde_json::to_value(&message).expect("serialize message");
        assert!(without_attachments.get("attachments").is_none());
    }
}
