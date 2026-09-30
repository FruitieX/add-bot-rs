use chrono::Utc;
use chrono_tz::Tz;

use crate::{
    command::{RecentFormOptions, RecentFormStyle},
    services,
    settings::Settings,
    types::Username,
    util::escape_html,
};

fn index_to_pos(index: usize) -> String {
    match index {
        0 => "🥇".to_string(),
        1 => "🥈".to_string(),
        2 => "🥉".to_string(),
        index => format!("#{pos}", pos = index + 1),
    }
}

fn skill_level_to_cs2_rank(skill_level: u32) -> String {
    let unranked_text = "Unranked";
    let ranks = [
        unranked_text,
        "Silver I",
        "Silver II",
        "Silver III",
        "Silver IV",
        "Silver Elite",
        "Silver Elite Master",
        "Gold Nova I",
        "Gold Nova II",
        "Gold Nova III",
        "Gold Nova Master",
        "Master Guardian I",
        "Master Guardian II",
        "Master Guardian Elite",
        "Distinguished Master Guardian",
        "Legendary Eagle",
        "Legendary Eagle Master",
        "Supreme Master First Class",
        "Global Elite",
    ];

    if skill_level < 1000 {
        let rank = ranks
            .get(skill_level as usize)
            .unwrap_or(&unranked_text)
            .to_string();

        format!("{skill_level}, {rank}")
    } else {
        skill_level.to_string()
    }
}

pub async fn hall_of_fame(settings: &Settings, rank_type: String) -> String {
    let res = services::leetify::hall_of_fame(settings, &rank_type).await;

    match res {
        Ok(hall_of_fame) => {
            let avg = hall_of_fame.avg_skill_level;
            let median = hall_of_fame.median_skill_level;
            let list = hall_of_fame
                .entries
                .iter()
                .take(10)
                .enumerate()
                .map(|(index, entry)| {
                    let username = &entry.username;
                    let pos = index_to_pos(index);
                    let skill_level = skill_level_to_cs2_rank(entry.skill_level);

                    format!("{pos}: {username} (rating: {skill_level})")
                })
                .collect::<Vec<String>>()
                .join("\n");

            if hall_of_fame.entries.is_empty() {
                return format!("No entries found for {rank_type}. ☹️",);
            }

            format!(
                "Hall of fame, or top 10 {rank_type} ranks:\n\n{list}\n\nAvg: {avg:.0}, Median: {median}"
            )
        }
        Err(e) => {
            eprintln!("Failed to fetch stats from Leetify: {}", e);
            "Failed to fetch stats from Leetify".to_string()
        }
    }
}

pub async fn hall_of_shame(settings: &Settings, tz: &Tz) -> String {
    let res = services::leetify::hall_of_shame(settings).await;

    match res {
        Ok(entries) => {
            if entries.is_empty() {
                return "No Leetify match data found. ☹️".to_string();
            }

            let list = entries
                .iter()
                .enumerate()
                .map(|(index, entry)| {
                    let t = entry.last_played.with_timezone(&tz.clone());
                    let t = t.format("%Y-%m-%d");
                    let days_ago = (Utc::now().with_timezone(tz).date_naive()
                        - entry.last_played.with_timezone(tz).date_naive())
                    .num_days();
                    let days = if days_ago == 1 { "day" } else { "days" };
                    let username = &entry.username;
                    let pos = index_to_pos(index);
                    let spree = if entry.spree > 1 {
                        format!(" ({spree} day spree)", spree = entry.spree)
                    } else {
                        "".to_string()
                    };

                    format!("{pos} {t} ({days_ago} {days} ago): {username}{spree}")
                })
                .collect::<Vec<String>>()
                .join("\n");

            let days_since_last_played: Vec<i64> = entries
                .iter()
                .map(|entry| {
                    (Utc::now().date_naive() - entry.last_played.with_timezone(tz).date_naive())
                        .num_days()
                })
                .collect();

            let avg =
                days_since_last_played.iter().sum::<i64>() / days_since_last_played.len() as i64;

            format!(
                "Hall of shame, or longest time since last played with team:\n\n{list}\n\nAvg: {avg:.0} days",
            )
        }
        Err(e) => {
            eprintln!("Failed to fetch stats from Leetify: {}", e);
            "Failed to fetch stats from Leetify".to_string()
        }
    }
}

