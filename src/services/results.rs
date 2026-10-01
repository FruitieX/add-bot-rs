use super::failure::DataFailure;
use chrono_tz::Tz;
use std::collections::{HashMap, HashSet, VecDeque};

use chrono::{DateTime, Datelike, Duration, NaiveDate, Utc};
use color_eyre::{eyre::eyre, Result};
use plotters::{
    coord::Shift,
    element::DashedPathElement,
    prelude::{BitMapBackend, DrawingArea, IntoDrawingArea, PathElement, Polygon, Rectangle},
    style::{text_anchor::HPos, Color, RGBColor},
};

use crate::{
    services::{
        chart_style::{BORDER, INK, MUTED, WIDTH},
        chart_text::layout_text,
        leetify::{
            collect_player_histories, get_leetify_games_checked, get_leetify_matches, LeetifyGame,
            LeetifyMatchHydration,
        },
    },
    settings::Settings,
    types::Username,
};

const DAYS_SHOWN: usize = 90;
const FORM_WINDOW: usize = 20;
const LEFT: i32 = 150;
const RIGHT: i32 = 1330;
const TOP: i32 = 390;
const BOTTOM: i32 = 810;
const TILE_TOP: i32 = 970;
const TILE_STEP: i32 = 17;
const WIN: RGBColor = RGBColor(52, 199, 89);
const LOSS: RGBColor = RGBColor(255, 59, 48);
const TIE: RGBColor = RGBColor(255, 214, 10);
const COLORS: [RGBColor; 3] = [WIN, LOSS, TIE];

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

fn result_index(result: &str) -> Option<usize> {
    match result {
        "win" => Some(0),
        "loss" => Some(1),
        "tie" => Some(2),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug)]
struct TrendPoint {
    time: DateTime<Utc>,
    rate: Option<f64>,
}

struct ResultsData {
    tz: Tz,
    exclusions: String,
    start: NaiveDate,
    end: NaiveDate,
    daily: Vec<Vec<usize>>,
    totals: [u32; 3],
    trend: Vec<TrendPoint>,
}

fn win_rate(counts: [u32; 3]) -> Option<f64> {
    let decided = counts[0] + counts[1];
    (decided > 0).then(|| 100.0 * counts[0] as f64 / decided as f64)
}

#[cfg(test)]
fn collect_results(games: Vec<LeetifyGame>, today: NaiveDate) -> ResultsData {
    collect_results_local(games, today, chrono_tz::UTC)
}

fn collect_results_local(mut games: Vec<LeetifyGame>, today: NaiveDate, tz: Tz) -> ResultsData {
    let start = today - Duration::days(DAYS_SHOWN as i64 - 1);
    let end = today + Duration::days(1);
    games.sort_by_key(|game| (game.game_finished_at, game_key(game)));
    let mut data = ResultsData {
        tz,
        exclusions: String::new(),
        start,
        end,
        daily: vec![Vec::new(); DAYS_SHOWN],
        totals: [0; 3],
        trend: Vec::new(),
    };
    let mut seen = HashSet::new();
    let mut window = VecDeque::new();
    let mut rolling_counts = [0; 3];
    for game in games {
        let Some(result) = result_index(&game.match_result.to_ascii_lowercase()) else {
            continue;
        };
        let date = game.game_finished_at.with_timezone(&tz).date_naive();
        if date >= end || !seen.insert(game_key(&game)) {
            continue;
        }
        window.push_back(result);
        rolling_counts[result] += 1;
        if window.len() > FORM_WINDOW {
            rolling_counts[window.pop_front().unwrap()] -= 1;
        }
        if date < start {
            continue;
        } // Earlier games prime only the rolling window.
        data.daily[(date - start).num_days() as usize].push(result);
        data.totals[result] += 1;
        if window.len() == FORM_WINDOW {
            let point = TrendPoint {
                // Centre daily observations over the matching column of result squares.
                time: date.and_hms_opt(12, 0, 0).unwrap().and_utc(),
                rate: win_rate(rolling_counts),
            };
            if let Some(last) = data
                .trend
                .last_mut()
                .filter(|last| last.time.date_naive() == date)
            {
                *last = point;
            } else {
                data.trend.push(point);
            }
        }
    }
    data
}

fn shared_ids(histories: &[(String, Vec<LeetifyGame>)]) -> HashSet<String> {
    let mut counts = HashMap::new();
    for (_, games) in histories {
        let ids = games.iter().map(game_key).collect::<HashSet<_>>();
        for id in ids {
            *counts.entry(id).or_insert(0) += 1;
        }
    }
    counts
        .into_iter()
        .filter_map(|(id, count)| (count > 1).then_some(id))
        .collect()
}

