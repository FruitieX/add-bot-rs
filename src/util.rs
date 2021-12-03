use crate::state::ChatId;
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
pub fn mk_username(user: &User) -> String {
    user.username.clone().unwrap_or_else(|| {
        if let Some(last_name) = &user.last_name {
            format!("{} {}", user.first_name, last_name)
        } else {
            user.first_name.clone()
        }
    })
}

/// Formats a Chrono NaiveTime using our desired time format.
pub fn fmt_naive_time(t: &NaiveTime) -> String {
    t.format("%H:%M").to_string()
}

/// Helper for sending Telegram messages (and logging errors to stderr).
pub async fn send_msg(bot: &Bot, chat_id: ChatId, text: &str) {
    let res = bot
        .send_message(chat_id, text)
        .parse_mode(ParseMode::MarkdownV2)
        .send()
        .await;

    if let Err(error) = res {
        eprintln!("Error while sending Telegram message: {}", error);
    }
}
