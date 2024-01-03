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
                    let skill_level = entry.skill_level;

                    format!("{pos}: {username} (rating: {skill_level})")
                })
                .collect::<Vec<String>>()
                .join("\n");

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
            let list = entries
                .iter()
                .take(10)
                .enumerate()
                .map(|(index, entry)| {
                    let t = entry.last_played.with_timezone(&tz.clone());
                    let t = t.format("%Y-%m-%d %H:%M:%S");
                    let days_ago = (Utc::now().with_timezone(tz).date_naive()
                        - entry.last_played.with_timezone(tz).date_naive())
                    .num_days();
                    let days = if days_ago == 1 { "day" } else { "days" };
                    let username = &entry.username;
                    let pos = index_to_pos(index);

                    format!("{pos} {t} ({days_ago} {days} ago): {username}")
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

            let median = days_since_last_played
                .get(days_since_last_played.len() / 2)
                .unwrap_or(&0);

            format!("Hall of shame, or longest time since last played with team:\n\n{list}\n\nAvg: {avg:.0} days, Median: {median:.0} days",)
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
            let leetify = fmt_leetify_stat(stats.ratings.leetify);
            let t_leetify = fmt_leetify_stat(stats.ratings.t_leetify);

            let leetify = format!("{leetify} (CT: {ct_leetify} / T: {t_leetify})",);
            let premier_rank = stats
                .ranks
                .iter()
                .find(|r| r.r#type.as_deref() == Some("premier"));
            let skill_level = premier_rank
                .and_then(|r| r.skill_level)
                .map(|r| r.to_string())
                .unwrap_or("N/A".to_string());

            let text = format!("Stats for {username} from last 30 matches:\n- Leetify rating: {leetify}\n- Aim: {aim:.2}\n- Positioning: {positioning:.2}\n- Utility: {utility:.2}\n- Opening duels: {opening:.2}\n- Clutch: {clutch:.2}\n- Premier rating: {skill_level}");
            text
        }
        Err(e) => {
            eprintln!("Failed to fetch player stats from Leetify: {}", e);
            "Failed to fetch player stats from Leetify".to_string()
        }
    }
}
