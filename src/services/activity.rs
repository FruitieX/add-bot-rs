use super::{
    chart_style::{self, BORDER, INK, MUTED, WIDTH},
    chart_text::ChartBackend,
};
use std::collections::{HashMap, HashSet};

use chrono::{Duration, NaiveDate, Utc};
use color_eyre::{eyre::eyre, Result};
use plotters::{
    chart::{ChartBuilder, LabelAreaPosition},
    prelude::{BitMapBackend, IntoDrawingArea, PathElement, Rectangle, Text},
    style::{
        text_anchor::{HPos, Pos, VPos},
        Color, IntoFont, RGBColor, TextStyle, WHITE,
    },
};

use crate::{
    services::leetify::{get_leetify_games, LeetifyGame},
    settings::Settings,
};

/// Generate a bar chart over the last 90 days of how many unique games the configured
/// players have played. The public Leetify API no longer exposes teammate rosters,
/// so games are deduplicated by match ID instead of being restricted to configured teams.
use crate::types::Username;

fn game_key(game: &LeetifyGame) -> String {
    game.id.clone().unwrap_or_else(|| {
        format!(
            "{}:{}:{}-{}",
            game.game_finished_at.timestamp(),
            game.map_name,
            game.scores.0,
            game.scores.1
        )
    })
}

pub async fn get_activity_chart(
    settings: &Settings,
    filter_user: Option<&Username>,
) -> Result<Vec<u8>> {
    // Gather games per player in parallel.
    let mappings = settings.players.steamid_mappings.clone();
    let futures: Vec<_> = mappings
        .into_iter()
        .map(|(username, steamid)| {
            let settings = settings.clone();
            async move { (username, get_leetify_games(&settings, &steamid).await) }
        })
        .collect();
    let player_results = futures::future::join_all(futures).await;

    // Keep per-player games (raw) and master list for total aggregation
    let mut per_player_games: Vec<(String, Vec<LeetifyGame>)> = Vec::new();
    let mut all_games: Vec<LeetifyGame> = Vec::new();
    for (username, maybe_stats) in player_results.into_iter() {
        if let Some(games) = maybe_stats {
            let un = username.to_string();
            let include = match filter_user {
                Some(fu) => *fu == username,
                None => true,
            };
            if include {
                all_games.extend(games.clone());
            }
            per_player_games.push((un, games));
        }
    }

    render_activity_chart(per_player_games, all_games, filter_user)
}