pub async fn last_played(settings: &Settings, tz: &Tz, username: Username) -> String {
    let res = services::leetify::last_played(settings, &username).await;

    match res {
        Ok(game) => {
            let t = game.game_finished_at;
            let t = t.with_timezone(&tz.clone()).format("%Y-%m-%d %H:%M:%S");
            let days_ago = (Utc::now().with_timezone(tz).date_naive()
                - game.game_finished_at.with_timezone(tz).date_naive())
            .num_days();
            let days = if days_ago == 1 { "day" } else { "days" };
            let map = game.map_name;
            let match_result = format!("{}-{} {}", game.scores.0, game.scores.1, game.match_result);

            let text = format!(
                        "{username} last played with team (according to Leetify):\n- Date: {t} ({days_ago} {days} ago)\n- Map: {map}\n- Result: {match_result}"
                    );
            text
        }
        Err(e) => {
            eprintln!("Failed to fetch last played stats from Leetify: {}", e);
            "Failed to fetch last played stats from Leetify".to_string()
        }
    }
}

const RANDOM_WIN_ICONS: &[&str] = &[
    "😀", "😃", "😄", "😁", "😆", "😊", "😎", "🥳", "🤩", "😍", "🥰", "😇", "😸", "😺", "😻", "😹",
    "🙌", "👏", "👍", "🤘", "💪", "🏆", "🥇", "🏅", "🎖️", "👑", "💎", "💰", "💵", "💸", "🎉", "🎊",
    "🎆", "🎇", "✨", "🌟", "⭐", "💫", "🔥", "🚀", "📈", "✅", "✔️", "💯", "🟢", "🟩", "💚", "🍀",
    "🌈", "☀️", "🌞", "🌻", "🌸", "🌺", "🌼", "🎁", "🎂", "🍰", "🍔", "🍟",
];

const RANDOM_LOSS_ICONS: &[&str] = &[
    "😢", "😭", "😞", "😔", "😟", "😕", "🙁", "☹️", "😣", "😖", "😫", "😩", "😤", "😠", "😡", "🤬",
    "😱", "😨", "😰", "😓", "😥", "😪", "🤒", "🤕", "🤢", "🤮", "😵", "🥴", "😿", "🙀", "👎", "💔",
    "💀", "☠️", "🪦", "⚰️", "🧟", "👻", "💩", "🗑️", "💥", "🧨", "🔻", "📉", "❌", "🚫", "⛔", "🛑",
    "🟥", "🔴", "🥀", "🌧️", "⛈️", "🌩️", "🌪️", "🕳️", "🪤", "🚑", "🩼", "🤦",
];

const RANDOM_TIE_ICONS: &[&str] = &[
    "😐", "😑", "😶", "🫤", "🤔", "🤨", "🧐", "😴", "🥱", "🤷", "🤝", "🫱", "🫲", "👋", "👀", "⚖️",
    "➖", "🟰", "↔️", "🔄", "⏸️", "⏯️", "🟨", "🟡", "⚪", "⚫", "⬜", "⬛", "🔘", "◻️", "◼️", "🌗",
    "🌓", "🌙", "☁️", "🌫️", "🌥️", "🪨", "🧱", "⚓", "🧭", "⏳", "⌛", "🕰️", "⏰", "🎲", "🧩", "🪙",
    "👔", "🥢", "🍽️", "🥖", "🫖", "☕", "🥛", "🚦", "🛸", "🤖", "📦", "💤",
];

