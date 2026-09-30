use chrono::NaiveTime;
use lazy_static::lazy_static;
use regex::Regex;
use serde::{Deserialize, Serialize};
use unicode_segmentation::UnicodeSegmentation;

use crate::types::Username;

const VERSION: &str = env!("CARGO_PKG_VERSION");
lazy_static! {
    pub static ref HELP_TEXT: String = format!(
        "<b>add-bot v{VERSION}</b>

<b>Queues</b>
/1930 — Join or leave the 19:30 queue.
/add — Join or leave the instant queue.
/ls — List queues.
/predict — Predict queue win rates.
/rm — Leave all queues.

Use any HHMM time for a timed queue. Predictions use the latest 30 matches;
<code>/predict 50</code> changes the match count (1–100).

<b>Stats &amp; form</b>
/lastplayed — Last game stats.
/stats — Leetify stats and recent form.
/form — Recent match form.
/activity — Daily games over the last 90 days.
/results — Win-rate trend and results over the last 90 days.

Player stats default to yourself; add <code>@username</code> to view another player.
Activity and results default to all configured players.

<b>Leaderboards</b>
/halloffame — Top 10 by skill.
/hallofshame — Ranked by last played date.
/aim — Aim rating.
/positioning — Positioning rating.
/utility — Utility rating.
/opening — Opening duels.
/clutch — Clutch rating.
/teamflash — Team flashes per round (also /flashes).

<b>Weather &amp; electricity</b>
/temperature — Current temperature.
/weather — Weather forecast.
/el — Electricity prices and queue cost forecasts.

<b>Form themes</b>
Choose a theme and 5 or 10 results per row:
<code>/form halloween</code>
<code>/stats burger 5</code>
<code>/form @username cs2 10</code>
Or supply three emoji in win/loss/tie order: <code>/form 🍟🥬➖</code>.

squares (initial default), letters, trophy, drama, mood, moon, xmas,
halloween, burger, panda, noodle, pirate, space, cat, dog, weather,
garden, arcade, slop, hotdog, stocks, team, random, counterstrike,
mistakes, bike, car, traffic.

<code>cs2</code> and <code>kynäri</code> are aliases for counterstrike.
Random draws fresh emoji from good/win, bad/loss, and neutral/tie pools each time.
Choosing a theme for your own stats or form saves your preference.
Without a theme, requests use the target player's saved preference."
    );
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecentFormStyle {
    #[default]
    Squares,
    Letters,
    Trophy,
    Drama,
    Mood,
    Moon,
    Xmas,
    Halloween,
    Burger,
    Panda,
    Noodle,
    Pirate,
    Space,
    Cat,
    Dog,
    Weather,
    Garden,
    Arcade,
    Slop,
    Hotdog,
    Stocks,
    Team,
    Random,
    Counterstrike,
    Mistakes,
    Bike,
    Car,
    Traffic,
    Custom([String; 3]),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentFormOptions {
    pub style: RecentFormStyle,
    pub explicit_style: bool,
    pub results_per_row: usize,
}

impl Default for RecentFormOptions {
    fn default() -> Self {
        Self {
            style: RecentFormStyle::default(),
            explicit_style: false,
            results_per_row: 10,
        }
    }
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
        form_options: RecentFormOptions,
    },

    /// Recent match results for a user
    RecentForm {
        for_user: Option<Username>,
        form_options: RecentFormOptions,
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

    /// Daily games played chart for last 90 days (optionally filter by @username)
    Activity {
        for_user: Option<Username>,
    },

    /// Win-rate trend and match results for last 90 days (optionally filter by @username)
    Results {
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

fn parse_recent_form_style(s: &str) -> Option<RecentFormStyle> {
    match s.to_ascii_lowercase().as_str() {
        "squares" | "square" | "color" | "colour" => Some(RecentFormStyle::Squares),
        "letters" | "letter" | "classic" | "wlt" => Some(RecentFormStyle::Letters),
        "trophy" | "trophies" | "medals" => Some(RecentFormStyle::Trophy),
        "drama" | "dramatic" => Some(RecentFormStyle::Drama),
        "mood" | "vibes" => Some(RecentFormStyle::Mood),
        "moon" | "sunmoon" => Some(RecentFormStyle::Moon),
        "xmas" | "christmas" | "jul" => Some(RecentFormStyle::Xmas),
        "halloween" | "spooky" => Some(RecentFormStyle::Halloween),
        "burger" | "burgers" => Some(RecentFormStyle::Burger),
        "panda" | "pandas" => Some(RecentFormStyle::Panda),
        "noodle" | "noodles" | "ramen" => Some(RecentFormStyle::Noodle),
        "pirate" | "pirates" => Some(RecentFormStyle::Pirate),
        "space" | "cosmos" => Some(RecentFormStyle::Space),
        "cat" | "cats" => Some(RecentFormStyle::Cat),
        "dog" | "dogs" => Some(RecentFormStyle::Dog),
        "weather" => Some(RecentFormStyle::Weather),
        "garden" | "gardening" => Some(RecentFormStyle::Garden),
        "arcade" | "gaming" => Some(RecentFormStyle::Arcade),
        "slop" => Some(RecentFormStyle::Slop),
        "hotdog" | "hotdogs" => Some(RecentFormStyle::Hotdog),
        "stocks" | "stock" | "stonks" => Some(RecentFormStyle::Stocks),
        "team" => Some(RecentFormStyle::Team),
        "random" => Some(RecentFormStyle::Random),
        "counterstrike" | "cs2" | "kynäri" => Some(RecentFormStyle::Counterstrike),
        "mistakes" | "mistake" => Some(RecentFormStyle::Mistakes),
        "bike" | "bicycle" | "cycling" => Some(RecentFormStyle::Bike),
        "car" | "cars" | "driving" => Some(RecentFormStyle::Car),
        "traffic" => Some(RecentFormStyle::Traffic),
        _ => None,
    }
}

fn parse_recent_form_args(
    args: Option<String>,
) -> Result<(Option<Username>, RecentFormOptions), Box<dyn std::error::Error + Send + Sync>> {
    let mut for_user = None;
    let mut style = None;
    let mut custom_icons = Vec::new();
    let mut results_per_row = None;

    for arg in args.as_deref().unwrap_or_default().split_whitespace() {
        if let Some(username) = parse_username_arg(arg.to_string()) {
            if for_user.replace(username).is_some() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "only one @username can be provided",
                )
                .into());
            }
        } else if let Some(parsed_style) = parse_recent_form_style(arg) {
            if style.replace(parsed_style).is_some() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "only one recent-form style can be provided",
                )
                .into());
            }
        } else if arg == "5" || arg == "10" {
            if results_per_row.replace(arg.parse::<usize>()?).is_some() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "only one row width can be provided",
                )
                .into());
            }
        } else if arg.graphemes(true).all(|icon| emojis::get(icon).is_some()) {
            custom_icons.extend(arg.graphemes(true).map(str::to_owned));
        } else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "usage: /stats or /form [@username] [theme or three emoji in win/loss/tie order] [5|10]",
            )
            .into());
        }
    }

    if !custom_icons.is_empty() {
        if style.is_some() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "only one recent-form style can be provided",
            )
            .into());
        }
        let icons: [String; 3] = custom_icons.try_into().map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "custom themes need exactly three emoji in win/loss/tie order",
            )
        })?;
        style = Some(RecentFormStyle::Custom(icons));
    }

    let explicit_style = style.is_some();
    Ok((
        for_user,
        RecentFormOptions {
            style: style.unwrap_or_default(),
            explicit_style,
            results_per_row: results_per_row.unwrap_or(10),
        },
    ))
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
                let (for_user, form_options) = parse_recent_form_args(args)?;
                Some(Command::Stats {
                    for_user,
                    form_options,
                })
            }
            "form" | "recentform" => {
                let (for_user, form_options) = parse_recent_form_args(args)?;
                Some(Command::RecentForm {
                    for_user,
                    form_options,
                })
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

            "results" => {
                let for_user = args.and_then(parse_username_arg);
                Some(Command::Results { for_user })
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
    fn custom_form_themes_accept_three_complete_emoji() {
        for command in ["stats", "form"] {
            for (args, icons) in [
                ("🍟🥬➖", ["🍟", "🥬", "➖"]),
                ("🍟 🥬 ➖", ["🍟", "🥬", "➖"]),
                ("👍🏽👨‍👩‍👧‍👦🇫🇮", ["👍🏽", "👨‍👩‍👧‍👦", "🇫🇮"]),
                ("☀️🏳️‍🌈1️⃣", ["☀️", "🏳️‍🌈", "1️⃣"]),
                ("🍟🍟🍟", ["🍟", "🍟", "🍟"]),
            ] {
                let parsed = parse_cmd(&format!("/{command} @player {args} 5"))
                    .unwrap()
                    .unwrap();
                let (for_user, options) = match parsed {
                    Command::Stats {
                        for_user,
                        form_options,
                    }
                    | Command::RecentForm {
                        for_user,
                        form_options,
                    } => (for_user, form_options),
                    _ => panic!("unexpected command"),
                };
                assert_eq!(for_user, Some(Username::new("player".to_string())));
                assert_eq!(
                    options.style,
                    RecentFormStyle::Custom(icons.map(str::to_owned))
                );
                assert!(options.explicit_style);
                assert_eq!(options.results_per_row, 5);
            }
        }
    }

    #[test]
    fn custom_form_themes_reject_bad_counts_text_and_multiple_styles() {
        for command in ["stats", "form"] {
            for args in [
                "🍟",
                "🍟🥬",
                "🍟🥬➖🍔",
                "🍟🥬➖ 🍔🥬➖",
                "abc",
                "🍟a➖",
                "<b>🍟🥬➖</b>",
                "🍟🥬&",
                "burger 🍟🥬➖",
                "🍟🥬➖ burger",
                "🍟🥬➖ 5 10",
            ] {
                assert!(parse_cmd(&format!("/{command} {args}")).is_err(), "{args}");
            }
        }
    }

    #[test]
    fn recent_form_parser_distinguishes_explicit_icons_from_row_width() {
        for command in ["stats", "form"] {
            for (args, explicit, expected_style) in [
                ("", false, RecentFormStyle::Squares),
                ("5", false, RecentFormStyle::Squares),
                ("squares", true, RecentFormStyle::Squares),
                ("@player trophy 5", true, RecentFormStyle::Trophy),
                ("moon 5", true, RecentFormStyle::Moon),
                ("xmas", true, RecentFormStyle::Xmas),
                ("christmas", true, RecentFormStyle::Xmas),
                ("jul", true, RecentFormStyle::Xmas),
                ("halloween", true, RecentFormStyle::Halloween),
                ("burger", true, RecentFormStyle::Burger),
                ("panda", true, RecentFormStyle::Panda),
                ("noodle", true, RecentFormStyle::Noodle),
                ("pirate", true, RecentFormStyle::Pirate),
                ("space", true, RecentFormStyle::Space),
                ("cat", true, RecentFormStyle::Cat),
                ("dog", true, RecentFormStyle::Dog),
                ("weather", true, RecentFormStyle::Weather),
                ("garden", true, RecentFormStyle::Garden),
                ("arcade", true, RecentFormStyle::Arcade),
                ("slop", true, RecentFormStyle::Slop),
                ("hotdog", true, RecentFormStyle::Hotdog),
                ("stocks", true, RecentFormStyle::Stocks),
                ("@player HALLOWEEN 5", true, RecentFormStyle::Halloween),
                ("ramen", true, RecentFormStyle::Noodle),
                ("stonks", true, RecentFormStyle::Stocks),
                ("team", true, RecentFormStyle::Team),
                ("random", true, RecentFormStyle::Random),
                ("counterstrike", true, RecentFormStyle::Counterstrike),
                ("cs2", true, RecentFormStyle::Counterstrike),
                ("kynäri", true, RecentFormStyle::Counterstrike),
                ("mistakes", true, RecentFormStyle::Mistakes),
                ("bike", true, RecentFormStyle::Bike),
                ("car", true, RecentFormStyle::Car),
                ("traffic", true, RecentFormStyle::Traffic),
            ] {
                let parsed = parse_cmd(&format!("/{command} {args}")).unwrap().unwrap();
                let options = match parsed {
                    Command::Stats { form_options, .. }
                    | Command::RecentForm { form_options, .. } => form_options,
                    _ => panic!("unexpected command"),
                };
                assert_eq!(options.explicit_style, explicit);
                assert_eq!(options.style, expected_style);
                assert_eq!(
                    options.results_per_row,
                    if args.contains('5') { 5 } else { 10 }
                );
            }
        }
    }

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
