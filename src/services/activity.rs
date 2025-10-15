use std::collections::{HashMap, HashSet};

use chrono::{Datelike, Duration, NaiveDate, Utc};
use color_eyre::{eyre::eyre, Result};
use plotters::{
    chart::{ChartBuilder, LabelAreaPosition},
    prelude::{BitMapBackend, IntoDrawingArea, LineSeries, PathElement, Rectangle, Text},
    style::{
        self, register_font,
        text_anchor::{HPos, Pos, VPos},
        Color, IntoFont, RGBColor, TextStyle, BLACK, WHITE,
    },
};

use crate::{
    services::leetify::{get_leetify_stats, LeetifyGame},
    settings::Settings,
};

/// Generate a bar chart over the last year of how many unique games the configured
/// players have played (team games counted only once even if multiple configured players
/// participated).
use crate::types::Username;

pub async fn get_activity_chart(
    settings: &Settings,
    filter_user: Option<&Username>,
) -> Result<Vec<u8>> {
    const SHOW_PLAYER_LINES: bool = false; // feature flag for per-player contributions
                                           // Gather games per player in parallel
    let mappings = settings.players.steamid_mappings.clone();
    let futures: Vec<_> = mappings
        .into_iter()
        .map(|(username, steamid)| async move { (username, get_leetify_stats(steamid).await) })
        .collect();
    let player_results = futures::future::join_all(futures).await; // Vec<(Username, Option<Value>)>

    // Get all configured SteamIDs for team game filtering
    let all_configured_steamids: Vec<String> = settings
        .players
        .steamid_mappings
        .values()
        .map(|s| s.to_string())
        .collect();

    // Keep per-player games (raw) and master list for total aggregation
    let mut per_player_games: Vec<(String, Vec<LeetifyGame>)> = Vec::new();
    let mut all_games: Vec<LeetifyGame> = Vec::new();
    for (username, maybe_stats) in player_results.into_iter() {
        if let Some(stats) = maybe_stats {
            if let Some(games_field) = stats.get("games") {
                if let Ok(games) = serde_json::from_value::<Vec<LeetifyGame>>(games_field.clone()) {
                    // Filter games to only include those with 2+ configured players
                    let filtered_games: Vec<LeetifyGame> = games
                        .into_iter()
                        .filter(|game| {
                            // Count how many configured players were in this game
                            let configured_players_in_game = all_configured_steamids
                                .iter()
                                .filter(|steam_id| {
                                    game.own_team_steam64_ids
                                        .iter()
                                        .any(|id| id.to_string() == **steam_id)
                                })
                                .count();

                            // Only include games where 2+ configured players participated
                            configured_players_in_game >= 2
                        })
                        .collect();

                    let un = username.to_string();
                    let include = match filter_user {
                        Some(fu) => *fu == username,
                        None => true,
                    };
                    if include {
                        all_games.extend(filtered_games.clone());
                    }
                    per_player_games.push((un, filtered_games));
                }
            }
        }
    }

    if all_games.is_empty() {
        return Err(eyre!("No games found for any configured player"));
    }

    let today = Utc::now().date_naive();
    // Window length (adjust here to change chart span)
    let span_days = 90;
    let start = today - Duration::days(span_days);

    // Deduplicate games across players (or within single player if filtered). Since API doesn't expose an explicit match id in
    // LeetifyGame, construct a synthetic key from (finish timestamp + map + score).
    let mut seen: HashSet<String> = HashSet::new();
    let mut counts: HashMap<NaiveDate, u32> = HashMap::new();
    for g in all_games.into_iter() {
        let key = format!(
            "{}:{}:{}-{}",
            g.game_finished_at.timestamp(),
            g.map_name,
            g.scores.0,
            g.scores.1
        );
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
    let mut daily_participants: HashMap<NaiveDate, HashSet<String>> = HashMap::new();
    let mut seen_global_games: HashSet<String> = HashSet::new();

    if let Some(filtered_user) = filter_user {
        // When filtering by user, only track games where the filtered user participated
        // and show their teammates
        let filtered_username = filtered_user.to_string();

        if let Some((_, filtered_games)) = per_player_games
            .iter()
            .find(|(username, _)| *username == filtered_username)
        {
            for g in filtered_games.iter() {
                let key = format!(
                    "{}:{}:{}-{}",
                    g.game_finished_at.timestamp(),
                    g.map_name,
                    g.scores.0,
                    g.scores.1
                );
                let d = g.game_finished_at.date_naive();

                if d >= start && d <= today && seen_global_games.insert(key.clone()) {
                    // Find ALL configured players who participated in this game with the filtered user
                    for (other_username, other_games) in per_player_games.iter() {
                        for other_game in other_games.iter() {
                            let other_key = format!(
                                "{}:{}:{}-{}",
                                other_game.game_finished_at.timestamp(),
                                other_game.map_name,
                                other_game.scores.0,
                                other_game.scores.1
                            );

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
                let key = format!(
                    "{}:{}:{}-{}",
                    g.game_finished_at.timestamp(),
                    g.map_name,
                    g.scores.0,
                    g.scores.1
                );
                let d = g.game_finished_at.date_naive();

                if d >= start && d <= today && seen_global_games.insert(key.clone()) {
                    // For each unique game, find ALL configured players who participated
                    for (other_username, other_games) in per_player_games.iter() {
                        for other_game in other_games.iter() {
                            let other_key = format!(
                                "{}:{}:{}-{}",
                                other_game.game_finished_at.timestamp(),
                                other_game.map_name,
                                other_game.scores.0,
                                other_game.scores.1
                            );

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
    for (username, games) in per_player_games.into_iter() {
        let mut map: HashMap<NaiveDate, u32> = HashMap::new();
        let mut seen_player: HashSet<String> = HashSet::new();
        for g in games.into_iter() {
            let key = format!(
                "{}:{}:{}-{}",
                g.game_finished_at.timestamp(),
                g.map_name,
                g.scores.0,
                g.scores.1
            );
            if !seen_player.insert(key) {
                continue;
            }
            let d = g.game_finished_at.date_naive();
            if d >= start && d <= today {
                *map.entry(d).or_insert(0) += 1;
            }
        }
        // Only retain per-player daily if no filter or this is the filtered user
        if filter_user
            .map(|fu| fu.to_string() == username)
            .unwrap_or(true)
        {
            per_player_daily.push((username, map));
        }
    }

    let mut max_count = counts.values().copied().max().unwrap_or(1);
    if SHOW_PLAYER_LINES {
        for (_, m) in per_player_daily.iter() {
            if let Some(mc) = m.values().copied().max() {
                if mc > max_count {
                    max_count = mc;
                }
            }
        }
    }
    max_count = max_count.max(5); // ensure some space

    // Prepare chart
    let width: usize = 1024;
    let height: usize = 720;
    register_font(
        "sans-serif",
        plotters::style::FontStyle::Normal,
        include_bytes!("../../assets/Roboto-Regular.ttf"),
    )
    .map_err(|_| eyre!("Failed to register font"))?;

    let mut buffer = vec![0; width * height * 3];
    {
        let root = BitMapBackend::with_buffer(&mut buffer, (width as u32, height as u32))
            .into_drawing_area();
        root.fill(&WHITE)?;

        let caption = if let Some(u) = filter_user {
            format!("Games played per day — {u} (last {span_days} days)")
        } else {
            format!("Games played per day (last {span_days} days)")
        };
        let mut ctx = ChartBuilder::on(&root)
            .set_label_area_size(LabelAreaPosition::Left, 70)
            .set_label_area_size(LabelAreaPosition::Bottom, 70)
            .caption(caption, ("sans-serif", 40))
            .margin(20)
            .build_cartesian_2d(
                start..today.succ_opt().unwrap_or(today),
                0f32..(max_count as f32 + 1.0),
            )?;

        let x_label_style = style::TextStyle::from(("sans-serif", 25).into_font());
        let y_label_style = style::TextStyle::from(("sans-serif", 30).into_font());

        // Generate month start dates for x-axis ticks
        let mut month_ticks = Vec::new();
        let mut tmp = NaiveDate::from_ymd_opt(start.year(), start.month(), 1).unwrap_or(start);
        while tmp <= today {
            month_ticks.push(tmp);
            let (y, m) = (tmp.year(), tmp.month());
            let (ny, nm) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
            if let Some(next) = NaiveDate::from_ymd_opt(ny, nm, 1) {
                tmp = next;
            } else {
                break;
            }
        }

        ctx.configure_mesh()
            .x_label_style(x_label_style)
            .y_label_style(y_label_style)
            .x_desc("Date")
            .y_desc("Games")
            .y_labels(10)
            .x_labels(0)
            .x_label_formatter(&|_| String::from(""))
            .y_label_formatter(&|v| format!("{}", v))
            .disable_mesh()
            .draw()?;

        // Draw custom x-axis ticks at month boundaries
        for &tick_date in &month_ticks {
            if tick_date >= start && tick_date <= today {
                ctx.draw_series(std::iter::once(PathElement::new(
                    vec![(tick_date, 0f32), (tick_date, max_count as f32 + 1.0)],
                    RGBColor(200, 200, 200).stroke_width(1),
                )))?;
            }
        }

        // Draw month labels using backend coordinates
        for &tick_date in &month_ticks {
            if tick_date >= start && tick_date <= today {
                // Calculate pixel position for the tick date
                let days_from_start = (tick_date - start).num_days() as f64;
                let total_days = ((today - start).num_days() + 1) as f64;
                let x_ratio = days_from_start / total_days;

                // Chart area bounds (approximate, accounting for margins and label areas)
                let chart_left = 90i32; // Left margin + y-label area
                let chart_right = (width as i32) - 20; // Right margin
                let chart_bottom = (height as i32) - 75; // Position for labels

                let x_pixel = chart_left + ((chart_right - chart_left) as f64 * x_ratio) as i32;

                root.draw(&Text::new(
                    tick_date.format("%Y-%m").to_string(),
                    (x_pixel, chart_bottom),
                    TextStyle::from(("sans-serif", 25)).pos(Pos::new(HPos::Center, VPos::Top)),
                ))?;
            }
        }
        // Calculate teammate frequency for filtered user or global player totals
        let top_players: Vec<String> = if let Some(_filtered_user) = filter_user {
            // When filtering, show teammates ordered by how often they played with the filtered user
            let mut teammate_counts: HashMap<String, u32> = HashMap::new();

            for participants in daily_participants.values() {
                for teammate in participants.iter() {
                    *teammate_counts.entry(teammate.clone()).or_insert(0) += 1;
                }
            }

            let mut teammate_totals: Vec<(String, u32)> = teammate_counts.into_iter().collect();
            teammate_totals.sort_by(|a, b| b.1.cmp(&a.1));

            teammate_totals
                .iter()
                .take(10)
                .map(|(name, _)| name.clone())
                .collect()
        } else {
            // Original logic for global view - top players by total games
            let mut player_totals: Vec<(String, u32)> = per_player_daily
                .iter()
                .map(|(username, daily_counts)| {
                    let total = daily_counts.values().sum::<u32>();
                    (username.clone(), total)
                })
                .collect();

            player_totals.sort_by(|a, b| b.1.cmp(&a.1));

            player_totals
                .iter()
                .take(10)
                .map(|(name, _)| name.clone())
                .collect()
        };

        let palette = colorous::TABLEAU10;
        let others_color = RGBColor(128, 128, 128);

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
                    if top_players.contains(participant) {
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

            // Sort players by their position in top_players list for consistent order
            players_that_day.sort_by(|a, b| {
                if a == "Others" {
                    std::cmp::Ordering::Greater // Others goes last
                } else if b == "Others" {
                    std::cmp::Ordering::Less
                } else {
                    let idx_a = top_players.iter().position(|p| p == a).unwrap_or(999);
                    let idx_b = top_players.iter().position(|p| p == b).unwrap_or(999);
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
                } else if let Some(color_idx) = top_players.iter().position(|p| p == &username) {
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

        // Draw legend for top players + others
        if !top_players.is_empty() {
            // Add top players to legend
            for (idx, player_name) in top_players.iter().enumerate() {
                let color = palette[idx % palette.len()];
                let rgb = RGBColor(color.r, color.g, color.b);

                ctx.draw_series(std::iter::once(Rectangle::new(
                    [(start, 0.), (start, 0.)],
                    rgb,
                )))?
                .label(player_name.clone())
                .legend(move |(x, y)| Rectangle::new([(x, y - 15), (x + 15, y + 5)], rgb.filled()));
            }

            // Add others if there are more players/teammates than can be shown
            let has_others = if filter_user.is_some() {
                // For filtered view, check if there are more than 10 unique teammates
                let total_teammates: HashSet<String> = daily_participants
                    .values()
                    .flat_map(|participants| participants.iter())
                    .cloned()
                    .collect();
                total_teammates.len() > 10
            } else {
                // For global view, check if there are more than 10 players
                per_player_daily.len() > 10
            };

            if has_others {
                ctx.draw_series(std::iter::once(Rectangle::new(
                    [(start, 0.), (start, 0.)],
                    others_color,
                )))?
                .label("Others".to_string())
                .legend(move |(x, y)| {
                    Rectangle::new([(x, y - 15), (x + 15, y + 5)], others_color.filled())
                });
            }

            ctx.configure_series_labels()
                .border_style(BLACK)
                .background_style(WHITE.mix(0.8))
                .position(plotters::chart::SeriesLabelPosition::UpperLeft)
                .label_font(("sans-serif", 20))
                .draw()?;
        }

        // Draw per-player line series (distinct color per player)
        if SHOW_PLAYER_LINES {
            let palette = colorous::TABLEAU10; // [Color; 10]
            for (idx, (username, daily)) in per_player_daily.iter().enumerate() {
                // Build ordered series (convert to i32)
                let series_pts = dates
                    .iter()
                    .map(|d| (*d, *daily.get(d).unwrap_or(&0u32) as f32))
                    .collect::<Vec<_>>();
                let color = palette[idx % palette.len()];
                let rgb = RGBColor(color.r, color.g, color.b);
                ctx.draw_series(LineSeries::new(series_pts.into_iter(), rgb))?
                    .label(username.clone())
                    .legend(move |(x, y)| PathElement::new(vec![(x, y), (x + 20, y)], rgb));
            }
            if !per_player_daily.is_empty() {
                ctx.configure_series_labels()
                    .border_style(BLACK)
                    .background_style(WHITE.mix(0.8))
                    .position(plotters::chart::SeriesLabelPosition::UpperLeft)
                    .label_font(("sans-serif", 20))
                    .draw()?;
            }
        }

        // (Using native ticks/labels now; manual month labels removed.)
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
