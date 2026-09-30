use crate::{
    state::{AddRemovePlayerOp, Queue},
    types::Username,
};
use chrono::NaiveTime;
use teloxide::{
    payloads::SendMessageSetters,
    prelude::{Request, Requester},
    types::{ChatId, InputFile, ParseMode, User},
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
pub async fn send_msg(bot: &Bot, chat_id: &ChatId, text: &str) {
    let request = bot.send_message(*chat_id, text).parse_mode(ParseMode::Html);

    let res = request.send().await;

    if let Err(error) = res {
        eprintln!("Error while sending Telegram message: {}", error);
    }
}

/// Helper for sending Telegram photo (and logging errors to stderr).
pub async fn send_photo(bot: &Bot, chat_id: &ChatId, photo: InputFile) {
    let request = bot.send_photo(*chat_id, photo);

    let res = request.send().await;

    if let Err(error) = res {
        eprintln!("Error while sending Telegram photo message: {}", error);
    }
}

/// Constructs a status message describing current queue status.
pub fn mk_queue_status_msg(
    queue: &Queue,
    label: &str,
    op: &AddRemovePlayerOp,
    predicted_winrate: Option<&str>,
) -> String {
    let username = match op {
        AddRemovePlayerOp::PlayerAdded(username) | AddRemovePlayerOp::PlayerRemoved(username) => {
            escape_html(&username.to_string())
        }
    };
    if queue.num_players() == 0 {
        return format!("🎮 {label} · {username} left · Queue removed (empty)");
    }
    let action = match op {
        AddRemovePlayerOp::PlayerAdded(name) => {
            if queue.get_players().0.contains(name) {
                "joined"
            } else {
                "joined reserve"
            }
        }
        AddRemovePlayerOp::PlayerRemoved(_) => "left",
    };
    let players_str = mk_queue_roster(queue, false);
    let predicted_winrate = predicted_winrate
        .map(|value| format!("\n{value}"))
        .unwrap_or_default();

    format!(
        "🎮 {label} · {} · {username} {action}\n{players_str}{predicted_winrate}\nJoin/leave: {}",
        queue_occupancy(queue),
        queue.add_cmd,
    )
}

/// Creates a string containing the list of players in queue.
pub fn mk_queue_roster(queue: &Queue, highlight: bool) -> String {
    let (players, reserve) = queue.get_players();

    let fmt_usernames = |usernames: Vec<Username>| {
        usernames
            .iter()
            .map(|username| {
                if highlight
                    && username
                        .to_string()
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_')
                {
                    format!("@{}", escape_html(&username.to_string()))
                } else {
                    escape_html(&username.to_string())
                }
            })
            .collect::<Vec<String>>()
            .join(" · ")
    };

    let players = fmt_usernames(players);
    let players = if players.is_empty() {
        String::from("no players")
    } else {
        players
    };

    if let Some(reserve) = reserve {
        let reserve = reserve
            .iter()
            .map(|name| escape_html(&name.to_string()))
            .collect::<Vec<_>>()
            .join(" · ");
        format!("{players}\nReserve: {reserve}")
    } else {
        players
    }
}

pub fn queue_occupancy(queue: &Queue) -> String {
    format!("{}/{}", queue.num_players().min(queue.size()), queue.size())
}

pub fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveTime;

    #[test]
    fn queue_status_uses_dense_predicted_winrate_transition() {
        let mut queue = Queue::new(
            NaiveTime::from_hms_opt(20, 45, 0).unwrap(),
            "/2045".to_string(),
        );
        queue.insert_player(Username::new("Machofantastic".to_string()));

        let message = mk_queue_status_msg(
            &queue,
            "Today 20:45",
            &AddRemovePlayerOp::PlayerAdded(Username::new("Machofantastic".to_string())),
            Some("Predicted winrate: 54% → 48%"),
        );

        assert_eq!(
            message,
            "🎮 Today 20:45 · 1/5 · Machofantastic joined\nMachofantastic\nPredicted winrate: 54% → 48%\nJoin/leave: /2045"
        );
    }

    #[test]
    fn reserves_are_separate_unmentioned_and_do_not_inflate_occupancy() {
        let mut queue = Queue::new(NaiveTime::from_hms_opt(20, 45, 0).unwrap(), "/2045".into());
        for name in ["Alice", "Bobby", "Carol", "David", "Frank", "Grace"] {
            queue.insert_player(Username::new(name.into()));
        }
        assert_eq!(queue_occupancy(&queue), "5/5");
        assert_eq!(
            mk_queue_roster(&queue, true),
            "@Alice · @Bobby · @Carol · @David · @Frank\nReserve: Grace"
        );
        assert!(mk_queue_status_msg(
            &queue,
            "Today 20:45",
            &AddRemovePlayerOp::PlayerAdded(Username::new("Grace".into())),
            None
        )
        .starts_with("🎮 Today 20:45 · 5/5 · Grace joined reserve\n"));
    }

    #[test]
    fn empty_queue_message_has_no_stale_join_instruction() {
        let queue = Queue::new(NaiveTime::from_hms_opt(20, 45, 0).unwrap(), "/2045".into());
        assert_eq!(
            mk_queue_status_msg(
                &queue,
                "Today 20:45",
                &AddRemovePlayerOp::PlayerRemoved(Username::new("A&B".into())),
                Some("Predicted winrate: 50%")
            ),
            "🎮 Today 20:45 · A&amp;B left · Queue removed (empty)"
        );
    }
}
