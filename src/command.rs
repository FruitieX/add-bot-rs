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
    Remove,

    /// Lists chat queues.
    List,
}

impl Command {
    pub fn descriptions() -> &'static str {
        HELP_TEXT
    }
}

struct CmdMatches {
    cmd: String,
    bot_name: Option<String>,
    args: Option<String>,
}

/// Parses a Telegram message into command name, bot name and arguments.
fn get_cmd_matches(text: &str) -> Option<CmdMatches> {
    lazy_static! {
        // Construct a regex that matches TG commands
        static ref RE: Regex = Regex::new(r"^/([^@\s]+)@?(?:(\S+)|)\s?([\s\S]*)$").unwrap();
    }

    let caps = RE.captures(text)?;

    let cmd = caps.get(1)?.as_str().to_string();
    let bot_name = caps.get(2).map(|x| x.as_str().to_string());
    let args = caps.get(3).map(|x| x.as_str().to_string());

    Some(CmdMatches {
        cmd,
        bot_name,
        args,
    })
}

/// Checks whether string contains 3-4 digits.
fn matches_timed_queue(cmd: &str) -> bool {
    lazy_static! {
        // Construct a regex that matches timed queue commands.
        // E.g. `/1930` for 19:30 or `/645` for 6:45.
        static ref RE: Regex = Regex::new(r"^\d{3,4}$").unwrap();
    }

    RE.is_match(cmd)
}

pub fn parse_cmd(text: &str) -> Result<Option<Command>, Box<dyn std::error::Error + Send + Sync>> {
    let text = text.trim();

    let cmd_result = if let Some(cmd_matches) = get_cmd_matches(text) {
        // Message matched Telegram bot command regex, check if it's a command
        // we want to handle.
        let CmdMatches {
            cmd,
            bot_name: _bot_name,
            args: _args,
        } = cmd_matches;

        match cmd.as_str() {
            "help" | "info" => Some(Command::Help),
            "add" | "heti" | "kynär" | "kynäri" => Some(Command::Add(None)),
            "rm" => Some(Command::Remove),
            "ls" | "list" | "count" => Some(Command::List),
            _ => {
                // Didn't match any of our normal commands, check for timed
                // queue command match.
                if matches_timed_queue(&cmd) {
                    // Left pad with zeroes.
                    let timed_queue = format!("{:0>4}", cmd);

                    // Attempt parsing string as %H%M time.
                    let parsed_time = NaiveTime::parse_from_str(&timed_queue, "%H%M")?;

                    Some(Command::Add(Some(parsed_time)))
                } else {
                    None
                }
            }
        }
    } else {
        // No match, ignore message.
        // (We could handle messages that are not Telegram bot commands here)
        None
    };

    Ok(cmd_result)
}
