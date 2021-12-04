use anyhow::Result;
use chrono::NaiveTime;
use lazy_static::lazy_static;
use regex::Regex;

pub static HELP_TEXT: &str = "These commands are supported:

```
- /{hhmm}
  Add/remove yourself from the timed queue at hh:mm.
  For example: /1830

- /add
  Add/remove yourself from the instant queue.

- /rm
  Remove yourself from all queues.
```";

pub enum Command {
    /// Display help text for supported commands.
    Help,

    /// Add/remove player from instant queue or timed queue.
    Add(Option<NaiveTime>),

    /// Removes player from all queues.
    Rm,
}

impl Command {
    pub fn descriptions() -> &'static str {
        HELP_TEXT
    }
}

pub fn parse_cmd(text: &str) -> Result<Option<Command>, Box<dyn std::error::Error + Send + Sync>> {
    let text = text.trim();

    let cmd = match text {
        "/help" => Some(Command::Help),
        "/add" => Some(Command::Add(None)),
        "/rm" => Some(Command::Rm),
        _ => {
            lazy_static! {
                // Construct a regex that matches commands with four digits
                // (such as /xxxx).
                static ref RE: Regex = Regex::new(r"^/\d{4}$").unwrap();
            }

            if RE.is_match(text) {
                let parsed_time = NaiveTime::parse_from_str(&text[1..], "%H%M")?;
                Some(Command::Add(Some(parsed_time)))
            } else {
                None
            }
        }
    };

    Ok(cmd)
}
