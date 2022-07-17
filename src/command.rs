use anyhow::Result;
use chrono::NaiveTime;
use lazy_static::lazy_static;
use regex::Regex;

use crate::types::Username;

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
    AddRemove {
        time: Option<NaiveTime>,
        for_user: Option<Username>,
    },

    /// Removes player from all queues.
    RemoveAll,

    /// Lists chat queues.
    List,

    /// Tänään jäljellä
    Tj,

    /// Shows an image of the Pokemon that matches the current TJ
    Pokemon,
}

impl Command {
    pub fn descriptions() -> &'static str {
        HELP_TEXT
    }
}

struct CmdMatches {
    cmd: String,
    #[allow(dead_code)]
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
    let args = caps.get(3).and_then(|x| {
        let s = x.as_str().trim().to_string();

        // Convert empty strings to None values
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    });

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

fn matches_tj_regex(cmd: &str) -> bool {
    lazy_static! {
        static ref RE: Regex = Regex::new(r"^hajo+$").unwrap();
    }

    RE.is_match(cmd)
}

fn parse_time_arg(s: &str) -> Result<NaiveTime, chrono::ParseError> {
    // Left pad with zeroes.
    let timed_queue = format!("{:0>4}", s);

    // Attempt parsing string as %H%M time.
    NaiveTime::parse_from_str(&timed_queue, "%H%M")
}

fn parse_username_arg(s: String) -> Option<Username> {
    lazy_static! {
        // Construct a regex that matches `@username`.
        static ref RE: Regex = Regex::new(r"^@(\w{5,32})$").unwrap();
    }

    let caps = RE.captures(&s)?;
    let username = caps.get(1)?.as_str();

    Some(Username::new(username.to_string()))
}

pub fn parse_cmd(text: &str) -> Result<Option<Command>, Box<dyn std::error::Error + Send + Sync>> {
    let text = text.trim();

    let cmd_result = if let Some(cmd_matches) = get_cmd_matches(text) {
        // Message matched Telegram bot command regex, check if it's a command
        // we want to handle.
        let CmdMatches { cmd, args, .. } = cmd_matches;

        match cmd.as_str() {
            "help" | "info" => Some(Command::Help),
            "rm" => Some(Command::RemoveAll),
            "ls" | "list" | "count" => Some(Command::List),
            "tj" | "mornings" | "aamuja" | "dägä" | "dagar" | "daegae" | "morgnar" => {
                Some(Command::Tj)
            }
            "add" | "heti" | "kynär" | "kynäri" => {
                let for_user = args.and_then(parse_username_arg);

                Some(Command::AddRemove {
                    time: None,
                    for_user,
                })
            }
            "pokemon" => Some(Command::Pokemon),
            _ => {
                if matches_tj_regex(&cmd) {
                    Some(Command::Tj)
                }
                // Didn't match any of our normal commands, check for timed
                // queue command match.
                else if matches_timed_queue(&cmd) {
                    let parsed_time = parse_time_arg(&cmd)?;
                    let for_user = args.and_then(parse_username_arg);

                    Some(Command::AddRemove {
                        time: Some(parsed_time),
                        for_user,
                    })
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
