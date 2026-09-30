use crate::util::ReplyContext;
use crate::{
    command::{RecentFormOptions, RecentFormStyle},
    types::{QueueId, Username},
};
use chrono::{DateTime, Duration, LocalResult, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;
use indexmap::IndexSet;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use teloxide::types::{ChatId, MessageId};

pub const QUEUE_SIZE: usize = 5;

/// Contains the set of players who have added up to a queue, along with a
/// timeout for when the queue expires.
#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct Queue {
    players: IndexSet<Username>,
    pub timeout: NaiveTime,
    pub add_cmd: String,
    #[serde(default)]
    pub expires_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub origin: Option<ReplyContext>,
}

impl Queue {
    pub fn new(timeout: NaiveTime, add_cmd: String) -> Queue {
        Queue {
            timeout,
            players: Default::default(),
            add_cmd,
            expires_at: None,
            origin: None,
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
#[derive(Clone, Deserialize, Serialize, Default, PartialEq, Eq)]
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
#[derive(Clone, Deserialize, Serialize, Default, PartialEq, Eq)]
pub struct State {
    pub chats: HashMap<ChatId, Chat>,
    #[serde(default)]
    pub recent_form_styles: HashMap<Username, RecentFormStyle>,
    #[serde(default)]
    pub schema_version: u32,
    #[serde(default)]
    pub next_notification_id: u64,
    #[serde(default)]
    pub pending_notifications: Vec<PendingNotification>,
    #[serde(default)]
    pub notification_receipts: Vec<NotificationReceipt>,
    #[serde(default)]
    pub telegram_retry_until: Option<DateTime<Utc>>,
}

pub const STATE_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct PendingNotification {
    pub id: u64,
    pub destination: ReplyContext,
    pub text: String,
    pub attempts: u32,
    pub next_attempt: DateTime<Utc>,
    pub blocked: bool,
    #[serde(default = "Utc::now")]
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub last_failure: Option<String>,
}

#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct NotificationReceipt {
    pub id: u64,
    pub message_id: MessageId,
}

/// Resolve the next local calendar occurrence. Choose the first occurrence
/// still in the future in an autumn overlap. Move a spring gap forward to the
/// first valid minute, instead of silently moving the request to another day.
pub fn next_local_deadline(time: NaiveTime, now: DateTime<Tz>) -> DateTime<Utc> {
    for offset in 0..=1 {
        let mut local = (now.date_naive() + Duration::days(offset)).and_time(time);
        for _ in 0..=180 {
            match now.timezone().from_local_datetime(&local) {
                LocalResult::Single(date) if date > now => return date.with_timezone(&Utc),
                LocalResult::Ambiguous(first, second) => {
                    if first > now {
                        return first.with_timezone(&Utc);
                    }
                    if second > now {
                        return second.with_timezone(&Utc);
                    }
                    break;
                }
                LocalResult::None => {
                    local += Duration::minutes(1);
                }
                _ => break,
            }
        }
    }
    // Some historical timezone transitions skip an entire local date. Find
    // the next valid date rather than falling back to an already-expired time.
    let tomorrow = now + Duration::days(1);
    next_local_deadline(time, tomorrow)
}

impl State {
    pub fn enqueue(&mut self, destination: ReplyContext, text: String, now: DateTime<Utc>) -> u64 {
        self.next_notification_id += 1;
        let id = self.next_notification_id;
        self.pending_notifications.push(PendingNotification {
            id,
            destination,
            text,
            attempts: 0,
            next_attempt: now,
            blocked: false,
            created_at: now,
            last_failure: None,
        });
        id
    }

    pub fn migrate(&mut self, tz: Tz, now: DateTime<Utc>) -> color_eyre::Result<()> {
        if self.schema_version > STATE_SCHEMA_VERSION {
            return Err(color_eyre::eyre::eyre!(
                "State schema {} is newer than this bot supports; state was not changed",
                self.schema_version
            ));
        }
        for chat in self.chats.values_mut() {
            for (id, queue) in &mut chat.queues {
                if queue.expires_at.is_none() {
                    queue.expires_at = Some(if id.is_instant_queue() {
                        now + Duration::minutes(30)
                    } else {
                        next_local_deadline(queue.timeout, now.with_timezone(&tz))
                    });
                }
                if let Some(deadline) = queue.expires_at {
                    queue.timeout = deadline.with_timezone(&tz).time();
                }
            }
        }
        self.schema_version = STATE_SCHEMA_VERSION;
        Ok(())
    }
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

#[cfg(test)]
mod deadline_tests {
    use super::*;
    use chrono::Timelike;

    #[test]
    fn next_day_is_a_local_calendar_day_across_dst() {
        let tz = chrono_tz::Europe::Helsinki;
        let now = tz.with_ymd_and_hms(2026, 3, 28, 20, 0, 0).unwrap();
        let due = next_local_deadline(NaiveTime::from_hms_opt(19, 30, 0).unwrap(), now);
        assert_eq!(
            due.with_timezone(&tz),
            tz.with_ymd_and_hms(2026, 3, 29, 19, 30, 0).unwrap()
        );
        assert_eq!((due - now.with_timezone(&Utc)).num_minutes(), 22 * 60 + 30);
    }

    #[test]
    fn nonexistent_spring_time_moves_to_the_first_valid_minute() {
        let tz = chrono_tz::Europe::Helsinki;
        let now = tz.with_ymd_and_hms(2026, 3, 29, 1, 0, 0).unwrap();
        let due =
            next_local_deadline(NaiveTime::from_hms_opt(3, 30, 0).unwrap(), now).with_timezone(&tz);
        assert_eq!(due.date_naive(), now.date_naive());
        assert_eq!((due.hour(), due.minute()), (4, 0));
    }

    #[test]
    fn repeated_autumn_time_uses_the_first_occurrence_still_in_the_future() {
        let tz = chrono_tz::Europe::Helsinki;
        let time = NaiveTime::from_hms_opt(3, 30, 0).unwrap();
        let before = tz.with_ymd_and_hms(2026, 10, 25, 2, 0, 0).unwrap();
        assert_eq!(
            next_local_deadline(time, before).to_rfc3339(),
            "2026-10-25T00:30:00+00:00"
        );
        let between = DateTime::parse_from_rfc3339("2026-10-25T00:45:00Z")
            .unwrap()
            .with_timezone(&tz);
        assert_eq!(
            next_local_deadline(time, between).to_rfc3339(),
            "2026-10-25T01:30:00+00:00"
        );
    }
}
