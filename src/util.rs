use crate::{
    state::{AddRemovePlayerOp, Queue},
    types::{QueueId, Username},
};
use chrono::NaiveTime;
use teloxide::{
    payloads::SendMessageSetters,
    prelude::{Request, Requester},
    types::{ChatId, ParseMode, User},
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
    let request = if markdown {
        // Telegram wants me to escape these (and probably some other)
        // characters in this ParseMode.
        let text = text.replace('-', r"\-");
        let text = text.replace('.', r"\.");

        bot.send_message(*chat_id, text)
            .parse_mode(ParseMode::MarkdownV2)
    } else {
        bot.send_message(*chat_id, text).parse_mode(ParseMode::Html)
    };

    let res = request.send().await;

    if let Err(error) = res {
        eprintln!("Error while sending Telegram message: {}", error);
    }
}

/// Constructs a status message describing current queue status.
pub fn mk_queue_status_msg(queue: &Queue, queue_id: &QueueId, op: &AddRemovePlayerOp) -> String {
    let players_str = mk_players_str(queue, false, false);

    format!(
        "{} queue: {}.\n{}.\nUse {} to add/remove yourself from the queue!",
        queue_id, op, players_str, queue.add_cmd,
    )
}

/// Creates a string containing the list of players in queue.
pub fn mk_players_str(queue: &Queue, highlight: bool, short: bool) -> String {
    let (players, reserve) = queue.get_players();

    let fmt_usernames = |usernames: Vec<Username>| {
        usernames
            .iter()
            .map(|username| {
                if highlight {
                    format!("@{}", username)
                } else {
                    username.to_string()
                }
            })
            .collect::<Vec<String>>()
            .join(", ")
    };

    let player_count = format!("{}/{}", queue.num_players(), queue.size());

    let players = fmt_usernames(players);
    let players = if players.is_empty() {
        String::from("no players")
    } else {
        players
    };

    let reserve = reserve.map(fmt_usernames);

    let title = if short { "" } else { "Players: " };

    if let Some(reserve) = reserve {
        format!(
            "{}{} ({}, Reserve: {})",
            title, player_count, players, reserve
        )
    } else {
        format!("{}{} ({})", title, player_count, players)
    }
}
