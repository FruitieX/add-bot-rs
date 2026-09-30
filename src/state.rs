use crate::{
    command::{RecentFormOptions, RecentFormStyle},
    types::{QueueId, Username},
};
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
    #[serde(default)]
    pub recent_form_styles: HashMap<Username, RecentFormStyle>,
}

impl State {
    /// Resolve a target's saved style and remember explicit choices made by that user.
    /// Returns whether the stored preference changed.
    pub fn resolve_recent_form_options(
        &mut self,
        requester: Option<&Username>,
        target: &Username,
        mut options: RecentFormOptions,
    ) -> (RecentFormOptions, bool) {
        let target_key = Username::new(target.to_string().to_ascii_lowercase());
        if options.explicit_style {
            let is_self = requester
                .map(|user| user.to_string().eq_ignore_ascii_case(&target.to_string()))
                .unwrap_or(false);
            if is_self && self.recent_form_styles.get(&target_key) != Some(&options.style) {
                self.recent_form_styles
                    .insert(target_key, options.style.clone());
                return (options, true);
            }
        } else {
            options.style = self
                .recent_form_styles
                .get(&target_key)
                .cloned()
                .unwrap_or_default();
        }
        (options, false)
    }

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

#[cfg(test)]
mod preference_tests {
    use super::*;

    fn user(name: &str) -> Username {
        Username::new(name.to_string())
    }

    fn explicit(style: RecentFormStyle) -> RecentFormOptions {
        RecentFormOptions {
            style,
            explicit_style: true,
            ..Default::default()
        }
    }

    #[test]
    fn custom_theme_is_saved_restored_and_cannot_be_changed_by_another_user() {
        let mut state = State::default();
        let alice = user("alice");
        let bob = user("bob");
        let style = RecentFormStyle::Custom(["👍🏽", "🇫🇮", "➖"].map(str::to_owned));
        let (resolved, changed) =
            state.resolve_recent_form_options(Some(&alice), &alice, explicit(style.clone()));
        assert_eq!(resolved.style, style);
        assert!(changed);
        let json = serde_json::to_string(&state).unwrap();
        let mut restored: State = serde_json::from_str(&json).unwrap();
        let (resolved, changed) =
            restored.resolve_recent_form_options(Some(&bob), &alice, RecentFormOptions::default());
        assert_eq!(resolved.style, style);
        assert!(!changed);
        let (_, changed) = restored.resolve_recent_form_options(
            Some(&bob),
            &alice,
            explicit(RecentFormStyle::Burger),
        );
        assert!(!changed);
        assert_eq!(restored.recent_form_styles.get(&alice), Some(&style));
        let (_, changed) = restored.resolve_recent_form_options(
            Some(&alice),
            &alice,
            explicit(RecentFormStyle::Squares),
        );
        assert!(changed);
        assert_eq!(
            restored.recent_form_styles.get(&alice),
            Some(&RecentFormStyle::Squares)
        );
    }

    #[test]
    fn recent_form_self_choice_is_saved_and_used_for_requests_by_others() {
        let mut state = State::default();
        let alice = user("Alice");
        let bob = user("Bob");
        let (_, changed) = state.resolve_recent_form_options(
            Some(&alice),
            &user("alice"),
            explicit(RecentFormStyle::Trophy),
        );
        assert!(changed);
        let options = RecentFormOptions {
            results_per_row: 5,
            ..Default::default()
        };
        let (resolved, changed) = state.resolve_recent_form_options(Some(&bob), &alice, options);
        assert_eq!(resolved.style, RecentFormStyle::Trophy);
        assert_eq!(resolved.results_per_row, 5);
        assert!(!changed);
        let (resolved, _) =
            state.resolve_recent_form_options(Some(&alice), &alice, RecentFormOptions::default());
        assert_eq!(resolved.style, RecentFormStyle::Trophy);
    }

    #[test]
    fn recent_form_other_users_can_override_output_but_cannot_change_preferences() {
        let mut state = State::default();
        let alice = user("alice");
        let bob = user("bob");
        state.resolve_recent_form_options(Some(&alice), &alice, explicit(RecentFormStyle::Trophy));
        for requester in [Some(&bob), None] {
            let (resolved, changed) = state.resolve_recent_form_options(
                requester,
                &alice,
                explicit(RecentFormStyle::Letters),
            );
            assert_eq!(resolved.style, RecentFormStyle::Letters);
            assert!(!changed);
            assert_eq!(
                state.recent_form_styles.get(&alice),
                Some(&RecentFormStyle::Trophy)
            );
        }
        // An explicit request for the original icon style is also a saved choice.
        let (_, changed) = state.resolve_recent_form_options(
            Some(&alice),
            &alice,
            explicit(RecentFormStyle::Squares),
        );
        assert!(changed);
        assert_eq!(
            state.recent_form_styles.get(&alice),
            Some(&RecentFormStyle::Squares)
        );
    }

    #[test]
    fn recent_form_preferences_survive_serialization_and_old_state_files_load() {
        let mut state: State = serde_json::from_str(r#"{"chats":{}}"#).unwrap();
        let alice = user("alice");
        let (options, changed) =
            state.resolve_recent_form_options(Some(&alice), &alice, RecentFormOptions::default());
        assert_eq!(options.style, RecentFormStyle::Squares);
        assert!(!changed);
        assert!(state.recent_form_styles.is_empty());
        state.resolve_recent_form_options(Some(&alice), &alice, explicit(RecentFormStyle::Mood));
        let json = serde_json::to_string(&state).unwrap();
        let mut restored: State = serde_json::from_str(&json).unwrap();
        let (options, _) =
            restored.resolve_recent_form_options(None, &alice, RecentFormOptions::default());
        assert_eq!(options.style, RecentFormStyle::Mood);
    }
}
