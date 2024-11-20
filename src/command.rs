use anyhow::Result;
use chrono::NaiveTime;
use lazy_static::lazy_static;
use regex::Regex;

use crate::types::Username;

const VERSION: &str = env!("CARGO_PKG_VERSION");
lazy_static! {
    pub static ref HELP_TEXT: String = format!(
        "add-bot v{VERSION}

The following commands are supported:
```
- /1930         Add/remove player from timed queue at 19:30.
- /add          Add/remove player from the instant queue.
- /ls           List existing queues.
- /rm           Remove yourself from all queues.
- /lastplayed   Last played game stats for player.
- /stats        Leetify stats for player.
- /halloffame   Top 10 players by skill level.
- /hallofshame  Top 10 players by last played date.
```Most commands accept an optional `@username` argument, which defaults to yourself."
    );
}

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

    /// Leetify stats for user
    Stats {
        for_user: Option<Username>,
    },

    /// Last played stats from Leetify
    LastPlayed {
        for_user: Option<Username>,
    },

    /// Top 10 players by last played date
    HallOfShame,

    /// Top 10 players by skill level
    HallOfFame {
        rank_type: String,
    },

    // Get the latest electricity prices as a chart
    Sahko,
}

impl Command {
    pub fn help() -> String {
        HELP_TEXT.as_str().to_string()
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
    // Construct a regex that matches TG commands
    lazy_static! {
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

fn matches_cs_map_name(cmd: &str) -> bool {
    cmd.starts_with("de_") || cmd.starts_with("cs_")
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
            "help" | "info" | "version" | "v" | "start" => Some(Command::Help),
            "rm" => Some(Command::RemoveAll),
            "ls" | "list" | "count" => Some(Command::List),
            "statistics" | "stats" | "leetify" => {
                let for_user = args.and_then(parse_username_arg);

                Some(Command::Stats { for_user })
            }
            "hallofshame" | "wallofshame" | "shame" => Some(Command::HallOfShame),
            "halloffame" | "walloffame" | "fame" | "top" | "top10" | "ranks" | "premier" => {
                Some(Command::HallOfFame {
                    rank_type: "premier".to_string(),
                })
            }
            "sahko" | "el" | "elpriser" => Some(Command::Sahko),

            "wingman" => Some(Command::HallOfFame {
                rank_type: "wingman".to_string(),
            }),

            "cs_office" | "office" => Some(Command::HallOfFame {
                rank_type: "cs_office".to_string(),
            }),
            "cs_italy" | "italy" => Some(Command::HallOfFame {
                rank_type: "cs_italy".to_string(),
            }),
            "de_mirage" | "mirage" => Some(Command::HallOfFame {
                rank_type: "de_mirage".to_string(),
            }),
            "de_overpass" | "overpass" => Some(Command::HallOfFame {
                rank_type: "de_overpass".to_string(),
            }),
            "de_inferno" | "inferno" => Some(Command::HallOfFame {
                rank_type: "de_inferno".to_string(),
            }),
            "de_nuke" | "nuke" => Some(Command::HallOfFame {
                rank_type: "de_nuke".to_string(),
            }),
            "de_train" | "train" => Some(Command::HallOfFame {
                rank_type: "de_train".to_string(),
            }),
            "de_vertigo" | "vertigo" => Some(Command::HallOfFame {
                rank_type: "de_vertigo".to_string(),
            }),
            "de_dust2" | "dust2" => Some(Command::HallOfFame {
                rank_type: "de_dust2".to_string(),
            }),
            "de_cache" | "cache" => Some(Command::HallOfFame {
                rank_type: "de_cache".to_string(),
            }),
            "de_ancient" | "ancient" => Some(Command::HallOfFame {
                rank_type: "de_ancient".to_string(),
            }),
            "de_anubis" | "anubis" => Some(Command::HallOfFame {
                rank_type: "de_anubis".to_string(),
            }),

            "lastplayed" => {
                let for_user = args.and_then(parse_username_arg);

                Some(Command::LastPlayed { for_user })
            }
            "add" | "instant" | "heti" | "kynär" | "kynäri" => {
                let for_user = args.and_then(parse_username_arg);

                Some(Command::AddRemove {
                    time: None,
                    for_user,
                })
            }
            _ => {
                // Didn't match any of our normal commands, check for timed
                // queue command match.
                if matches_timed_queue(&cmd) {
                    let parsed_time = parse_time_arg(&cmd)?;
                    let for_user = args.and_then(parse_username_arg);

                    Some(Command::AddRemove {
                        time: Some(parsed_time),
                        for_user,
                    })
                } else if matches_cs_map_name(&cmd) {
                    Some(Command::HallOfFame {
                        rank_type: cmd.to_string(),
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