fn render_activity_chart(
    per_player_games: Vec<(String, Vec<LeetifyGame>)>,
    all_games: Vec<LeetifyGame>,
    filter_user: Option<&Username>,
) -> Result<Vec<u8>> {
    if all_games.is_empty() {
        return Err(eyre!("No games found for any configured player"));
    }

    let today = Utc::now().date_naive();
    // Window length (adjust here to change chart span)
    let span_days = 90;
    let start = today - Duration::days(span_days);

    // Deduplicate games across players using the public API's match ID.
    let mut seen: HashSet<String> = HashSet::new();
    let mut counts: HashMap<NaiveDate, u32> = HashMap::new();
    for g in all_games.into_iter() {
        let key = game_key(&g);
        if !seen.insert(key.clone()) {
            continue;
        }
        let d = g.game_finished_at.date_naive();
        if d >= start && d <= today {
            *counts.entry(d).or_insert(0) += 1;
        }
    }

    // Ensure all days present with 0
    let mut dates: Vec<NaiveDate> = Vec::new();
    let mut cur = start;
    while cur <= today {
        dates.push(cur);
        cur = cur.succ_opt().unwrap();
    }

    // Build daily participants - behavior changes based on filter_user
    // Map date -> set of participating configured usernames (including filtered user when in filtered mode)
    let mut daily_participants: HashMap<NaiveDate, HashSet<String>> = HashMap::new();
    let mut seen_global_games: HashSet<String> = HashSet::new();

    if let Some(filtered_user) = filter_user {
        // When filtering by user, only track games where the filtered user participated
        // and show their teammates (and the filtered user themselves)
        let filtered_username = filtered_user.to_string();

        if let Some((_, filtered_games)) = per_player_games
            .iter()
            .find(|(username, _)| *username == filtered_username)
        {
            // For counting how many unique games filtered user had in timeframe
            let mut filtered_seen: HashSet<String> = HashSet::new();

            for g in filtered_games.iter() {
                let key = game_key(g);
                let d = g.game_finished_at.date_naive();

                if d >= start && d <= today && seen_global_games.insert(key.clone()) {
                    // Add the filtered user into the day's participants
                    daily_participants
                        .entry(d)
                        .or_default()
                        .insert(filtered_username.clone());

                    // Count this game for filtered user
                    filtered_seen.insert(key.clone());

                    // Find ALL configured players who participated in this game with the filtered user
                    for (other_username, other_games) in per_player_games.iter() {
                        for other_game in other_games.iter() {
                            let other_key = game_key(other_game);

                            // Same game - this player was a teammate
                            if key == other_key && *other_username != filtered_username {
                                daily_participants
                                    .entry(d)
                                    .or_default()
                                    .insert(other_username.clone());
                            }
                        }
                    }
                }
            }
        }
    } else {
        // Original logic for global view
        for (_username, games) in per_player_games.iter() {
            for g in games.iter() {
                let key = game_key(g);
                let d = g.game_finished_at.date_naive();

                if d >= start && d <= today && seen_global_games.insert(key.clone()) {
                    // For each unique game, find ALL configured players who participated
                    for (other_username, other_games) in per_player_games.iter() {
                        for other_game in other_games.iter() {
                            let other_key = game_key(other_game);

                            // Same game - this player participated
                            if key == other_key {
                                daily_participants
                                    .entry(d)
                                    .or_default()
                                    .insert(other_username.clone());
                            }
                        }
                    }
                }
            }
        }
    }

    // Per-player daily (unique games per player)
    let mut per_player_daily: Vec<(String, HashMap<NaiveDate, u32>)> = Vec::new();
    for (username, games) in per_player_games.iter() {
        let mut map: HashMap<NaiveDate, u32> = HashMap::new();
        let mut seen_player: HashSet<String> = HashSet::new();
        for g in games.iter() {
            let key = game_key(g);
            if !seen_player.insert(key.clone()) {
                continue;
            }
            let d = g.game_finished_at.date_naive();
            if d >= start && d <= today {
                *map.entry(d).or_insert(0) += 1;
            }
        }
        // Only retain per-player daily if no filter or this is the filtered user
        if filter_user
            .map(|fu| fu.to_string() == *username)
            .unwrap_or(true)
        {
            per_player_daily.push((username.clone(), map));
        }
    }

    let max_count = counts.values().copied().max().unwrap_or(1).max(5);

    // Participant colors retain the existing activity aggregation semantics.
    // Calculate teammate frequency for filtered user or global player totals
    // Build a map of username -> set of unique game keys (within timeframe) so we can compute
    // intersections per game (avoids double-counting and matches per-player unique-game counts)
    let mut player_keys_map: HashMap<String, HashSet<String>> = HashMap::new();
    for (username, games) in per_player_games.iter() {
        let mut keys: HashSet<String> = HashSet::new();
        for g in games.iter() {
            let d = g.game_finished_at.date_naive();
            if d < start || d > today {
                continue;
            }
            let key = game_key(g);
            keys.insert(key);
        }
        player_keys_map.insert(username.clone(), keys);
    }

    // top_players_with_counts: Vec<(username, count)>
    let top_players_with_counts: Vec<(String, u32)> = if let Some(filtered_user) = filter_user {
        // When filtering, compute counts per GAME (not per day). Show teammates ordered by how
        // often they played in the same game as the filtered user. Include the filtered user
        // themself first with their total number of unique games in timeframe.
        let filtered_username = filtered_user.to_string();

        let filtered_set = player_keys_map
            .get(&filtered_username)
            .cloned()
            .unwrap_or_default();
        let filtered_games_count = filtered_set.len() as u32;

        // For each other player, compute intersection size with filtered_set
        let mut teammate_totals: Vec<(String, u32)> = Vec::new();
        for (other_username, other_set) in player_keys_map.iter() {
            if other_username == &filtered_username {
                continue;
            }
            let mut inter: u32 = 0;
            for k in filtered_set.iter() {
                if other_set.contains(k) {
                    inter += 1;
                }
            }
            if inter > 0 {
                teammate_totals.push((other_username.clone(), inter));
            }
        }
        teammate_totals.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

        // Build vector including filtered user first
        let mut v: Vec<(String, u32)> = Vec::new();
        v.push((filtered_username.clone(), filtered_games_count));
        for (name, cnt) in teammate_totals.into_iter().take(9) {
            v.push((name, cnt));
        }
        v
    } else {
        // Original logic for global view - top players by total games
        let mut player_totals: Vec<(String, u32)> = per_player_daily
            .iter()
            .map(|(username, daily_counts)| {
                let total = daily_counts.values().sum::<u32>();
                (username.clone(), total)
            })
            .collect();

        player_totals.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

        player_totals.into_iter().take(10).collect()
    };

    // Build a helper vector of top names for quick lookup and mapping index -> name
    let top_names: Vec<String> = top_players_with_counts
        .iter()
        .map(|(n, _)| n.clone())
        .collect();

    let palette = colorous::TABLEAU10;
    let others_color = RGBColor(128, 128, 128);

    let has_others = daily_participants
        .values()
        .any(|players| players.iter().any(|name| !top_names.contains(name)));
    let legend_entries = top_players_with_counts.len() + usize::from(has_others);
    let legend_rows = legend_entries.div_ceil(2);
    let width = WIDTH as usize;
    let height = 1040 + legend_rows * 44;
    let mut buffer = vec![0; width * height * 3];
    {
        let root = ChartBackend(BitMapBackend::with_buffer(
            &mut buffer,
            (width as u32, height as u32),
        ))
        .into_drawing_area();
        chart_style::frame(&root)?;
        chart_style::text(&root, "Match activity", (100, 86), 46, INK, true)?;
        let scope = filter_user
            .map(ToString::to_string)
            .unwrap_or_else(|| "Configured players".into());
        chart_style::text(
            &root,
            &chart_style::fit_text(
                &format!(
                    "{scope}  ·  {} – {}  ·  UTC dates",
                    start.format("%-d %b %Y"),
                    today.format("%-d %b %Y")
                ),
                22,
                1300,
            ),
            (100, 150),
            22,
            MUTED,
            false,
        )?;
        let total = counts.values().sum::<u32>();
        let active_days = counts.values().filter(|count| **count > 0).count();
        let busiest = counts.values().copied().max().unwrap_or(0);
        for (x, value, label) in [
            (100, total.to_string(), "RECORDED MATCHES"),
            (590, active_days.to_string(), "ACTIVE DAYS"),
            (1080, busiest.to_string(), "BUSIEST DAY · MATCHES"),
        ] {
            chart_style::text(&root, &value, (x, 210), 46, INK, true)?;
            chart_style::text(&root, label, (x, 270), 20, MUTED, true)?;
        }
        let plot = root.clone().shrink((80, 330), (1340, 480));
        let mut ctx = ChartBuilder::on(&plot)
            .set_label_area_size(LabelAreaPosition::Left, 48)
            .set_label_area_size(LabelAreaPosition::Bottom, 0)
            .margin_right(10)
            .margin_top(10)
            .build_cartesian_2d(
                start..today.succ_opt().unwrap_or(today),
                0f32..(max_count as f32 + 1.0),
            )?;
        ctx.configure_mesh()
            .x_labels(0)
            .y_labels(6)
            .disable_x_mesh()
            .axis_style(WHITE)
            .bold_line_style(BORDER)
            .light_line_style(WHITE)
            .y_label_style(("sans-serif", 22).into_font().color(&MUTED))
            .y_label_formatter(&|v| {
                if v.fract().abs() < 0.01 {
                    format!("{v:.0}")
                } else {
                    String::new()
                }
            })
            .set_all_tick_mark_size(0)
            .draw()?;
        // Date labels use the chart's actual coordinate transform.
        for offset in [0, 15, 30, 45, 60, 75, 90] {
            let date = start + Duration::days(offset);
            if date > today {
                continue;
            }
            let point = ctx.backend_coord(&(date, 0.0));
            root.draw(&Text::new(
                date.format("%-d %b").to_string(),
                (point.0, 835),
                TextStyle::from(("sans-serif", 22))
                    .color(&MUTED)
                    .pos(Pos::new(HPos::Center, VPos::Top)),
            ))?;
        }

        // Draw segmented bars
        for d in dates.iter() {
            let total_count = *counts.get(d).unwrap_or(&0) as i32;
            if total_count == 0 {
                continue;
            }

            // Get participants for this day - behavior depends on filter
            let participants_this_day = daily_participants.get(d);
            let mut players_that_day: Vec<String> = Vec::new();
            let mut has_others_that_day = false;

            if let Some(participants) = participants_this_day {
                for participant in participants.iter() {
                    if top_names.contains(participant) {
                        players_that_day.push(participant.clone());
                    } else {
                        has_others_that_day = true;
                    }
                }

                if has_others_that_day {
                    players_that_day.push("Others".to_string());
                }
            }

            // For filtered users, if no teammates found, skip this day
            // For global view, if no players found, skip this day
            if players_that_day.is_empty() {
                continue;
            }

            // Sort players by their position in top_names list for consistent order
            players_that_day.sort_by(|a, b| {
                if a == "Others" {
                    std::cmp::Ordering::Greater // Others goes last
                } else if b == "Others" {
                    std::cmp::Ordering::Less
                } else {
                    let idx_a = top_names.iter().position(|p| p == a).unwrap_or(999);
                    let idx_b = top_names.iter().position(|p| p == b).unwrap_or(999);
                    idx_a.cmp(&idx_b)
                }
            });

            // Use floating point calculation but round to integers for drawing
            let num_players = players_that_day.len() as f64;
            let segment_height_f64 = total_count as f64 / num_players;

            let mut current_height_f64 = 0.0f64;
            let next = d.succ_opt().unwrap_or(*d);

            for username in players_that_day.into_iter() {
                let rgb = if username == "Others" {
                    others_color
                } else if let Some(color_idx) = top_names.iter().position(|p| p == &username) {
                    let color = palette[color_idx % palette.len()];
                    RGBColor(color.r, color.g, color.b)
                } else {
                    continue;
                };

                let bottom_height = current_height_f64;
                current_height_f64 += segment_height_f64;
                let top_height = current_height_f64;

                let segment_height = top_height - bottom_height;

                if segment_height > 0.0 {
                    ctx.draw_series(std::iter::once(Rectangle::new(
                        [(*d, bottom_height as f32), (next, top_height as f32)],
                        rgb.filled(),
                    )))?;
                }
            }
        }

        root.draw(&PathElement::new(vec![(100, 885), (1400, 885)], BORDER))?;
        chart_style::text(
            &root,
            if filter_user.is_some() {
                "PLAYERS IN SHARED MATCHES · RECORDED COUNTS"
            } else {
                "PLAYERS · RECORDED MATCH COUNTS"
            },
            (100, 910),
            23,
            INK,
            true,
        )?;
        let mut entries = top_players_with_counts
            .iter()
            .enumerate()
            .map(|(index, (name, count))| {
                let color = palette[index % palette.len()];
                (
                    format!("{} · {count}", chart_style::fit_text(name, 22, 440)),
                    RGBColor(color.r, color.g, color.b),
                )
            })
            .collect::<Vec<_>>();
        if has_others {
            entries.push(("Others".into(), others_color));
        }
        for (index, (label, color)) in entries.iter().enumerate() {
            let x = 100 + (index % 2) as i32 * 660;
            let y = 958 + (index / 2) as i32 * 44;
            root.draw(&Rectangle::new(
                [(x, y + 3), (x + 18, y + 21)],
                color.filled(),
            ))?;
            chart_style::text(
                &root,
                &chart_style::fit_text(label, 22, 580),
                (x + 32, y),
                22,
                INK,
                false,
            )?;
        }
        chart_style::text(
            &root,
            "Colors mark participants; segment sizes are not player match totals.",
            (100, height as i32 - 65),
            18,
            MUTED,
            false,
        )?;

        root.present()?;
    }

    let image = image::RgbImage::from_raw(width as u32, height as u32, buffer)
        .ok_or_else(|| eyre!("Image buffer not large enough"))?;

    let mut bytes: Vec<u8> = Vec::new();
    image.write_to(
        &mut std::io::Cursor::new(&mut bytes),
        image::ImageFormat::Png,
    )?;

    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "writes sample charts for manual visual review"]
    fn write_activity_preview() {
        let mut games = Vec::new();
        for day in (0..90).step_by(3) {
            for index in 0..=day % 4 {
                games.push(LeetifyGame {
                    id: Some(format!("{day}-{index}")),
                    own_team_steam64_ids: vec![],
                    game_finished_at: Utc::now() - Duration::days(day),
                    map_name: "de_nuke".into(),
                    match_result: "win".into(),
                    scores: (13, 9),
                    skill_level: None,
                    teammates_flashed: None,
                    flashbangs_thrown: None,
                    rounds_count: None,
                });
            }
        }
        let bob = games.iter().step_by(2).cloned().collect::<Vec<_>>();
        let players = vec![("Alice".into(), games.clone()), ("Bob".into(), bob.clone())];
        let all_games = games.iter().chain(&bob).cloned().collect();
        let chart = render_activity_chart(players.clone(), all_games, None).unwrap();
        std::fs::write("target/activity-preview.png", chart).unwrap();
        let chart =
            render_activity_chart(players, games.clone(), Some(&Username::new("Alice".into())))
                .unwrap();
        std::fs::write("target/activity-player-preview.png", chart).unwrap();
        let players = (0..12).map(|index| (format!("Player {index:02} with an exceptionally long display name for layout review"),games.clone())).collect();
        std::fs::write(
            "target/activity-long-names-preview.png",
            render_activity_chart(players, games, None).unwrap(),
        )
        .unwrap();
    }
}
