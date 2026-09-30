use crate::services::failure;
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

fn skill_level_to_cs2_rank(skill_level: u32) -> &'static str {
    const RANKS: [&str; 19] = [
        "Unranked",
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
    RANKS
        .get(skill_level as usize)
        .copied()
        .unwrap_or("Unknown rank")
}

fn map_display_name(map: &str) -> String {
    let name = map
        .strip_prefix("de_")
        .or_else(|| map.strip_prefix("cs_"))
        .unwrap_or(map);
    let mut characters = name.chars();
    match characters.next() {
        Some(first) => format!("{}{}", first.to_uppercase(), characters.as_str()),
        None => String::new(),
    }
}

fn format_hall_of_fame(leaderboard: &services::leetify::HallOfFame, rank_type: &str) -> String {
    let name = map_display_name(rank_type);
    if leaderboard.entries.is_empty() {
        return format!("No entries found for {}. ☹️", escape_html(&name));
    }
    let premier = rank_type == "premier";
    let list = leaderboard
        .entries
        .iter()
        .take(10)
        .enumerate()
        .map(|(index, entry)| {
            let value = if premier {
                format_rating_number(entry.skill_level)
            } else {
                skill_level_to_cs2_rank(entry.skill_level).to_string()
            };
            format!(
                "{} {} · {value}",
                index_to_pos(index),
                escape_html(&entry.username.to_string())
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let summary = if premier {
        format!(
            "Group average {} · Median {}",
            format_rating_number(leaderboard.avg_skill_level.round() as u32),
            format_rating_number(leaderboard.median_skill_level)
        )
    } else {
        format!(
            "Group median · {}",
            skill_level_to_cs2_rank(leaderboard.median_skill_level)
        )
    };
    format!("🏆 {} · Top 10\n\n{list}\n\n{summary}", escape_html(&name))
}

pub async fn hall_of_fame(settings: &Settings, rank_type: String) -> String {
    match services::leetify::hall_of_fame(settings, &rank_type).await {
        Ok(leaderboard) => format_hall_of_fame(&leaderboard, &rank_type),
        Err(error) => {
            eprintln!("Failed to fetch ranks from Leetify: {error}");
            let retry = if rank_type == "premier" {
                "/halloffame".into()
            } else {
                format!("/{rank_type}")
            };
            failure::message(&error, "Rank leaderboard", &retry)
        }
    }
}

fn format_hall_of_shame(
    leaderboard: &services::leetify::HallOfShame,
    now: chrono::DateTime<Tz>,
) -> String {
    let mut sections = Vec::new();
    if !leaderboard.entries.is_empty() {
        let list = leaderboard
            .entries
            .iter()
            .enumerate()
            .map(|(index, entry)| {
                let date = entry.last_played.with_timezone(&now.timezone());
                let days_ago = (now.date_naive() - date.date_naive()).num_days();
                let noun = if days_ago == 1 { "day" } else { "days" };
                let streak = entry
                    .spree
                    .filter(|days| *days > 1)
                    .map(|days| format!(" · 🔥 {days}-day streak"))
                    .unwrap_or_default();
                format!(
                    "{} {} · {days_ago} {noun} · {}{streak}",
                    index_to_pos(index),
                    escape_html(&entry.username.to_string()),
                    date.format("%-d %b")
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        sections.push(list);
        let average = leaderboard
            .entries
            .iter()
            .map(|entry| {
                (now.date_naive()
                    - entry
                        .last_played
                        .with_timezone(&now.timezone())
                        .date_naive())
                .num_days()
            })
            .sum::<i64>()
            / leaderboard.entries.len() as i64;
        sections.push(format!(
            "Group average · {average} {}",
            if average == 1 { "day" } else { "days" }
        ));
    } else {
        sections.push("No verified squad matches found in the available history.".to_string());
    }
    if !leaderboard.unavailable.is_empty() {
        sections.push(format!(
            "Unknown: {} (no verified squad history)",
            leaderboard
                .unavailable
                .iter()
                .map(|name| escape_html(&name.to_string()))
                .collect::<Vec<_>>()
                .join(" · ")
        ));
    }
    format!(
        "💀 Hall of shame · Last recorded squad match\n\n{}",
        sections.join("\n\n")
    )
}

pub async fn hall_of_shame(settings: &Settings, tz: &Tz) -> String {
    match services::leetify::hall_of_shame(settings, *tz).await {
        Ok(leaderboard) => format_hall_of_shame(&leaderboard, Utc::now().with_timezone(tz)),
        Err(error) => {
            eprintln!("Failed to fetch squad history from Leetify: {error}");
            failure::message(&error, "Squad history", "/hallofshame")
        }
    }
}

fn format_last_played(
    result: &services::leetify::LastPlayedResult,
    username: &Username,
    now: chrono::DateTime<Tz>,
) -> String {
    let game = &result.game;
    let date = game.game_finished_at.with_timezone(&now.timezone());
    let days_ago = (now.date_naive() - date.date_naive()).num_days();
    let noun = if days_ago == 1 { "day" } else { "days" };
    let icon = match game.match_result.as_str() {
        "win" => "🟩",
        "loss" => "🟥",
        "tie" => "🟨",
        _ => "—",
    };
    let teammates = result
        .teammates
        .iter()
        .map(|name| escape_html(&name.to_string()))
        .collect::<Vec<_>>()
        .join(" · ");
    format!("🎮 {} · Last recorded squad match\n{icon} {} · {}–{}\n{} · {days_ago} {noun} ago\nWith {teammates}", escape_html(&username.to_string()), escape_html(&map_display_name(&game.map_name)), game.scores.0, game.scores.1, date.format("%-d %b %H:%M"))
}

pub async fn last_played(settings: &Settings, tz: &Tz, username: Username) -> String {
    match services::leetify::last_played(settings, &username, *tz).await {
        Ok(result) => format_last_played(&result, &username, Utc::now().with_timezone(tz)),
        Err(error) => {
            eprintln!("Failed to fetch last squad match from Leetify: {error}");
            failure::message(&error, "Last squad match", "/lastplayed")
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
                    format!(
                        "<b>Teammates</b>\n{}",
                        failure::message(&e, "Teammate records", "/stats")
                    )
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
            failure::message(&e, "Stats", "/stats")
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
            failure::message(&error, "Recent form", "/form")
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
        _ => format!("{:.1}", value),
    }
}

fn stat_icon(stat_type: &str) -> &'static str {
    match stat_type {
        "aim" => "🎯",
        "positioning" => "🧭",
        "utility" => "🧨",
        "opening" => "⚔️",
        "clutch" => "🔥",
        _ => "🏆",
    }
}

fn format_stat_leaderboard(
    leaderboard: &services::leetify::StatLeaderboard,
    stat_type: &str,
) -> String {
    let name = stat_type_display_name(stat_type);
    if leaderboard.entries.is_empty() {
        return format!("No entries found for {name}. ☹️");
    }
    let list = leaderboard
        .entries
        .iter()
        .take(10)
        .enumerate()
        .map(|(index, entry)| {
            format!(
                "{} {} · {}",
                index_to_pos(index),
                escape_html(&entry.username.to_string()),
                format_stat_value(stat_type, entry.stat_value)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "{} {name} · Top 10\n\n{list}\n\nGroup average {} · Median {}",
        stat_icon(stat_type),
        format_stat_value(stat_type, leaderboard.avg),
        format_stat_value(stat_type, leaderboard.median)
    )
}

pub async fn stat_leaderboard(settings: &Settings, stat_type: String) -> String {
    match services::leetify::stat_leaderboard(settings, &stat_type).await {
        Ok(leaderboard) => format_stat_leaderboard(&leaderboard, &stat_type),
        Err(error) => {
            eprintln!("Failed to fetch stat leaderboard from Leetify: {error}");
            failure::message(&error, "Leaderboard", &format!("/{stat_type}"))
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

fn format_team_flash_leaderboard(leaderboard: &services::leetify::TeamFlashLeaderboard) -> String {
    if leaderboard.entries.is_empty() {
        return "No team flash data found. ☹️".to_string();
    }
    let list = leaderboard
        .entries
        .iter()
        .take(10)
        .enumerate()
        .map(|(index, entry)| {
            format!(
                "{} {} · {:.0} hits · {:.0} thrown · {:.2} hits/flash",
                index_to_shame_pos(index),
                escape_html(&entry.username.to_string()),
                entry.teammates_flashed_per_round * 100.0,
                entry.flashbangs_thrown_per_round * 100.0,
                entry.teammates_flashed_per_flash
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!("💥 Team flashes\nThrown and teammate hits per 100 rounds\n\n{list}\n\nGroup average · {:.0} hits · {:.0} thrown · {:.2} hits/flash", leaderboard.avg * 100.0, leaderboard.avg_flashbangs_thrown * 100.0, leaderboard.avg_teammates_flashed_per_flash)
}

pub async fn team_flash_leaderboard(settings: &Settings) -> String {
    match services::leetify::team_flash_leaderboard(settings).await {
        Ok(leaderboard) => format_team_flash_leaderboard(&leaderboard),
        Err(error) => {
            eprintln!("Failed to fetch team flash leaderboard from Leetify: {error}");
            failure::message(&error, "Team flashes", "/teamflash")
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
    fn team_flash_output_converts_both_rates_and_keeps_hits_per_flash() {
        let leaderboard = services::leetify::TeamFlashLeaderboard {
            entries: vec![services::leetify::TeamFlashEntry {
                username: Username::new("Rasse".into()),
                flashbangs_thrown_per_round: 0.75,
                teammates_flashed_per_round: 0.22,
                teammates_flashed_per_flash: 0.29,
            }],
            avg_flashbangs_thrown: 0.60,
            avg: 0.15,
            avg_teammates_flashed_per_flash: 0.25,
        };
        assert_eq!(format_team_flash_leaderboard(&leaderboard), "💥 Team flashes\nThrown and teammate hits per 100 rounds\n\n💀 Rasse · 22 hits · 75 thrown · 0.29 hits/flash\n\nGroup average · 15 hits · 60 thrown · 0.25 hits/flash");
    }

    #[test]
    fn metric_leaderboards_keep_percentage_units_and_one_decimal() {
        let leaderboard = services::leetify::StatLeaderboard {
            stat_type: "opening".into(),
            entries: vec![services::leetify::StatLeaderboardEntry {
                username: Username::new("A&B".into()),
                stat_value: 0.5231,
            }],
            avg: 0.51,
            median: 0.52,
        };
        assert_eq!(
            format_stat_leaderboard(&leaderboard, "opening"),
            "⚔️ Opening Duels · Top 10\n\n🥇 A&amp;B · 52.3%\n\nGroup average 51.0% · Median 52.0%"
        );
        assert_eq!(format_stat_value("aim", 67.12), "67.1");
    }

    #[test]
    fn premier_and_named_rank_leaderboards_have_distinct_summaries() {
        let mut leaderboard = services::leetify::HallOfFame {
            entries: vec![services::leetify::HallOfFameEntry {
                username: Username::new("Rasse".into()),
                skill_level: 15432,
            }],
            avg_skill_level: 13815.0,
            median_skill_level: 14010,
        };
        assert_eq!(
            format_hall_of_fame(&leaderboard, "premier"),
            "🏆 Premier · Top 10\n\n🥇 Rasse · 15,432\n\nGroup average 13,815 · Median 14,010"
        );
        leaderboard.entries[0].skill_level = 13;
        leaderboard.median_skill_level = 12;
        assert_eq!(format_hall_of_fame(&leaderboard, "de_mirage"), "🏆 Mirage · Top 10\n\n🥇 Rasse · Master Guardian Elite\n\nGroup median · Master Guardian II");
        assert!(format_hall_of_fame(&leaderboard, "wingman").starts_with("🏆 Wingman · Top 10"));
    }

    fn last_played_fixture() -> services::leetify::LastPlayedResult {
        services::leetify::LastPlayedResult {
            game: services::leetify::LeetifyGame {
                id: Some("test".into()),
                own_team_steam64_ids: vec![],
                game_finished_at: chrono::DateTime::parse_from_rfc3339("2026-09-28T18:43:12Z")
                    .unwrap()
                    .with_timezone(&Utc),
                map_name: "de_mirage".into(),
                match_result: "win".into(),
                scores: (13, 9),
                skill_level: None,
                teammates_flashed: None,
                flashbangs_thrown: None,
                rounds_count: None,
            },
            teammates: vec![Username::new("Alice".into()), Username::new("Bob".into())],
            spree: None,
        }
    }

    #[test]
    fn last_played_shows_local_time_friendly_map_result_and_teammates() {
        use chrono::TimeZone;
        let now = chrono_tz::Europe::Helsinki
            .with_ymd_and_hms(2026, 9, 30, 12, 0, 0)
            .unwrap();
        assert_eq!(format_last_played(&last_played_fixture(), &Username::new("Rasse".into()), now), "🎮 Rasse · Last recorded squad match\n🟩 Mirage · 13–9\n28 Sep 21:43 · 2 days ago\nWith Alice · Bob");
    }

    #[test]
    fn inactivity_uses_local_days_keeps_streaks_and_excludes_unknowns_from_average() {
        use chrono::TimeZone;
        let now = chrono_tz::Europe::Helsinki
            .with_ymd_and_hms(2026, 10, 1, 0, 30, 0)
            .unwrap();
        let leaderboard = services::leetify::HallOfShame {
            entries: vec![services::leetify::HallOfShameEntry {
                username: Username::new("Charlie".into()),
                spree: Some(3),
                last_played: chrono::DateTime::parse_from_rfc3339("2026-09-29T21:30:00Z")
                    .unwrap()
                    .with_timezone(&Utc),
            }],
            unavailable: vec![Username::new("A&B".into())],
        };
        assert_eq!(format_hall_of_shame(&leaderboard, now), "💀 Hall of shame · Last recorded squad match\n\n🥇 Charlie · 1 day · 30 Sep · 🔥 3-day streak\n\nGroup average · 1 day\n\nUnknown: A&amp;B (no verified squad history)");
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
