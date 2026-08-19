use chrono::Utc;
use chrono_tz::Tz;

use crate::{services, settings::Settings, types::Username};

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

fn format_recent_results(recent_matches: &[services::leetify::RecentMatch]) -> String {
    const DISPLAYED_RESULTS: usize = 10;

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
    let decisive_matches = wins + losses;
    let win_percentage = if decisive_matches == 0 {
        0.0
    } else {
        wins as f32 / decisive_matches as f32 * 100.0
    };
    let results = recent_matches
        .iter()
        .take(DISPLAYED_RESULTS)
        .map(|m| m.result.to_string())
        .collect::<Vec<_>>()
        .join(" ");

    format!("{results} ({wins}W/{losses}L/{ties}T, {win_percentage:.0}% win rate)")
}

pub async fn stats(settings: &Settings, username: &Username) -> String {
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
                let sign = if stat > 0. { "+" } else { "" };
                format!("{sign}{stat:.2}")
            };
            let ct_leetify = fmt_leetify_stat(stats.ratings.ct_leetify);
            let t_leetify = fmt_leetify_stat(stats.ratings.t_leetify);

            let premier_rank = stats
                .ranks
                .iter()
                .find(|r| r.r#type.as_deref() == Some("premier"));
            let skill_level = premier_rank
                .and_then(|r| r.skill_level)
                .map(|r| r.to_string())
                .unwrap_or("N/A".to_string());
            let recent_results = format_recent_results(&stats.recent_matches);

            let text = format!("Stats for {username} from last 30 matches:\n- CT Leetify rating: {ct_leetify}\n- T Leetify rating: {t_leetify}\n- Aim: {aim:.2}\n- Positioning: {positioning:.2}\n- Utility: {utility:.2}\n- Opening duels: {opening:.2}\n- Clutch: {clutch:.2}\n- Premier rating: {skill_level}\n- Recent results: {recent_results}");
            text
        }
        Err(e) => {
            eprintln!("Failed to fetch player stats from Leetify: {}", e);
            "Failed to fetch player stats from Leetify".to_string()
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
    fn recent_results_include_win_loss_counts_and_percentage() {
        let results = vec![
            result(MatchResult::Win),
            result(MatchResult::Loss),
            result(MatchResult::Win),
            result(MatchResult::Tie),
        ];

        assert_eq!(
            format_recent_results(&results),
            "W L W T (2W/1L/1T, 67% win rate)"
        );
    }

    #[test]
    fn ties_do_not_make_an_all_tie_result_a_win() {
        let results = vec![result(MatchResult::Tie)];

        assert_eq!(format_recent_results(&results), "T (0W/0L/1T, 0% win rate)");
    }

    #[test]
    fn only_ten_results_are_rendered_but_all_100_results_are_counted() {
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
            format_recent_results(&results),
            "W L T W L W L W L W (40W/55L/5T, 42% win rate)"
        );
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

                    format!("{pos}: {username} ({value:.2}/round)")
                })
                .collect::<Vec<String>>()
                .join("\n");

            if leaderboard.entries.is_empty() {
                return "No team flash data found. ☹️".to_string();
            }

            let avg = leaderboard.avg;

            format!(
                "Team Flash Hall of Shame 💥\n(teammates flashed per round)\n\n{list}\n\nAvg: {avg:.2}/round"
            )
        }
        Err(e) => {
            eprintln!("Failed to fetch team flash leaderboard from Leetify: {}", e);
            "Failed to fetch team flash leaderboard from Leetify".to_string()
        }
    }
}
