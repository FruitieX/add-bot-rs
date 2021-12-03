use anyhow::Result;
use chrono::NaiveTime;
use teloxide::utils::command::{BotCommand, ParseError};

/// Parser for queue command arguments. The following formats are supported:
///
/// - "/add" Add command with instant (nameless) queue
/// - "/add 17:07" Add command with timed queue
fn parse_queue_cmd(input: String) -> Result<(Option<NaiveTime>,), ParseError> {
    if input.is_empty() {
        Ok((None,))
    } else {
        let parsed_time = NaiveTime::parse_from_str(&input, "%H:%M")
            .map_err(|e| ParseError::IncorrectFormat(e.into()))?;

        Ok((Some(parsed_time),))
    }
}

#[derive(BotCommand)]
#[command(rename = "lowercase", description = "These commands are supported:")]
pub enum Command {
    #[command(description = "Display this help text.")]
    Help,
    #[command(
        description = "Add yourself to a queue. Use `/add` for instant queue, and `/add HH:MM` for timed queue.",
        parse_with = "parse_queue_cmd"
    )]
    Add(Option<NaiveTime>),
}
