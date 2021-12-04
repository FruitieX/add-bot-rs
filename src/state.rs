use crate::types::{ChatId, QueueId, UserId, Username};
use chrono::NaiveTime;
use serde::{Deserialize, Serialize};
use std::collections::{hash_map::Entry, HashMap};

/// Contains the set of players who have added up to a queue, along with a
/// timeout for when the queue expires.
#[derive(Clone, Deserialize, Serialize)]
pub struct Queue {
    pub players: HashMap<UserId, Username>,
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
    pub fn has_players(&self) -> bool {
        !self.players.is_empty()
    }
}

pub const QUEUE_SIZE: usize = 5;

/// A chat separates queues by Telegram groups.
#[derive(Clone, Deserialize, Serialize, Default)]
pub struct Chat {
    pub queues: HashMap<QueueId, Queue>,
}

pub enum AddRemovePlayerOp {
    PlayerAdded,
    PlayerRemoved,
}

impl std::fmt::Display for AddRemovePlayerOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            AddRemovePlayerOp::PlayerAdded => "Added to",
            AddRemovePlayerOp::PlayerRemoved => "Removed from",
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
    pub fn rm_chat_queue(&self, chat_id: &ChatId, queue_id: &QueueId) -> State {
        let mut state = self.clone();

        let chat = state.chats.get_mut(chat_id);
        chat.map(|chat| chat.queues.remove(queue_id));

        state
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
        user: (UserId, Username),
    ) -> (State, AddRemovePlayerResult, AddRemovePlayerOp) {
        let mut state = self.clone();

        // Ensure both chat and queue exists in respective HashMaps.
        let chat = state.chats.entry(*chat_id).or_insert_with(Chat::default);
        let queue = chat
            .queues
            .entry(queue_id.clone())
            .or_insert_with(|| Queue::new(timeout, add_cmd));

        let mut op = AddRemovePlayerOp::PlayerAdded;
        if let Entry::Vacant(e) = queue.players.entry(user.0.clone()) {
            // Add the player and keep timeout up to date.
            e.insert(user.1);
            queue.timeout = timeout;
        } else {
            // Remove the player.
            queue.players.remove(&user.0);
            op = AddRemovePlayerOp::PlayerRemoved;
        }

        let queue_player_count = queue.players.len();

        let result = match queue_player_count {
            0 => {
                // Remove queue if it's empty after remove operation.
                let queue = chat.queues.remove(queue_id).unwrap();
                AddRemovePlayerResult::QueueEmpty(queue)
            }
            x if x >= QUEUE_SIZE => {
                // Remove queue once it's full. Store removed queue in full_queue.
                let queue = chat.queues.remove(queue_id).unwrap();
                AddRemovePlayerResult::QueueFull(queue)
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
        user_id: &UserId,
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
                    let removed = queue.players.remove(user_id);

                    if removed.is_some() {
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