fn excluded_ids(
    histories: &[(String, Vec<LeetifyGame>)],
    hydration: &LeetifyMatchHydration,
    configured: &HashSet<crate::types::SteamID>,
) -> (HashSet<String>, HashSet<String>) {
    let shared = shared_ids(histories);
    let mut opposing = HashSet::new();
    let mut unverified = HashSet::new();
    for id in shared {
        let results = histories
            .iter()
            .flat_map(|(_, games)| games.iter())
            .filter(|game| game_key(game) == id)
            .filter_map(|game| result_index(&game.match_result.to_ascii_lowercase()))
            .collect::<HashSet<_>>();
        if results.len() > 1 {
            opposing.insert(id);
        } else if let Some(game) = hydration.matches.get(&id) {
            if game
                .teams
                .iter()
                .filter(|team| {
                    team.steam64_ids
                        .iter()
                        .any(|player| configured.contains(player))
                })
                .count()
                > 1
            {
                opposing.insert(id);
            }
        } else if results.contains(&2) {
            unverified.insert(id);
        }
    }
    (opposing, unverified)
}

pub async fn get_results_chart(
    settings: &Settings,
    filter_user: Option<&Username>,
    tz: Tz,
) -> Result<Vec<u8>> {
    let mut players = settings
        .players
        .steamid_mappings
        .iter()
        .filter(|(username, _)| filter_user.is_none_or(|user| user == *username))
        .collect::<Vec<_>>();
    // Shared games are counted once; retain a deterministic player's perspective.
    players.sort_by_key(|(username, _)| username.to_string());
    if players.is_empty() {
        return Err(match filter_user {
            Some(name) => DataFailure::Unlinked(name.clone()),
            None => DataFailure::NoPlayers,
        }
        .into());
    }
    let requests = players.into_iter().map(|(name, steam_id)| async move {
        (
            name.clone(),
            get_leetify_games_checked(settings, steam_id).await,
        )
    });
    let histories =
        collect_player_histories(futures::future::join_all(requests).await, filter_user)?;
    let today = Utc::now().with_timezone(&tz).date_naive();
    let shared = if filter_user.is_none() {
        // Opposing wins/losses are evident from the profile outcomes. Shared
        // ties need rosters because both teams have the same outcome. Hydrate
        // only those matches rather than fetching every shared match again.
        let shared = shared_ids(&histories.games);
        histories
            .games
            .iter()
            .flat_map(|(_, games)| games.iter())
            .filter(|game| result_index(&game.match_result.to_ascii_lowercase()) == Some(2))
            .filter_map(|game| game.id.clone())
            .filter(|id| shared.contains(id))
            .collect()
    } else {
        HashSet::new()
    };
    let hydration = get_leetify_matches(settings, shared).await;
    let configured = settings
        .players
        .steamid_mappings
        .values()
        .cloned()
        .collect::<HashSet<_>>();
    let (opposing, unverified) = excluded_ids(&histories.games, &hydration, &configured);
    let mut excluded = HashSet::new();
    let mut unknown = HashSet::new();
    let games = histories
        .games
        .iter()
        .flat_map(|(_, games)| games.iter())
        .filter(|game| {
            if filter_user.is_some() {
                return true;
            }
            let key = game_key(game);
            let day = game.game_finished_at.with_timezone(&tz).date_naive();
            let shown = day >= today - Duration::days(89) && day <= today;
            if opposing.contains(&key) {
                if shown {
                    excluded.insert(key);
                }
                return false;
            }
            if unverified.contains(&key) {
                if shown {
                    unknown.insert(key);
                }
                return false;
            }
            true
        })
        .cloned()
        .collect();
    let notice = (!histories.unavailable.is_empty()).then(|| {
        format!(
            "Partial history · unavailable for: {}",
            histories.unavailable.join(" · ")
        )
    });
    let mut data = collect_results_local(games, today, tz);
    let mut notes = Vec::new();
    if !excluded.is_empty() {
        notes.push(format!(
            "Excluded: {} {} with configured players on opposing teams",
            excluded.len(),
            if excluded.len() == 1 {
                "match"
            } else {
                "matches"
            }
        ));
    }
    if !unknown.is_empty() {
        notes.push(format!(
            "Omitted: {} shared {} with unavailable team details",
            unknown.len(),
            if unknown.len() == 1 {
                "match"
            } else {
                "matches"
            }
        ));
    }
    data.exclusions = notes.join(" · ");
    render_results_with_notice(&data, filter_user, notice.as_deref())
}