fn random_form_icons() -> (&'static str, &'static str, &'static str) {
    use rand::seq::IndexedRandom;

    let mut rng = rand::rng();
    (
        *RANDOM_WIN_ICONS
            .choose(&mut rng)
            .expect("win pool is nonempty"),
        *RANDOM_LOSS_ICONS
            .choose(&mut rng)
            .expect("loss pool is nonempty"),
        *RANDOM_TIE_ICONS
            .choose(&mut rng)
            .expect("tie pool is nonempty"),
    )
}

fn format_recent_results_with_options(
    recent_matches: &[services::leetify::RecentMatch],
    form_options: RecentFormOptions,
) -> String {
    // The profile endpoint returns its recent_matches collection newest first.
    // Keep the same order for the form display and calculate all counts from
    // only the configured recent-match window.
    let recent_matches = recent_matches
        .iter()
        .take(services::leetify::RECENT_MATCHES_LIMIT)
        .collect::<Vec<_>>();

    if recent_matches.is_empty() {
        return "No recent results available.".to_string();
    }

    let wins = recent_matches
        .iter()
        .filter(|m| matches!(&m.result, services::leetify::MatchResult::Win))
        .count();
    let losses = recent_matches
        .iter()
        .filter(|m| matches!(&m.result, services::leetify::MatchResult::Loss))
        .count();
    let ties = recent_matches
        .iter()
        .filter(|m| matches!(&m.result, services::leetify::MatchResult::Tie))
        .count();
    let win_percentage = if wins + losses == 0 {
        "—".to_string()
    } else {
        format!("{:.0}%", wins as f32 / (wins + losses) as f32 * 100.0)
    };
    let (win_icon, loss_icon, tie_icon) = match &form_options.style {
        RecentFormStyle::Squares => ("🟩", "🟥", "🟨"),
        RecentFormStyle::Letters => ("W", "L", "T"),
        RecentFormStyle::Trophy => ("🏆", "💀", "👔"),
        RecentFormStyle::Drama => ("🎉", "🪦", "🤝"),
        RecentFormStyle::Mood => ("😎", "😭", "😐"),
        RecentFormStyle::Moon => ("🌞", "🌚", "🌗"),
        RecentFormStyle::Xmas => ("🎁", "🪨", "🎄"),
        RecentFormStyle::Halloween => ("🎃", "👻", "🍬"),
        RecentFormStyle::Burger => ("🍔", "🥬", "🍟"),
        RecentFormStyle::Panda => ("🐼", "🐻", "🎋"),
        RecentFormStyle::Noodle => ("🍜", "🫗", "🥢"),
        RecentFormStyle::Pirate => ("💰", "☠️", "⚓"),
        RecentFormStyle::Space => ("🚀", "☄️", "🛸"),
        RecentFormStyle::Cat => ("😸", "😿", "😼"),
        RecentFormStyle::Dog => ("🦴", "💩", "🐕"),
        RecentFormStyle::Weather => ("☀️", "⛈️", "☁️"),
        RecentFormStyle::Garden => ("🌻", "🥀", "🌱"),
        RecentFormStyle::Arcade => ("👾", "💥", "🕹️"),
        RecentFormStyle::Slop => ("🤖", "🗑️", "🫠"),
        RecentFormStyle::Hotdog => ("🌭", "💩", "🥖"),
        RecentFormStyle::Stocks => ("📈", "📉", "➖"),
        RecentFormStyle::Team => ("🏆", "🚑", "🤝"),
        RecentFormStyle::Random => random_form_icons(),
        RecentFormStyle::Counterstrike => ("💣", "🐔", "🛡️"),
        RecentFormStyle::Mistakes => ("🎯", "🤦", "🤷"),
        RecentFormStyle::Bike => ("🚴", "💥", "🚲"),
        RecentFormStyle::Car => ("🏎️", "🚧", "🚗"),
        RecentFormStyle::Traffic => ("🟢", "🔴", "🟡"),
        RecentFormStyle::Custom(icons) => (icons[0].as_str(), icons[1].as_str(), icons[2].as_str()),
    };
    let result_marker = |result: &services::leetify::MatchResult| match result {
        services::leetify::MatchResult::Win => win_icon,
        services::leetify::MatchResult::Loss => loss_icon,
        services::leetify::MatchResult::Tie => tie_icon,
    };
    let legend = format!("{win_icon} win · {loss_icon} loss · {tie_icon} tie");
    let results = recent_matches
        .iter()
        .map(|m| result_marker(&m.result))
        .collect::<Vec<_>>()
        .chunks(form_options.results_per_row)
        .map(|row| row.join(""))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "<pre>{results}</pre>\n<b>{wins}W · {losses}L · {ties}T · {win_percentage} win rate</b>\n<i>{legend}</i>"
    )
}

