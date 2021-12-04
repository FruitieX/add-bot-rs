use crate::state::State;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Path of the state file, relative to CWD.
const STATE_FILE_PATH: &str = "state.json";

/// Handles reading/write state from/to both memory and disk.
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
    /// Note that due to .clone() the caller does not end up holding the RwLock.
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