fn text(
    root: &DrawingArea<BitMapBackend<'_>, Shift>,
    value: &str,
    position: (i32, i32),
    size: u32,
    color: RGBColor,
    anchor: HPos,
) -> Result<()> {
    let bold = size >= 58
        || value == "WIN RATE"
        || value == "EVEN"
        || value.starts_with("MORE ")
        || value == "HOW YOUR FORM CHANGED"
        || value == "EVERY MATCH";
    let layout = layout_text(value, size, bold);
    let Some(bounds) = layout.bounds else {
        return Ok(());
    };
    let anchor_x = match anchor {
        HPos::Left => bounds.min.x,
        HPos::Center => (bounds.min.x + bounds.max.x) / 2.0,
        HPos::Right => bounds.max.x,
    };
    let offset_x = position.0 - anchor_x.round() as i32;
    let offset_y = position.1 - bounds.min.y as i32;
    // Plotters' ab_glyph backend omits px_bounds().min.x and truncates advances
    // before outlining. Rasterize correctly positioned glyphs directly instead.
    for glyph in layout.glyphs {
        let bounds = glyph.px_bounds();
        let mut failure = None;
        glyph.draw(|x, y, coverage| {
            if coverage > 0.0 && failure.is_none() {
                let pixel = (
                    offset_x + bounds.min.x as i32 + x as i32,
                    offset_y + bounds.min.y as i32 + y as i32,
                );
                failure = root.draw_pixel(pixel, &color.mix(coverage as f64)).err();
            }
        });
        if let Some(error) = failure {
            return Err(error.into());
        }
    }
    Ok(())
}

type FormCoordinate = (f64, f64);

// Split at the 50% crossing so the line and its fill always use the correct colour.
fn form_segments(a: FormCoordinate, b: FormCoordinate) -> Vec<(FormCoordinate, FormCoordinate)> {
    if (a.1 - 50.0) * (b.1 - 50.0) < 0.0 {
        let crossing = (a.0 + (50.0 - a.1) / (b.1 - a.1) * (b.0 - a.0), 50.0);
        vec![(a, crossing), (crossing, b)]
    } else {
        vec![(a, b)]
    }
}

#[cfg(test)]
fn render_results(data: &ResultsData, filter_user: Option<&Username>) -> Result<Vec<u8>> {
    render_results_with_notice(data, filter_user, None)
}

