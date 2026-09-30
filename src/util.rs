use crate::{
    state::{AddRemovePlayerOp, Queue},
    types::Username,
};
use chrono::NaiveTime;
use serde::{Deserialize, Serialize};
use teloxide::{
    payloads::{EditMessageTextSetters, SendMessageSetters, SendPhotoSetters},
    prelude::{Request, Requester},
    types::{ChatId, InputFile, Message, MessageId, ParseMode, ReplyParameters, ThreadId, User},
    ApiError, Bot, RequestError,
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

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
pub struct ReplyContext {
    pub chat_id: ChatId,
    pub thread_id: Option<ThreadId>,
    pub reply_to: Option<MessageId>,
}
impl ReplyContext {
    pub fn from_message(message: &Message) -> Self {
        Self {
            chat_id: message.chat.id,
            thread_id: message.thread_id,
            reply_to: Some(message.id),
        }
    }
}

pub fn telegram_retry_delay(error: &RequestError, attempt: u32) -> Option<std::time::Duration> {
    let seconds = match error {
        RequestError::RetryAfter(seconds) => seconds.seconds().max(1) as u64,
        RequestError::Network(error)
            if error.status().is_none_or(|status| status.is_server_error()) =>
        {
            2u64.pow(attempt.min(9)).min(300)
        }
        RequestError::InvalidJson { .. } => 2u64.pow(attempt.min(9)).min(300),
        RequestError::Api(ApiError::Unknown(message))
            if ["Internal Server Error", "Bad Gateway", "Gateway Timeout"]
                .iter()
                .any(|text| message.contains(text)) =>
        {
            2u64.pow(attempt.min(9)).min(300)
        }
        _ => return None,
    };
    Some(std::time::Duration::from_secs(seconds))
}

/// Keep credentials and raw responses out of delivery diagnostics and state.
pub fn telegram_error_summary(error: &RequestError) -> String {
    match error {
        RequestError::Network(error) if error.is_timeout() => "Telegram request timed out".into(),
        RequestError::Network(error) if error.is_connect() => "Telegram connection failed".into(),
        RequestError::Network(error) => {
            format!("Telegram network failure (status={:?})", error.status())
        }
        RequestError::InvalidJson { .. } => "Invalid Telegram response".into(),
        RequestError::RetryAfter(seconds) => format!(
            "Telegram rate limit; retry after {} seconds",
            seconds.seconds()
        ),
        RequestError::Api(error) => error.to_string(),
        RequestError::MigrateToChatId(chat) => {
            format!("Telegram group migrated to {chat}; destination needs updating")
        }
        RequestError::Io(_) => "Telegram upload I/O failure".into(),
    }
}

async fn retry_request<T, F: std::future::Future<Output = Result<T, RequestError>>>(
    mut request: impl FnMut() -> F,
) -> Result<T, RequestError> {
    for attempt in 0..3 {
        match request().await {
            Ok(result) => return Ok(result),
            Err(error) => match telegram_retry_delay(&error, attempt + 1)
                .filter(|delay| delay.as_secs() <= 30 && attempt < 2)
            {
                Some(delay) => tokio::time::sleep(delay).await,
                None => return Err(error),
            },
        }
    }
    unreachable!()
}

pub async fn send_msg_once(
    bot: &Bot,
    destination: ReplyContext,
    text: &str,
) -> Result<Message, RequestError> {
    let mut request = bot
        .send_message(destination.chat_id, text)
        .parse_mode(ParseMode::Html);
    if let Some(thread) = destination.thread_id {
        request = request.message_thread_id(thread);
    }
    if let Some(reply) = destination.reply_to {
        request =
            request.reply_parameters(ReplyParameters::new(reply).allow_sending_without_reply());
    }
    request.send().await
}

pub async fn send_msg(
    bot: &Bot,
    destination: ReplyContext,
    text: &str,
) -> Result<Message, RequestError> {
    retry_request(|| send_msg_once(bot, destination, text)).await
}

pub async fn send_photo(
    bot: &Bot,
    destination: ReplyContext,
    photo: InputFile,
) -> Result<Message, RequestError> {
    retry_request(|| {
        let mut request = bot.send_photo(destination.chat_id, photo.clone());
        if let Some(thread) = destination.thread_id {
            request = request.message_thread_id(thread);
        }
        if let Some(reply) = destination.reply_to {
            request =
                request.reply_parameters(ReplyParameters::new(reply).allow_sending_without_reply());
        }
        request.send()
    })
    .await
}

pub async fn edit_msg(
    bot: &Bot,
    destination: ReplyContext,
    id: MessageId,
    text: &str,
) -> Result<(), RequestError> {
    match retry_request(|| {
        bot.edit_message_text(destination.chat_id, id, text)
            .parse_mode(ParseMode::Html)
            .send()
    })
    .await
    {
        Ok(_) | Err(RequestError::Api(ApiError::MessageNotModified)) => Ok(()),
        Err(error) => Err(error),
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

    fn telegram_fixture() -> (Bot, std::thread::JoinHandle<(String, Vec<u8>)>) {
        use std::io::{BufRead, Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(std::time::Duration::from_secs(3)))
                .unwrap();
            let mut reader = std::io::BufReader::new(&mut socket);
            let mut headers = String::new();
            loop {
                let mut line = String::new();
                assert!(reader.read_line(&mut line).unwrap() > 0);
                if line == "\r\n" {
                    break;
                }
                headers.push_str(&line);
            }
            let lower = headers.to_ascii_lowercase();
            let mut body = Vec::new();
            if let Some(length) = lower
                .lines()
                .find_map(|line| line.strip_prefix("content-length:").map(str::trim))
            {
                body.resize(length.parse().unwrap(), 0);
                reader.read_exact(&mut body).unwrap();
            } else if lower.contains("transfer-encoding: chunked") {
                loop {
                    let mut size = String::new();
                    reader.read_line(&mut size).unwrap();
                    let size =
                        usize::from_str_radix(size.trim().split(';').next().unwrap(), 16).unwrap();
                    if size == 0 {
                        break;
                    }
                    let offset = body.len();
                    body.resize(offset + size, 0);
                    reader.read_exact(&mut body[offset..]).unwrap();
                    let mut newline = [0; 2];
                    reader.read_exact(&mut newline).unwrap();
                }
            }
            drop(reader);
            let response = r#"{"ok":true,"result":{"message_id":23,"date":0,"chat":{"id":1,"type":"private","first_name":"Test"},"text":"ok"}}"#;
            write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).unwrap();
            (headers, body)
        });
        let client = teloxide::net::default_reqwest_settings()
            .no_proxy()
            .timeout(std::time::Duration::from_secs(3))
            .build()
            .unwrap();
        let bot = Bot::with_client("123:LOCAL_TEST", client)
            .set_api_url(reqwest::Url::parse(&format!("http://{address}")).unwrap());
        (bot, server)
    }

    #[tokio::test]
    async fn text_delivery_passes_topic_and_reply_context_to_telegram() {
        let (bot, server) = telegram_fixture();
        let destination = ReplyContext {
            chat_id: ChatId(1),
            thread_id: Some(ThreadId(MessageId(7))),
            reply_to: Some(MessageId(12)),
        };
        assert_eq!(
            send_msg(&bot, destination, "<b>Hello</b>")
                .await
                .unwrap()
                .id,
            MessageId(23)
        );
        let (_, body) = server.join().unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload["message_thread_id"], 7);
        assert_eq!(payload["reply_parameters"]["message_id"], 12);
        assert_eq!(
            payload["reply_parameters"]["allow_sending_without_reply"],
            true
        );
        assert_eq!(payload["parse_mode"], "HTML");
    }

    #[tokio::test]
    async fn chart_delivery_passes_topic_and_reply_context_to_telegram() {
        let (bot, server) = telegram_fixture();
        let destination = ReplyContext {
            chat_id: ChatId(1),
            thread_id: Some(ThreadId(MessageId(7))),
            reply_to: Some(MessageId(12)),
        };
        send_photo(&bot, destination, InputFile::memory(vec![1, 2, 3]))
            .await
            .unwrap();
        let (_, body) = server.join().unwrap();
        let body = String::from_utf8_lossy(&body);
        assert!(body.contains("name=\"message_thread_id\"\r\n\r\n7"));
        assert!(body.contains("name=\"reply_parameters\""));
        assert!(body.contains("\"message_id\":12"));
        assert!(body.contains("\"allow_sending_without_reply\":true"));
    }

    #[test]
    fn retry_policy_respects_rate_limits_and_does_not_retry_permanent_errors() {
        assert_eq!(
            telegram_retry_delay(
                &RequestError::RetryAfter(teloxide::types::Seconds::from_seconds(40)),
                1
            )
            .unwrap()
            .as_secs(),
            40
        );
        assert_eq!(
            telegram_retry_delay(
                &RequestError::Api(ApiError::Unknown("Internal Server Error".into())),
                50
            )
            .unwrap()
            .as_secs(),
            300
        );
        assert_eq!(
            telegram_retry_delay(&RequestError::Api(ApiError::BotBlocked), 1),
            None
        );
        assert_eq!(
            telegram_retry_delay(&RequestError::Api(ApiError::MessageNotModified), 1),
            None
        );
    }

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
