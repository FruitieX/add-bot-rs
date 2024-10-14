use crate::types::{QueueId, Username};
use chrono::NaiveTime;
use indexmap::IndexSet;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use teloxide::types::ChatId;

pub const QUEUE_SIZE: usize = 5;

/// Contains the set of players who have added up to a queue, along with a
/// timeout for when the queue expires.
#[derive(Clone, Deserialize, Serialize)]
pub struct Queue {
    players: IndexSet<Username>,
    pub timeout: NaiveTime,
    pub add_cmd: String,
}

impl Queue {
    pub fn new(timeout: NaiveTime, add_cmd: String) -> Queue {
        Queue {
            timeout,
            players: Default::default(),
            add_cmd,
        }
    }

    /// Return whether queue has players or not.
    pub fn has_players(&self) -> bool {
        !self.players.is_empty()
    }

    /// Return number of players in queue.
    pub fn num_players(&self) -> usize {
        self.players.len()
    }

    /// Return whether queue is full or not.
    pub fn is_full(&self) -> bool {
        self.players.len() >= QUEUE_SIZE
    }

    /// Returns lists of players split into players and reserve players.
    pub fn get_players(&self) -> (Vec<Username>, Option<Vec<Username>>) {
        if self.players.len() > QUEUE_SIZE {
            // Split full queues into players and reserve players.
            let mut players = self.players.clone();
            let reserve = players.split_off(QUEUE_SIZE);
            (
                players.into_iter().collect(),
                Some(reserve.into_iter().collect()),
            )
        } else {
            (self.players.clone().into_iter().collect(), None)
        }
    }

    /// Returns size of this queue.
    pub fn size(&self) -> usize {
        QUEUE_SIZE
    }

    /// Insert player by username.
    pub fn insert_player(&mut self, username: Username) {
        self.players.insert(username);
    }

    /// Remove player by username.
    pub fn remove_player(&mut self, username: &Username) {
        self.players.shift_remove(username);
    }
}

/// A chat separates queues by Telegram groups.
#[derive(Clone, Deserialize, Serialize, Default)]
pub struct Chat {
    pub queues: HashMap<QueueId, Queue>,
}

pub enum AddRemovePlayerOp {
    PlayerAdded(Username),
    PlayerRemoved(Username),
}

impl std::fmt::Display for AddRemovePlayerOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            AddRemovePlayerOp::PlayerAdded(username) => format!("Added {}", username),
            AddRemovePlayerOp::PlayerRemoved(username) => format!("Removed {}", username),
        };

        write!(f, "{}", s)
    }
}

pub enum AddRemovePlayerResult {
    QueueEmpty(Queue),
    PlayerQueued(Queue),
    QueueFull(Queue),
}

/// (De)Serializable state containing chats with active queues.
#[derive(Clone, Deserialize, Serialize, Default)]
pub struct State {
    pub chats: HashMap<ChatId, Chat>,
}

impl State {
    /// Removes a given chat queue.
    pub fn rm_chat_queue(&self, chat_id: &ChatId, queue_id: &QueueId) -> (State, Option<Queue>) {
        let mut state = self.clone();

        let chat = state.chats.get_mut(chat_id);
        let queue = chat.and_then(|chat| chat.queues.remove(queue_id));

        (state, queue)
    }

    /// Adds/removes player from given chat queue.
    ///
    /// Removes and returns the queue once it's full.
    pub fn add_remove_player(
        &self,
        chat_id: &ChatId,
        queue_id: &QueueId,
        add_cmd: String,
        timeout: NaiveTime,
        username: Username,
    ) -> (State, AddRemovePlayerResult, AddRemovePlayerOp) {
        let mut state = self.clone();

        // Ensure both chat and queue exists in respective HashMaps.
        let chat = state.chats.entry(*chat_id).or_default();
        let queue = chat
            .queues
            .entry(queue_id.clone())
            .or_insert_with(|| Queue::new(timeout, add_cmd));

        let op = if queue.players.contains(&username) {
            // Remove the player.
            queue.remove_player(&username);
            AddRemovePlayerOp::PlayerRemoved(username)
        } else {
            // Add the player
            queue.insert_player(username.clone());
            AddRemovePlayerOp::PlayerAdded(username)
        };

        let queue_player_count = queue.players.len();

        let result = match queue_player_count {
            0 => {
                // Remove queue if it's empty after remove operation.
                let queue = chat.queues.remove(queue_id).unwrap();
                AddRemovePlayerResult::QueueEmpty(queue)
            }
            x if x >= QUEUE_SIZE => {
                if queue_id.is_instant_queue() {
                    // Remove instant queue once it's full.
                    let queue = chat.queues.remove(queue_id).unwrap();
                    AddRemovePlayerResult::QueueFull(queue)
                } else {
                    AddRemovePlayerResult::QueueFull(queue.clone())
                }
            }
            _ => AddRemovePlayerResult::PlayerQueued(queue.clone()),
        };

        (state, result, op)
    }

    /// Removes player from all chat queues.
    ///
    /// Returns a tuple of new State and affected queue_ids.
    pub fn rm_player(
        &self,
        chat_id: &ChatId,
        username: &Username,
    ) -> (State, HashMap<QueueId, Queue>) {
        let mut state = self.clone();

        let chat = state.chats.get_mut(chat_id);

        // Maintain a list of queues affected by remove operation.
        let mut affected_queues = HashMap::new();

        if let Some(chat) = chat {
            chat.queues = chat
                .queues
                .iter_mut()
                .map(|(queue_id, queue)| {
                    // Remove player from all chat queues
                    let removed = queue.players.shift_remove(username);

                    if removed {
                        affected_queues.insert(queue_id.clone(), queue.clone());
                    }

                    (queue_id.clone(), queue.clone())
                })
                .filter(|(_, queue)| {
                    // Filter out empty queues
                    queue.has_players()
                })
                .collect();
        }

        (state, affected_queues)
    }
}
