use anyhow::Result;
use chrono::NaiveTime;
use lazy_static::lazy_static;
use regex::Regex;

pub static HELP_TEXT: &str = "These commands are supported:

```
- /help, /info
  Displays this help text.
  
- /{hhmm}
  Add/remove yourself from the timed queue at hh:mm.
  For example: /1830

- /add, /heti, /kynäri
  Add/remove yourself from the instant queue.

- /rm
  Remove yourself from all queues.

- /ls, /count
  Lists queues.
```";

pub enum Command {
    /// Display help text for supported commands.
    Help,

    /// Add/remove player from instant queue or timed queue.
    Add(Option<NaiveTime>),

    /// Removes player from all queues.
    Rm,

    /// Lists chat queues.
    Ls,
}

impl Command {
    pub fn descriptions() -> &'static str {
        HELP_TEXT
    }
}

pub fn parse_cmd(text: &str) -> Result<Option<Command>, Box<dyn std::error::Error + Send + Sync>> {
    let text = text.trim();

    let cmd = match text {
        "/help" | "/info" => Some(Command::Help),
        "/add" | "/heti" | "/kynäri" => Some(Command::Add(None)),
        "/rm" => Some(Command::Rm),
        "/ls" | "/count" => Some(Command::Ls),
        _ => {
            lazy_static! {
                // Construct a regex that matches commands with four digits
                // (such as /xxxx).
                static ref RE: Regex = Regex::new(r"^/(\d{3,4})(@(.+))?$").unwrap();
            }

            let caps = RE.captures(text);

            if let Some(caps) = caps {
                let cmd = caps.get(1);
                // let bot_name = caps.get(2);

                if let Some(cmd) = cmd {
                    let cmd = cmd.as_str();
                    let cmd = format!("{:0>4}", cmd);
                    let parsed_time = NaiveTime::parse_from_str(&cmd, "%H%M")?;
                    Some(Command::Add(Some(parsed_time)))
                } else {
                    None
                }
            } else {
                None
            }
        }
    };

    Ok(cmd)
}
