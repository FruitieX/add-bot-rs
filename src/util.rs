use crate::{
    state::{Queue, QUEUE_SIZE},
    types::{ChatId, QueueId, Username},
};
use chrono::NaiveTime;
use teloxide::{
    payloads::SendMessageSetters,
    prelude::{Request, Requester},
    types::{ParseMode, User},
    Bot,
};

/// Tries in order to extract a user's:
///
/// - Username if it exists
/// - First and last names if last name exists
/// - First name
pub fn mk_username(user: &User) -> Username {
    let str = user.username.clone().unwrap_or_else(|| {
        if let Some(last_name) = &user.last_name {
            format!("{} {}", user.first_name, last_name)
        } else {
            user.first_name.clone()
        }
    });

    Username::new(str)
}

/// Formats a Chrono NaiveTime using our desired time format.
pub fn fmt_naive_time(t: &NaiveTime) -> String {
    t.format("%H:%M").to_string()
}

/// Helper for sending Telegram messages (and logging errors to stderr).
pub async fn send_msg(bot: &Bot, chat_id: &ChatId, text: &str, markdown: bool) {
    let chat_id: i64 = (*chat_id).into();

    let request = if markdown {
        // Telegram wants me to escape these characters in this ParseMode.
        let text = text.replace("-", r"\-");
        let text = text.replace(".", r"\.");

        bot.send_message(chat_id, text)
            .parse_mode(ParseMode::MarkdownV2)
    } else {
        bot.send_message(chat_id, text).parse_mode(ParseMode::Html)
    };

    let res = request.send().await;

    if let Err(error) = res {
        eprintln!("Error while sending Telegram message: {}", error);
    }
}

/// Constructs a status message describing current queue status.
pub fn mk_queue_status_msg(queue: &Queue, queue_id: &QueueId) -> String {
    let total_players = queue.players.len();
    let queue_size = QUEUE_SIZE;

    format!(
        "{}/{} added to {} queue. Use {} to add/remove yourself from the queue!",
        total_players, queue_size, queue_id, queue.add_cmd,
    )
}