fn format_teammate_result(entry: &services::leetify::TeammateStatsEntry) -> String {
    let matches = entry.games();
    let record = if entry.ties > 0 {
        format!("{}W/{}L/{}T", entry.wins, entry.losses, entry.ties)
    } else {
        format!("{}W/{}L", entry.wins, entry.losses)
    };
    let rate = if matches <= 1 {
        String::new()
    } else if entry.wins + entry.losses == 0 {
        " · —".to_string()
    } else {
        format!(
            " · {:.0}%",
            entry.wins as f32 / (entry.wins + entry.losses) as f32 * 100.0
        )
    };
    let noun = if matches == 1 { "game" } else { "games" };
    format!(
        "{} · {record}{rate} · {matches} {noun}",
        escape_html(&entry.username.to_string())
    )
}

fn format_teammate_results(entries: &[services::leetify::TeammateStatsEntry]) -> String {
    if entries.is_empty() {
        return "No teammate records available.".to_string();
    }

    entries
        .iter()
        .map(format_teammate_result)
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_teammate_stats(stats: &services::leetify::TeammateStats) -> String {
    format!(
        "<b>Teammates</b> · your last {} {}\n{}",
        stats.matches_considered,
        if stats.matches_considered == 1 {
            "match"
        } else {
            "matches"
        },
        format_teammate_results(&stats.entries)
    )
}

fn format_rating_number(value: u32) -> String {
    let digits = value.to_string();
    let mut output = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            output.push(',');
        }
        output.push(digit);
    }
    output
}

fn form_heading(count: usize) -> String {
    format!(
        "Last {count} {} · newest first",
        if count == 1 { "match" } else { "matches" }
    )
}

