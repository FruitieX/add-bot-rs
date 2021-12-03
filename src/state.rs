use anyhow::Result;
use chrono::NaiveTime;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;

pub type QueueId = String;
pub type ChatId = i64;
pub type UserId = i64;
pub type Username = String;

/// Contains the set of players who have added up to a queue, along with a
/// timeout for when the queue expires.
#[derive(Clone, Deserialize, Serialize)]
pub struct Queue {
    players: HashMap<UserId, Username>,
    timeout: NaiveTime,
}

/// A chat separates queues by Telegram groups.
#[derive(Clone, Deserialize, Serialize)]
pub struct Chat {
    queues: HashMap<QueueId, Queue>,
}

/// (De)Serializable state containing chats with active queues.
#[derive(Clone, Deserialize, Serialize, Default)]
pub struct State {
    chats: HashMap<ChatId, Chat>,
}

/// Path of the state file, relative to CWD.
const STATE_FILE_PATH: &str = "state.json";

/// Contains bot state, plus logic for persisting state to disk.
#[derive(Clone, Default)]
pub struct StateContainer {
    state: Arc<RwLock<State>>,
}

impl StateContainer {
    /// Try restoring state from file.
    ///
    /// Fails if there was an error while deserializing from JSON, in other
    /// error cases returns default (empty) state.
    pub async fn try_read_from_file() -> Result<StateContainer, serde_json::Error> {
        let file = tokio::fs::read_to_string(STATE_FILE_PATH).await;

        match file {
            Ok(json) => {
                let state: State = serde_json::from_str(&json)?;
                let state = Arc::new(RwLock::new(state));
                Ok(StateContainer { state })
            }
            Err(_) => Ok(Default::default()),
        }
    }

    /// Returns current state of the RwLock.
    pub async fn read(&self) -> State {
        self.state.read().await.clone()
    }

    /// Writes new state to the RwLock and JSON state file.
    pub async fn write(&self, state: State) {
        // Only hold onto RwLock inside this block
        {
            let mut unlocked_state = self.state.write().await;
            *unlocked_state = state.clone();
        }

        let json = serde_json::to_string(&state).unwrap();
        let file_res = tokio::fs::write(STATE_FILE_PATH, json).await;
        if let Err(error) = file_res {
            eprintln!("Error while writing state file: {}", error);
        }
    }
}
