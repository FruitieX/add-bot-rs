use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
pub struct QueueId(String);

impl QueueId {
    pub fn new(id: String) -> QueueId {
        QueueId(id)
    }

    pub fn is_instant_queue(&self) -> bool {
        self.0.is_empty()
    }
}

impl std::fmt::Display for QueueId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = if self.is_instant_queue() {
            "Instant"
        } else {
            &self.0
        };

        write!(f, "{}", s)
    }
}

#[derive(Copy, Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
pub struct ChatId(i64);

impl ChatId {
    pub fn new(id: i64) -> ChatId {
        ChatId(id)
    }
}

impl From<ChatId> for i64 {
    fn from(id: ChatId) -> Self {
        id.0
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
pub struct Username(String);

impl Username {
    pub fn new(str: String) -> Username {
        Username(str)
    }
}

impl std::fmt::Display for Username {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