pub async fn stats(
    settings: &Settings,
    username: &Username,
    form_options: RecentFormOptions,
) -> String {
    let res = services::leetify::player_stats(settings, username).await;

    match res {
        Ok(stats) => {
            let aim = stats.ratings.aim;
            let positioning = stats.ratings.positioning;
            let opening = stats.ratings.opening * 100.;
            let clutch = stats.ratings.clutch * 100.;
            let utility = stats.ratings.utility;

            let fmt_leetify_stat = |stat: f32| {
                let stat = stat * 100.;
                if stat < 0. {
                    format!("−{:.2}", stat.abs())
                } else {
                    format!("{}{stat:.2}", if stat > 0. { "+" } else { "" })
                }
            };
            let ct_leetify = fmt_leetify_stat(stats.ratings.ct_leetify);
            let t_leetify = fmt_leetify_stat(stats.ratings.t_leetify);

            let premier_rank = stats
                .ranks
                .iter()
                .find(|r| r.r#type.as_deref() == Some("premier"));
            let skill_level = premier_rank
                .and_then(|r| r.skill_level)
                .map(format_rating_number)
                .unwrap_or("N/A".to_string());
            let recent_results =
                format_recent_results_with_options(&stats.recent_matches, form_options);
            let teammate_stats = match services::leetify::teammate_stats(settings, username).await {
                Ok(teammate_stats) => format_teammate_stats(&teammate_stats),
                Err(e) => {
                    eprintln!("Failed to fetch teammate stats from Leetify: {}", e);
                    "<b>Teammates</b>\nTeammate records unavailable.".to_string()
                }
            };

            let text = format!(
                "<b>Stats for {username}</b>\n\n\
                 <b>Recent form</b> · {}\n\
                 {recent_results}\n\n\
                 <b>Leetify ratings</b> · Premier {skill_level}\n\
                 Aim {aim:.1} · Positioning {positioning:.1} · Utility {utility:.1}\n\
                 Opening {opening:.1}% · Clutch {clutch:.1}%\n\
                 CT {ct_leetify} · T {t_leetify}\n\n\
                 {teammate_stats}",
                form_heading(
                    stats
                        .recent_matches
                        .len()
                        .min(services::leetify::RECENT_MATCHES_LIMIT)
                ),
                username = escape_html(&username.to_string()),
            );
            text
        }
        Err(e) => {
            eprintln!("Failed to fetch player stats from Leetify: {}", e);
            "Failed to fetch player stats from Leetify".to_string()
        }
    }
}

pub async fn recent_form(
    settings: &Settings,
    username: &Username,
    form_options: RecentFormOptions,
) -> String {
    match services::leetify::player_stats(settings, username).await {
        Ok(profile) => format!(
            "<b>{}</b> · {}\n{}",
            escape_html(&username.to_string()),
            form_heading(
                profile
                    .recent_matches
                    .len()
                    .min(services::leetify::RECENT_MATCHES_LIMIT)
            ),
            format_recent_results_with_options(&profile.recent_matches, form_options)
        ),
        Err(error) => {
            eprintln!("Failed to fetch recent form from Leetify: {error}");
            "Failed to fetch recent form from Leetify".to_string()
        }
    }
}

fn stat_type_display_name(stat_type: &str) -> String {
    match stat_type {
        "aim" => "Aim".to_string(),
        "positioning" => "Positioning".to_string(),
        "utility" => "Utility".to_string(),
        "opening" => "Opening Duels".to_string(),
        "clutch" => "Clutch".to_string(),
        _ => stat_type.to_string(),
    }
}

fn format_stat_value(stat_type: &str, value: f32) -> String {
    match stat_type {
        // Opening and clutch are stored as decimals (0.xx), display as percentages
        "opening" | "clutch" => format!("{:.1}%", value * 100.0),
        // Aim, positioning, utility are direct ratings (e.g. 0.85)
        _ => format!("{:.2}", value),
    }
}

pub async fn stat_leaderboard(settings: &Settings, stat_type: String) -> String {
    let res = services::leetify::stat_leaderboard(settings, &stat_type).await;

    match res {
        Ok(leaderboard) => {
            let stat_name = stat_type_display_name(&stat_type);
            let list = leaderboard
                .entries
                .iter()
                .take(10)
                .enumerate()
                .map(|(index, entry)| {
                    let username = &entry.username;
                    let pos = index_to_pos(index);
                    let stat_value = format_stat_value(&stat_type, entry.stat_value);

                    format!("{pos}: {username} ({stat_value})")
                })
                .collect::<Vec<String>>()
                .join("\n");

            if leaderboard.entries.is_empty() {
                return format!("No entries found for {stat_name}. ☹️");
            }

            let avg = format_stat_value(&stat_type, leaderboard.avg);
            let median = format_stat_value(&stat_type, leaderboard.median);

            format!("{stat_name} Leaderboard (top 10):\n\n{list}\n\nAvg: {avg}, Median: {median}")
        }
        Err(e) => {
            eprintln!("Failed to fetch stat leaderboard from Leetify: {}", e);
            "Failed to fetch stat leaderboard from Leetify".to_string()
        }
    }
}

fn index_to_shame_pos(index: usize) -> String {
    match index {
        0 => "💀".to_string(),
        1 => "🦴".to_string(),
        2 => "👀".to_string(),
        index => format!("#{pos}", pos = index + 1),
    }
}

pub async fn team_flash_leaderboard(settings: &Settings) -> String {
    let res = services::leetify::team_flash_leaderboard(settings).await;

    match res {
        Ok(leaderboard) => {
            let list = leaderboard
                .entries
                .iter()
                .take(10)
                .enumerate()
                .map(|(index, entry)| {
                    let username = &entry.username;
                    let pos = index_to_shame_pos(index);
                    let value = entry.teammates_flashed_per_round;

                    let thrown = entry.flashbangs_thrown_per_round;
                    let ratio = entry.teammates_flashed_per_flash;

                    format!(
                        "{pos}: {username} ({thrown:.2} thrown, {value:.2} teammates hit / round, {ratio:.2} hit / flash)"
                    )
                })
                .collect::<Vec<String>>()
                .join("\n");

            if leaderboard.entries.is_empty() {
                return "No team flash data found. ☹️".to_string();
            }

            let avg = leaderboard.avg;

            format!(
                "Flashbangs per round 💥\n(thrown, teammates hit, hit / flash)\n\n{list}\n\nAvg: {thrown_avg:.2} thrown, {avg:.2} teammates hit / round, {ratio_avg:.2} hit / flash",
                thrown_avg = leaderboard.avg_flashbangs_thrown,
                ratio_avg = leaderboard.avg_teammates_flashed_per_flash
            )
        }
        Err(e) => {
            eprintln!("Failed to fetch team flash leaderboard from Leetify: {}", e);
            "Failed to fetch team flash leaderboard from Leetify".to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::leetify::{MatchResult, RecentMatch};

    fn result(result: MatchResult) -> RecentMatch {
        RecentMatch { result }
    }

    #[test]
    fn empty_form_does_not_claim_a_zero_win_rate() {
        assert_eq!(
            format_recent_results_with_options(&[], RecentFormOptions::default()),
            "No recent results available."
        );
    }

    #[test]
    fn teammate_output_counts_ties_only_when_present_and_keeps_single_games() {
        let entry = services::leetify::TeammateStatsEntry {
            username: Username::new("Bob".into()),
            wins: 8,
            losses: 10,
            ties: 2,
        };
        assert_eq!(
            format_teammate_result(&entry),
            "Bob · 8W/10L/2T · 44% · 20 games"
        );
        let single = services::leetify::TeammateStatsEntry {
            username: Username::new("Alice".into()),
            wins: 1,
            losses: 0,
            ties: 0,
        };
        assert_eq!(format_teammate_result(&single), "Alice · 1W/0L · 1 game");
        let ties = services::leetify::TeammateStatsEntry {
            username: Username::new("Charlie".into()),
            wins: 0,
            losses: 0,
            ties: 2,
        };
        assert_eq!(
            format_teammate_result(&ties),
            "Charlie · 0W/0L/2T · — · 2 games"
        );
    }

    #[test]
    fn random_pools_are_nonempty_valid_emoji_without_duplicates_or_overlap() {
        let mut seen = std::collections::HashSet::new();
        for pool in [RANDOM_WIN_ICONS, RANDOM_LOSS_ICONS, RANDOM_TIE_ICONS] {
            assert!(!pool.is_empty());
            for icon in pool {
                assert!(emojis::get(icon).is_some(), "invalid emoji: {icon}");
                assert!(seen.insert(icon), "duplicate emoji: {icon}");
            }
        }
    }

    #[test]
    fn custom_theme_renders_results_and_legend_in_win_loss_tie_order() {
        let matches = vec![
            result(MatchResult::Win),
            result(MatchResult::Loss),
            result(MatchResult::Tie),
            result(MatchResult::Win),
            result(MatchResult::Win),
            result(MatchResult::Loss),
        ];
        let rendered = format_recent_results_with_options(
            &matches,
            RecentFormOptions {
                style: RecentFormStyle::Custom(["🍟", "🥬", "➖"].map(str::to_owned)),
                results_per_row: 5,
                explicit_style: true,
            },
        );
        assert_eq!(rendered,
            "<pre>🍟🥬➖🍟🍟\n🥬</pre>\n<b>3W · 2L · 1T · 60% win rate</b>\n<i>🍟 win · 🥬 loss · ➖ tie</i>");
    }

    #[test]
    fn random_theme_keeps_distinct_icons_consistent_with_legend() {
        let matches = vec![
            result(MatchResult::Win),
            result(MatchResult::Loss),
            result(MatchResult::Tie),
            result(MatchResult::Win),
        ];
        let rendered = format_recent_results_with_options(
            &matches,
            RecentFormOptions {
                style: RecentFormStyle::Random,
                ..RecentFormOptions::default()
            },
        );
        let legend = rendered.lines().last().unwrap();
        let icons = legend
            .trim_start_matches("<i>")
            .trim_end_matches("</i>")
            .split(" · ")
            .map(|entry| entry.split(' ').next().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(icons.len(), 3);
        assert!(RANDOM_WIN_ICONS.contains(&icons[0]));
        assert!(RANDOM_LOSS_ICONS.contains(&icons[1]));
        assert!(RANDOM_TIE_ICONS.contains(&icons[2]));
        assert_ne!(icons[0], icons[1]);
        assert_ne!(icons[0], icons[2]);
        assert_ne!(icons[1], icons[2]);
        assert_eq!(
            rendered,
            format!(
                "<pre>{}{}{}{}</pre>\n<b>2W · 1L · 1T · 67% win rate</b>\n<i>{} win · {} loss · {} tie</i>",
                icons[0], icons[1], icons[2], icons[0], icons[0], icons[1], icons[2]
            )
        );
    }

    #[test]
    fn recent_results_include_win_loss_counts_and_percentage() {
        let results = vec![
            result(MatchResult::Win),
            result(MatchResult::Loss),
            result(MatchResult::Win),
            result(MatchResult::Tie),
        ];

        assert_eq!(
            format_recent_results_with_options(&results, RecentFormOptions::default()),
            "<pre>🟩🟥🟩🟨</pre>\n<b>2W · 1L · 1T · 67% win rate</b>\n<i>🟩 win · 🟥 loss · 🟨 tie</i>"
        );
    }

    #[test]
    fn ties_do_not_make_an_all_tie_result_a_win() {
        let results = vec![result(MatchResult::Tie)];

        assert_eq!(
            format_recent_results_with_options(&results, RecentFormOptions::default()),
            "<pre>🟨</pre>\n<b>0W · 0L · 1T · — win rate</b>\n<i>🟩 win · 🟥 loss · 🟨 tie</i>"
        );
    }

    #[test]
    fn only_latest_thirty_results_are_rendered_and_counted() {
        let mut results = vec![
            result(MatchResult::Win),
            result(MatchResult::Loss),
            result(MatchResult::Tie),
            result(MatchResult::Win),
            result(MatchResult::Loss),
            result(MatchResult::Win),
            result(MatchResult::Loss),
            result(MatchResult::Win),
            result(MatchResult::Loss),
            result(MatchResult::Win),
        ];
        results.extend((0..35).map(|_| result(MatchResult::Win)));
        results.extend((0..51).map(|_| result(MatchResult::Loss)));
        results.extend((0..4).map(|_| result(MatchResult::Tie)));

        assert_eq!(
            format_recent_results_with_options(&results, RecentFormOptions::default()),
            "<pre>🟩🟥🟨🟩🟥🟩🟥🟩🟥🟩\n🟩🟩🟩🟩🟩🟩🟩🟩🟩🟩\n🟩🟩🟩🟩🟩🟩🟩🟩🟩🟩</pre>\n<b>25W · 4L · 1T · 86% win rate</b>\n<i>🟩 win · 🟥 loss · 🟨 tie</i>"
        );
    }
}