fn render_results_with_notice(
    data: &ResultsData,
    filter_user: Option<&Username>,
    notice: Option<&str>,
) -> Result<Vec<u8>> {
    let rows = data.daily.iter().map(Vec::len).max().unwrap_or(0).max(4);
    let legend_y = TILE_TOP + rows as i32 * TILE_STEP + 28;
    let height = (legend_y + 180) as u32;
    let mut buffer = vec![0; WIDTH as usize * height as usize * 3];
    {
        let root = BitMapBackend::with_buffer(&mut buffer, (WIDTH, height)).into_drawing_area();
        super::chart_style::frame(&root)?;
        text(&root, "Wins, losses & ties", (100, 86), 68, INK, HPos::Left)?;
        let user = filter_user
            .map(ToString::to_string)
            .unwrap_or_else(|| "Configured players".into());
        text(
            &root,
            &super::chart_style::fit_text(
                &format!(
                    "{user}  ·  {} – {}  ·  {DAYS_SHOWN} days · {}",
                    data.start.format("%d %b"),
                    (data.end - Duration::days(1)).format("%d %b %Y"),
                    data.tz
                ),
                40,
                1300,
            ),
            (100, 150),
            40,
            MUTED,
            HPos::Left,
        )?;
        if let Some(notice) = notice {
            text(
                &root,
                &super::chart_style::fit_text(notice, 36, 1300),
                (100, 180),
                36,
                MUTED,
                HPos::Left,
            )?;
        }
        let overall = win_rate(data.totals)
            .map(|rate| format!("{rate:.0}%"))
            .unwrap_or_else(|| "—".into());
        text(&root, &overall, (100, 210), 74, INK, HPos::Left)?;
        text(&root, "WIN RATE", (300, 222), 38, MUTED, HPos::Left)?;
        text(
            &root,
            "90 days · ties excluded",
            (300, 262),
            36,
            MUTED,
            HPos::Left,
        )?;
        for (index, (x, label)) in [(760, "wins"), (980, "losses"), (1200, "ties")]
            .into_iter()
            .enumerate()
        {
            root.draw(&Rectangle::new(
                [(x, 223), (x + 19, 242)],
                COLORS[index].filled(),
            ))?;
            text(
                &root,
                &data.totals[index].to_string(),
                (x + 35, 212),
                58,
                INK,
                HPos::Left,
            )?;
            text(&root, label, (x + 35, 257), 38, MUTED, HPos::Left)?;
        }
        text(
            &root,
            "HOW YOUR FORM CHANGED",
            (LEFT, 303),
            40,
            INK,
            HPos::Left,
        )?;
        text(
            &root,
            "20-match win rate · after each played day · ties excluded",
            (LEFT, 348),
            36,
            MUTED,
            HPos::Left,
        )?;

        let start_seconds = data
            .start
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp() as f64;
        let x_pixel = |seconds: f64| {
            LEFT + ((seconds - start_seconds) / (DAYS_SHOWN as f64 * 86_400.0)
                * (RIGHT - LEFT) as f64)
                .round() as i32
        };
        let y_pixel = |rate: f64| BOTTOM - (rate / 100.0 * (BOTTOM - TOP) as f64).round() as i32;
        let even_y = y_pixel(50.0);
        root.draw(&Rectangle::new(
            [(LEFT, TOP), (RIGHT, even_y)],
            WIN.mix(0.035).filled(),
        ))?;
        root.draw(&Rectangle::new(
            [(LEFT, even_y), (RIGHT, BOTTOM)],
            LOSS.mix(0.035).filled(),
        ))?;
        for rate in [0.0, 25.0, 50.0, 75.0, 100.0] {
            let y = y_pixel(rate);
            if rate != 50.0 {
                root.draw(&PathElement::new(
                    vec![(LEFT, y), (RIGHT, y)],
                    BORDER.stroke_width(1),
                ))?;
            }
            text(
                &root,
                &format!("{rate:.0}%"),
                (LEFT - 20, y - 12),
                38,
                MUTED,
                HPos::Right,
            )?;
        }
        let mut inactive_gaps = false;
        let mut runs: Vec<(RGBColor, Vec<FormCoordinate>)> = Vec::new();
        let mut join_run = false;
        for pair in data.trend.windows(2) {
            let (Some(rate_a), Some(rate_b)) = (pair[0].rate, pair[1].rate) else {
                join_run = false;
                continue;
            };
            let a = (pair[0].time.timestamp() as f64, rate_a);
            let b = (pair[1].time.timestamp() as f64, rate_b);
            if pair[1].time - pair[0].time > Duration::days(3) {
                inactive_gaps = true;
                join_run = false;
                root.draw(&DashedPathElement::new(
                    vec![(x_pixel(a.0), y_pixel(a.1)), (x_pixel(b.0), y_pixel(b.1))],
                    4,
                    6,
                    MUTED.mix(0.6).stroke_width(2),
                ))?;
                continue;
            }
            for (a, b) in form_segments(a, b) {
                let color = if (a.1 + b.1) / 2.0 >= 50.0 { WIN } else { LOSS };
                if let Some((last_color, points)) = runs.last_mut() {
                    if join_run && *last_color == color && points.last() == Some(&a) {
                        points.push(b);
                        continue;
                    }
                }
                runs.push((color, vec![a, b]));
                join_run = true;
            }
        }
        // Fill each continuous region once to avoid dark seams between adjacent polygons.
        for (color, points) in runs {
            let path = points
                .iter()
                .map(|p| (x_pixel(p.0), y_pixel(p.1)))
                .collect::<Vec<_>>();
            let mut area = vec![(path[0].0, even_y)];
            area.extend_from_slice(&path);
            area.push((path.last().unwrap().0, even_y));
            root.draw(&Polygon::new(area, color.mix(0.16).filled()))?;
            root.draw(&PathElement::new(path, color.stroke_width(4)))?;
        }
        root.draw(&DashedPathElement::new(
            vec![(LEFT, even_y), (RIGHT, even_y)],
            7,
            7,
            MUTED.mix(0.75).stroke_width(2),
        ))?;
        // The daily line has no scatter markers. Preserve a short stroke for a
        // standalone daily observation (including observations between tie-only gaps).
        for daily_run in data.trend.split(|point| point.rate.is_none()) {
            if let [TrendPoint {
                time,
                rate: Some(rate),
            }] = daily_run
            {
                let color = if *rate >= 50.0 { WIN } else { LOSS };
                let x = x_pixel(time.timestamp() as f64);
                root.draw(&PathElement::new(
                    vec![(x - 4, y_pixel(*rate)), (x + 4, y_pixel(*rate))],
                    color.stroke_width(4),
                ))?;
            }
        }
        text(
            &root,
            "MORE WINS THAN LOSSES",
            (LEFT + 18, TOP + 14),
            36,
            RGBColor(39, 157, 72),
            HPos::Left,
        )?;
        text(
            &root,
            "MORE LOSSES THAN WINS",
            (LEFT + 18, BOTTOM - 36),
            36,
            RGBColor(216, 57, 49),
            HPos::Left,
        )?;
        text(
            &root,
            "EVEN",
            (RIGHT + 16, even_y - 10),
            36,
            MUTED,
            HPos::Left,
        )?;
        if !data.trend.iter().any(|point| point.rate.is_some()) {
            let message = if data.totals.iter().sum::<u32>() == 0 {
                "No matches in this 90-day period"
            } else if data.totals[0] + data.totals[1] == 0 {
                "No decided matches yet — ties are shown below"
            } else {
                "20 matches needed for the trend — your results are shown below"
            };
            text(
                &root,
                message,
                ((LEFT + RIGHT) / 2, even_y + 40),
                40,
                MUTED,
                HPos::Center,
            )?;
        }
        let mut ticks = vec![data.start];
        for offset in 10..DAYS_SHOWN - 10 {
            let date = data.start + Duration::days(offset as i64);
            if date.day() == 1 || date.day() == 15 {
                ticks.push(date);
            }
        }
        ticks.push(data.end - Duration::days(1));
        for date in ticks {
            let x = x_pixel(date.and_hms_opt(12, 0, 0).unwrap().and_utc().timestamp() as f64);
            text(
                &root,
                &date.format("%d %b").to_string(),
                (x, BOTTOM + 23),
                38,
                MUTED,
                HPos::Center,
            )?;
        }
        text(&root, "EVERY MATCH", (LEFT, 901), 40, INK, HPos::Left)?;
        text(
            &root,
            "One square per match · blank days = no recorded matches",
            (LEFT, 935),
            36,
            MUTED,
            HPos::Left,
        )?;
        let day_width = (RIGHT - LEFT) as f64 / DAYS_SHOWN as f64;
        for (day, results) in data.daily.iter().enumerate() {
            let x = LEFT + (day as f64 * day_width).round() as i32 + 1;
            let date = data.start + Duration::days(day as i64);
            if date.day() == 1 {
                root.draw(&PathElement::new(
                    vec![
                        (x - 1, TILE_TOP),
                        (x - 1, TILE_TOP + rows as i32 * TILE_STEP),
                    ],
                    BORDER.stroke_width(1),
                ))?;
            }
            for (row, &result) in results.iter().enumerate() {
                let y = TILE_TOP + row as i32 * TILE_STEP;
                root.draw(&Rectangle::new(
                    [(x, y), (x + 11, y + 11)],
                    COLORS[result].filled(),
                ))?;
            }
        }
        for (index, (x, label)) in [(LEFT, "Win"), (LEFT + 150, "Loss"), (LEFT + 320, "Tie")]
            .into_iter()
            .enumerate()
        {
            root.draw(&Rectangle::new(
                [(x, legend_y), (x + 14, legend_y + 14)],
                COLORS[index].filled(),
            ))?;
            text(&root, label, (x + 25, legend_y - 2), 36, MUTED, HPos::Left)?;
        }
        let note = if inactive_gaps {
            "Dotted = no matches for >3 days"
        } else {
            "Daily matches: top to bottom"
        };
        text(&root, note, (RIGHT, legend_y), 36, MUTED, HPos::Right)?;
        text(
            &root,
            if filter_user.is_some() {
                "Available Leetify history; older matches may be missing."
            } else {
                "Shared matches counted once · Older Leetify matches may be missing."
            },
            (LEFT, legend_y + 45),
            36,
            MUTED,
            HPos::Left,
        )?;
        if !data.exclusions.is_empty() {
            text(
                &root,
                &super::chart_style::fit_text(&data.exclusions, 36, (RIGHT - LEFT) as u32),
                (LEFT, legend_y + 90),
                36,
                MUTED,
                HPos::Left,
            )?;
        }
        root.present()?;
    }
    let image = image::RgbImage::from_raw(WIDTH, height, buffer)
        .ok_or_else(|| eyre!("Image buffer not large enough"))?;
    let mut bytes = Vec::new();
    image.write_to(
        &mut std::io::Cursor::new(&mut bytes),
        image::ImageFormat::Png,
    )?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::chart_text::REGULAR_FONT;
    use ab_glyph::{Font, ScaleFont};

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 9, 30).unwrap()
    }

    fn game(id: usize, date: NaiveDate, hour: u32, result: &str) -> LeetifyGame {
        serde_json::from_value(serde_json::json!({
            "id": format!("match-{id}"),
            "ownTeamSteam64Ids": [],
            "gameFinishedAt": date.and_hms_opt(hour, 0, 0).unwrap().and_utc(),
            "mapName": "de_mirage",
            "matchResult": result,
            "scores": [13, 10],
            "skillLevel": null
        }))
        .unwrap()
    }

    #[test]
    fn results_group_matches_using_the_selected_local_calendar() {
        let mut midnight = game(1, today() - Duration::days(1), 22, "win");
        let local =
            collect_results_local(vec![midnight.clone()], today(), chrono_tz::Europe::Helsinki);
        let utc = collect_results(vec![midnight.clone()], today());
        assert_eq!(local.daily[89], vec![0]);
        assert_eq!(utc.daily[88], vec![0]);
        midnight.game_finished_at = (today() - Duration::days(90))
            .and_hms_opt(22, 30, 0)
            .unwrap()
            .and_utc();
        assert_eq!(
            collect_results_local(vec![midnight.clone()], today(), chrono_tz::Europe::Helsinki)
                .totals,
            [1, 0, 0]
        );
        assert_eq!(collect_results(vec![midnight], today()).totals, [0, 0, 0]);
        let dst = NaiveDate::from_ymd_opt(2026, 10, 25).unwrap();
        let first = game(2, dst, 0, "win");
        let second = game(3, dst, 1, "loss");
        assert_eq!(
            collect_results_local(vec![first, second], dst, chrono_tz::Europe::Helsinki).daily[89],
            vec![0, 1]
        );
    }

    #[test]
    fn global_results_exclude_opponents_even_when_the_shared_match_is_tied() {
        use crate::services::leetify::{LeetifyMatch, LeetifyMatchTeam};
        use crate::types::SteamID;
        let alice = SteamID::new("alice".into());
        let bob = SteamID::new("bob".into());
        let configured = HashSet::from([alice.clone(), bob.clone()]);
        let win = game(1, today(), 12, "win");
        let loss = game(1, today(), 12, "loss");
        let tie = game(2, today(), 13, "tie");
        let shared_win = game(3, today(), 14, "win");
        let unknown = game(4, today(), 15, "tie");
        let histories = vec![
            (
                "Alice".into(),
                vec![
                    win.clone(),
                    tie.clone(),
                    shared_win.clone(),
                    unknown.clone(),
                ],
            ),
            (
                "Bob".into(),
                vec![loss, tie.clone(), shared_win.clone(), unknown],
            ),
        ];
        let make_match = |id: usize, opposing: bool| LeetifyMatch {
            id: format!("match-{id}"),
            game_finished_at: win.game_finished_at,
            map_name: "de_nuke".into(),
            teams: vec![
                LeetifyMatchTeam {
                    steam64_ids: if opposing {
                        vec![alice.clone()]
                    } else {
                        vec![alice.clone(), bob.clone()]
                    },
                    score: 12,
                },
                LeetifyMatchTeam {
                    steam64_ids: if opposing {
                        vec![bob.clone()]
                    } else {
                        vec![SteamID::new("outside".into())]
                    },
                    score: 12,
                },
            ],
        };
        let hydration = LeetifyMatchHydration {
            matches: HashMap::from([
                ("match-2".into(), make_match(2, true)),
                ("match-3".into(), make_match(3, false)),
            ]),
            failed_match_ids: HashSet::from(["match-1".into(), "match-4".into()]),
        };
        let (opponents, unknown) = excluded_ids(&histories, &hydration, &configured);
        assert_eq!(
            opponents,
            HashSet::from(["match-1".into(), "match-2".into()])
        );
        assert_eq!(unknown, HashSet::from(["match-4".into()]));
        let selected = histories
            .iter()
            .flat_map(|(_, games)| games.iter())
            .filter(|g| !opponents.contains(&game_key(g)) && !unknown.contains(&game_key(g)))
            .cloned()
            .collect();
        assert_eq!(collect_results(selected, today()).totals, [1, 0, 0]);
        // The same opposing matches are valid results in an individual view.
        assert_eq!(
            collect_results(histories[0].1.clone(), today()).totals,
            [2, 0, 2]
        );
    }

    #[test]
    fn totals_and_strip_use_exactly_ninety_days_and_chronological_unique_results() {
        let start = today() - Duration::days(89);
        let win = game(1, start, 16, "WIN");
        let mut fallback = game(2, start, 18, "loss");
        fallback.id = None;
        let games = vec![
            game(3, today(), 18, "tie"),
            fallback.clone(),
            game(4, start - Duration::days(1), 18, "win"),
            game(5, today() + Duration::days(1), 18, "loss"),
            game(6, today(), 19, "unknown"),
            win.clone(),
            win,
            fallback,
        ];
        let data = collect_results(games, today());
        assert_eq!(data.daily.len(), 90);
        assert_eq!(data.totals, [1, 1, 1]);
        assert_eq!(data.daily[0], vec![0, 1]);
        assert_eq!(data.daily[89], vec![2]);
        assert_eq!(win_rate(data.totals), Some(50.0));
    }

    #[test]
    fn rolling_window_is_twenty_matches_with_ties_excluded_only_from_percentage() {
        let date = today() - Duration::days(1);
        let games = (0..21)
            .rev()
            .map(|i| {
                let result = if i < 10 {
                    "win"
                } else if i < 15 {
                    "loss"
                } else {
                    "tie"
                };
                game(i, if i == 20 { today() } else { date }, i as u32, result)
            })
            .collect();
        let data = collect_results(games, today());
        assert_eq!(data.trend.len(), 2);
        assert!((data.trend[0].rate.unwrap() - 100.0 * 10.0 / 15.0).abs() < 1e-8);
        assert!((data.trend[1].rate.unwrap() - 100.0 * 9.0 / 14.0).abs() < 1e-8);
        assert_eq!(data.totals, [10, 5, 6]);
    }

    #[test]
    fn daily_trend_keeps_only_the_final_window_but_preserves_every_match() {
        let date = today() - Duration::days(1);
        let mut games = (0..19)
            .map(|i| game(i, date - Duration::days(1), i as u32, "win"))
            .collect::<Vec<_>>();
        games.extend([
            game(19, date, 9, "loss"),
            game(20, date, 10, "win"),
            game(21, date, 23, "loss"),
            game(22, today(), 10, "loss"),
        ]);
        games.reverse(); // API ordering must not decide which daily observation survives.
        let data = collect_results(games, today());
        assert_eq!(data.trend.len(), 2);
        assert_eq!(
            data.trend[0].time,
            date.and_hms_opt(12, 0, 0).unwrap().and_utc()
        );
        assert_eq!(data.trend[0].rate, Some(90.0));
        assert_eq!(data.trend[1].rate, Some(85.0));
        assert_eq!(data.daily[88], vec![1, 0, 1]);
        assert_eq!(data.daily[89], vec![1]);
        assert_eq!(data.totals, [20, 3, 0]);
    }

    #[test]
    fn pre_window_history_primes_the_trend_without_inflating_totals() {
        let start = today() - Duration::days(89);
        let mut games = (0..19)
            .map(|i| game(i, start - Duration::days(1), i as u32, "win"))
            .collect::<Vec<_>>();
        games.push(game(19, start, 1, "loss"));
        let data = collect_results(games, today());
        assert_eq!(data.totals, [0, 1, 0]);
        assert_eq!(data.trend.len(), 1);
        assert_eq!(data.trend[0].rate, Some(95.0));
    }

    #[test]
    fn no_percentage_for_tie_only_windows_and_no_trend_before_twenty_matches() {
        let short = collect_results(vec![game(1, today(), 12, "win")], today());
        assert!(short.trend.is_empty());
        let ties = collect_results(
            (0..20).map(|i| game(i, today(), i as u32, "tie")).collect(),
            today(),
        );
        assert_eq!(ties.trend.len(), 1);
        assert_eq!(ties.trend[0].rate, None);
        assert_eq!(win_rate(ties.totals), None);
    }

    #[test]
    fn fill_segments_split_precisely_at_even_with_both_directions() {
        for (a, b) in [((0.0, 80.0), (10.0, 20.0)), ((0.0, 20.0), (10.0, 80.0))] {
            let parts = form_segments(a, b);
            assert_eq!(parts, vec![(a, (5.0, 50.0)), ((5.0, 50.0), b)]);
        }
        assert_eq!(form_segments((0.0, 50.0), (10.0, 50.0)).len(), 1);
    }

    #[test]
    fn text_layout_preserves_fractional_advances_and_glyph_bearings() {
        let layout = layout_text("jAV", 31, false);
        let scaled = REGULAR_FONT.as_scaled(31.0);
        let j = scaled.glyph_id('j');
        let a = scaled.glyph_id('A');
        let v = scaled.glyph_id('V');
        let a_x = scaled.h_advance(j) + scaled.kern(j, a);
        let v_x = a_x + scaled.h_advance(a) + scaled.kern(a, v);
        assert_eq!(layout.glyphs[1].glyph().position.x, a_x);
        assert_eq!(layout.glyphs[2].glyph().position.x, v_x);
        assert_ne!(a_x.fract(), 0.0);
        // j has an ink bearing left of its advance origin; the old renderer lost it.
        assert!(layout.glyphs[0].px_bounds().min.x < 0.0);
        assert!(layout.bounds.unwrap().min.x < 0.0);
    }

    #[test]
    fn text_rasterization_respects_ink_alignment_with_descenders_and_spaces() {
        for anchor in [HPos::Left, HPos::Center, HPos::Right] {
            let mut buffer = vec![255; 320 * 80 * 3];
            {
                let root = BitMapBackend::with_buffer(&mut buffer, (320, 80)).into_drawing_area();
                text(&root, "jAV  fj", (160, 12), 31, INK, anchor).unwrap();
            }
            let image = image::RgbImage::from_raw(320, 80, buffer).unwrap();
            let pixels = image
                .enumerate_pixels()
                .filter(|(_, _, pixel)| pixel.0 != [255; 3])
                .map(|(x, y, _)| (x as i32, y as i32))
                .collect::<Vec<_>>();
            let min_x = pixels.iter().map(|p| p.0).min().unwrap();
            let max_x = pixels.iter().map(|p| p.0).max().unwrap();
            assert!(pixels.iter().map(|p| p.1).min().unwrap() >= 12);
            match anchor {
                HPos::Left => assert!((min_x - 160).abs() <= 1),
                HPos::Center => assert!(((min_x + max_x) / 2 - 160).abs() <= 1),
                HPos::Right => assert!((max_x - 159).abs() <= 1),
            }
        }
    }

    fn sparse_history() -> ResultsData {
        let start = today() - Duration::days(89);
        let mut games = (0..19)
            .map(|i| {
                game(
                    i,
                    start - Duration::days(1),
                    i as u32,
                    if i < 7 { "win" } else { "loss" },
                )
            })
            .collect::<Vec<_>>();
        for (i, day) in [
            7, 8, 32, 39, 40, 50, 52, 54, 55, 57, 58, 59, 60, 61, 65, 70, 71, 74, 76, 85, 86, 87,
        ]
        .into_iter()
        .enumerate()
        {
            games.push(game(
                19 + i,
                start + Duration::days(day),
                18,
                if i % 3 == 0 { "loss" } else { "win" },
            ));
        }
        collect_results(games, today())
    }

    #[test]
    fn isolated_matches_between_inactive_gaps_do_not_gain_coloured_scatter_dots() {
        let data = sparse_history();
        let point = data.trend[2]; // Single match at day 32, between two long gaps.
        assert!(point.time - data.trend[1].time > Duration::days(3));
        assert!(data.trend[3].time - point.time > Duration::days(3));
        let image = image::load_from_memory(&render_results(&data, None).unwrap())
            .unwrap()
            .to_rgb8();
        let seconds =
            (point.time - data.start.and_hms_opt(0, 0, 0).unwrap().and_utc()).num_seconds();
        let x = LEFT + (seconds as f64 / (90.0 * 86400.0) * (RIGHT - LEFT) as f64).round() as i32;
        let y = BOTTOM - (point.rate.unwrap() / 100.0 * (BOTTOM - TOP) as f64).round() as i32;
        for x in x - 2..=x + 2 {
            for y in y - 2..=y + 2 {
                let pixel = image.get_pixel(x as u32, y as u32).0;
                assert_ne!(pixel, [LOSS.0, LOSS.1, LOSS.2]);
                assert_ne!(pixel, [WIN.0, WIN.1, WIN.2]);
            }
        }
        assert_eq!(data.daily.iter().map(Vec::len).sum::<usize>(), 22);
    }

    #[test]
    fn rendering_handles_empty_short_tie_only_and_busy_days_without_clipping_tiles() {
        let cases = [
            vec![],
            vec![game(1, today(), 12, "win")],
            (0..20)
                .map(|i| game(i, today(), (i % 24) as u32, "tie"))
                .collect(),
        ];
        for games in cases {
            let data = collect_results(games, today());
            let bytes = render_results(&data, Some(&Username::new("@example".into()))).unwrap();
            let image = image::load_from_memory(&bytes).unwrap().to_rgb8();
            assert_eq!(image.width(), WIDTH);
            assert!(image.height() > (TILE_TOP + data.daily[89].len() as i32 * TILE_STEP) as u32);
            for (row, &result) in data.daily[89].iter().enumerate() {
                let x = LEFT + (89.0 * (RIGHT - LEFT) as f64 / 90.0).round() as i32 + 5;
                let y = TILE_TOP + row as i32 * TILE_STEP + 5;
                let color = COLORS[result];
                assert_eq!(
                    image.get_pixel(x as u32, y as u32).0,
                    [color.0, color.1, color.2]
                );
            }
        }
    }

    #[test]
    #[ignore = "writes a sample chart for manual visual review"]
    fn write_results_preview() {
        let start = today() - Duration::days(89);
        let mut games = Vec::new();
        let mut random = 16_u64;
        for day in 0..90 {
            random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
            let count = ((random >> 32) % 5) as u32;
            for i in 0..count {
                random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
                let value = (random >> 32) % 100;
                let chance = if day < 25 {
                    72
                } else if day < 56 {
                    29
                } else {
                    79
                };
                let result = if value < 6 {
                    "tie"
                } else if value < chance {
                    "win"
                } else {
                    "loss"
                };
                games.push(game(
                    games.len(),
                    start + Duration::days(day),
                    15 + i * 2,
                    result,
                ));
            }
        }
        std::fs::create_dir_all("target").unwrap();
        std::fs::write(
            "target/results-preview.png",
            render_results(&collect_results(games.clone(), today()), None).unwrap(),
        )
        .unwrap();
        std::fs::write(
            "target/results-partial-preview.png",
            render_results_with_notice(
                &collect_results(games.clone(), today()),
                None,
                Some("Partial history · unavailable for: Alice · Bob"),
            )
            .unwrap(),
        )
        .unwrap();
        let mut local = collect_results_local(games, today(), chrono_tz::Europe::Helsinki);
        local.exclusions = "Excluded: 2 matches with configured players on opposing teams · Omitted: 1 shared match with unavailable team details".into();
        std::fs::write(
            "target/results-local-exclusions-preview.png",
            render_results_with_notice(
                &local,
                None,
                Some("Partial history · unavailable for: Alice · Bob"),
            )
            .unwrap(),
        )
        .unwrap();
        std::fs::write(
            "target/results-sparse-preview.png",
            render_results(
                &sparse_history(),
                Some(&Username::new("Sample sparse history".into())),
            )
            .unwrap(),
        )
        .unwrap();
    }
}
