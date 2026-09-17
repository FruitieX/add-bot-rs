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
- /predict [1-100]
                Predicted win rates for current queues, using the latest 30 matches by default.
- /rm           Remove yourself from all queues.
- /lastplayed   Last played game stats for player.
- /stats        Leetify stats for player.
- /halloffame   Top 10 players by skill level.
- /hallofshame  Top 10 players by last played date.
- /aim          Leaderboard by aim rating.
- /positioning  Leaderboard by positioning.
- /utility      Leaderboard by utility usage.
- /opening      Leaderboard by opening duels.
- /clutch       Leaderboard by clutch rating.
- /teamflash    Flashbangs thrown and teammates flashed per round.
- /flashes      Flashbangs thrown and teammates flashed per round.
- /activity     Daily games played by all players (last 365 days).
- /temperature  Current temperature for configured location.
- /weather      Weather for configured location.
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

    /// Predicted win rates for queues in the current chat.
    Predictions {
        match_count: usize,
    },

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

    /// Current temperature for configured location
    Temperature,

    /// Weather for configured location
    Weather,

    /// Daily games played chart for last 365 days (optionally filter by @username)
    Activity {
        for_user: Option<Username>,
    },

    // Get the latest electricity prices as a chart
    Sahko,

    /// Leaderboard for a specific stat (aim, positioning, utility, opening, clutch)
    StatLeaderboard {
        stat_type: String,
    },

    /// Team flash hall of shame
    TeamFlash,
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
    // Prevent odd characters in bot reply
    if (!cmd.is_ascii()) || cmd.len() > 32 {
        return false;
    }

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

fn parse_prediction_match_count(
    args: Option<String>,
) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
    const MAX_MATCH_COUNT: usize = 100;

    let match_count = args.as_deref().unwrap_or("30").parse::<usize>()?;
    if !(1..=MAX_MATCH_COUNT).contains(&match_count) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("prediction match count must be between 1 and {MAX_MATCH_COUNT}"),
        )
        .into());
    }

    Ok(match_count)
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
            "predict" | "prediction" | "predictions" | "winpct" => {
                let match_count = parse_prediction_match_count(args)?;
                Some(Command::Predictions { match_count })
            }
            "statistics" | "stats" => {
                let for_user = args.and_then(parse_username_arg);

                Some(Command::Stats { for_user })
            }
            "aim" => Some(Command::StatLeaderboard {
                stat_type: "aim".to_string(),
            }),
            "positioning" | "pos" => Some(Command::StatLeaderboard {
                stat_type: "positioning".to_string(),
            }),
            "utility" | "util" | "nades" => Some(Command::StatLeaderboard {
                stat_type: "utility".to_string(),
            }),
            "opening" | "openingduels" | "duels" => Some(Command::StatLeaderboard {
                stat_type: "opening".to_string(),
            }),
            "clutch" | "clutches" => Some(Command::StatLeaderboard {
                stat_type: "clutch".to_string(),
            }),
            "teamflash" | "tf" | "flash" | "flashes" | "blind" => Some(Command::TeamFlash),
            "hallofshame" | "wallofshame" | "shame" => Some(Command::HallOfShame),
            "halloffame" | "walloffame" | "fame" | "top" | "top10" | "ranks" | "premier" => {
                Some(Command::HallOfFame {
                    rank_type: "premier".to_string(),
                })
            }
            "temperature" => Some(Command::Temperature),
            "weather" => Some(Command::Weather),
            "sahko" | "el" | "elpriser" => Some(Command::Sahko),

            "activity" | "games" | "played" | "daily" => {
                let for_user = args.and_then(parse_username_arg);
                Some(Command::Activity { for_user })
            }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prediction_match_count_defaults_to_thirty() {
        assert!(matches!(
            parse_cmd("/predict").unwrap(),
            Some(Command::Predictions { match_count: 30 })
        ));
    }

    #[test]
    fn prediction_match_count_accepts_one_to_one_hundred() {
        assert!(matches!(
            parse_cmd("/predict 1").unwrap(),
            Some(Command::Predictions { match_count: 1 })
        ));
        assert!(matches!(
            parse_cmd("/predict 100").unwrap(),
            Some(Command::Predictions { match_count: 100 })
        ));
    }

    #[test]
    fn prediction_match_count_rejects_values_outside_range() {
        assert!(parse_cmd("/predict 0").is_err());
        assert!(parse_cmd("/predict 101").is_err());
        assert!(parse_cmd("/predict many").is_err());
    }
}
