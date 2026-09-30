use std::collections::{HashSet, VecDeque};

use chrono::{DateTime, Datelike, Duration, NaiveDate, Utc};
use color_eyre::{eyre::eyre, Result};
use plotters::{
    coord::Shift,
    element::DashedPathElement,
    prelude::{
        BitMapBackend, Circle, DrawingArea, IntoDrawingArea, PathElement, Polygon, Rectangle, Text,
    },
    style::{
        self, register_font,
        text_anchor::{HPos, Pos, VPos},
        Color, IntoFont, RGBColor, TextStyle, WHITE,
    },
};

use crate::{
    services::leetify::{get_leetify_games, LeetifyGame},
    settings::Settings,
    types::Username,
};

const DAYS_SHOWN: usize = 90;
const FORM_WINDOW: usize = 20;
const WIDTH: u32 = 1500;
const LEFT: i32 = 128;
const RIGHT: i32 = 1370;
const TOP: i32 = 370;
const BOTTOM: i32 = 810;
const TILE_TOP: i32 = 970;
const TILE_STEP: i32 = 17;
const WIN: RGBColor = RGBColor(52, 199, 89);
const LOSS: RGBColor = RGBColor(255, 59, 48);
const TIE: RGBColor = RGBColor(255, 214, 10);
const INK: RGBColor = RGBColor(25, 43, 54);
const MUTED: RGBColor = RGBColor(101, 118, 129);
const BORDER: RGBColor = RGBColor(229, 234, 240);
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

fn collect_results(mut games: Vec<LeetifyGame>, today: NaiveDate) -> ResultsData {
    let start = today - Duration::days(DAYS_SHOWN as i64 - 1);
    let end = today + Duration::days(1);
    games.sort_by_key(|game| (game.game_finished_at, game_key(game)));
    let mut data = ResultsData {
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
        let date = game.game_finished_at.date_naive();
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
            data.trend.push(TrendPoint {
                time: game.game_finished_at,
                rate: win_rate(rolling_counts),
            });
        }
    }
    data
}

pub async fn get_results_chart(
    settings: &Settings,
    filter_user: Option<&Username>,
) -> Result<Vec<u8>> {
    let mut players = settings
        .players
        .steamid_mappings
        .iter()
        .filter(|(username, _)| filter_user.is_none_or(|user| user == *username))
        .collect::<Vec<_>>();
    // Shared games are counted once; retain a deterministic player's perspective.
    players.sort_by_key(|(username, _)| username.to_string());
    let requests = players
        .into_iter()
        .map(|(_, steam_id)| get_leetify_games(settings, steam_id));
    let games = futures::future::join_all(requests)
        .await
        .into_iter()
        .flatten()
        .flatten()
        .collect::<Vec<_>>();
    if games.is_empty() {
        return Err(eyre!("No games found for the selected players"));
    }
    render_results(
        &collect_results(games, Utc::now().date_naive()),
        filter_user,
    )
}

fn text(
    root: &DrawingArea<BitMapBackend<'_>, Shift>,
    value: &str,
    position: (i32, i32),
    size: u32,
    color: RGBColor,
    anchor: HPos,
) -> Result<()> {
    let font_style =
        if size >= 23 || value == "WIN RATE" || value == "EVEN" || value.starts_with("MORE ") {
            style::FontStyle::Bold
        } else {
            style::FontStyle::Normal
        };
    root.draw(&Text::new(
        value,
        position,
        TextStyle::from(("sans-serif", size, font_style).into_font())
            .color(&color)
            .pos(Pos::new(anchor, VPos::Top)),
    ))?;
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

fn render_results(data: &ResultsData, filter_user: Option<&Username>) -> Result<Vec<u8>> {
    let rows = data.daily.iter().map(Vec::len).max().unwrap_or(0).max(4);
    let legend_y = TILE_TOP + rows as i32 * TILE_STEP + 28;
    let height = (legend_y + 90) as u32;
    register_font(
        "sans-serif",
        style::FontStyle::Normal,
        include_bytes!("../../assets/Roboto-Regular.ttf"),
    )
    .map_err(|_| eyre!("Failed to register font"))?;
    register_font(
        "sans-serif",
        style::FontStyle::Bold,
        include_bytes!("../../assets/Roboto-Bold.ttf"),
    )
    .map_err(|_| eyre!("Failed to register bold font"))?;
    let mut buffer = vec![0; WIDTH as usize * height as usize * 3];
    {
        let root = BitMapBackend::with_buffer(&mut buffer, (WIDTH, height)).into_drawing_area();
        root.fill(&RGBColor(243, 246, 249))?;
        let bottom = height as i32 - 32;
        root.draw(&Rectangle::new([(54, 30), (1446, bottom)], WHITE.filled()))?;
        root.draw(&Rectangle::new(
            [(30, 54), (1470, bottom - 24)],
            WHITE.filled(),
        ))?;
        for center in [(54, 54), (1446, 54), (54, bottom - 24), (1446, bottom - 24)] {
            root.draw(&Circle::new(center, 24, WHITE.filled()))?;
        }
        text(&root, "Wins, losses & ties", (100, 86), 46, INK, HPos::Left)?;
        let user = filter_user
            .map(ToString::to_string)
            .unwrap_or_else(|| "Configured players".into());
        text(
            &root,
            &format!(
                "{user}  ·  {} – {}  ·  {DAYS_SHOWN} days",
                data.start.format("%d %b %Y"),
                (data.end - Duration::days(1)).format("%d %b %Y")
            ),
            (100, 150),
            22,
            MUTED,
            HPos::Left,
        )?;
        let overall = win_rate(data.totals)
            .map(|rate| format!("{rate:.0}%"))
            .unwrap_or_else(|| "—".into());
        text(&root, &overall, (100, 210), 52, INK, HPos::Left)?;
        text(&root, "WIN RATE", (260, 222), 20, MUTED, HPos::Left)?;
        text(
            &root,
            "90-day period · ties excluded",
            (260, 253),
            18,
            MUTED,
            HPos::Left,
        )?;
        for (index, (x, label)) in [(690, "wins"), (915, "losses"), (1155, "ties")]
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
                38,
                INK,
                HPos::Left,
            )?;
            text(&root, label, (x + 35, 257), 20, MUTED, HPos::Left)?;
        }
        text(
            &root,
            "HOW YOUR FORM CHANGED",
            (LEFT, 309),
            23,
            INK,
            HPos::Left,
        )?;
        text(
            &root,
            "Win rate over your last 20 matches · ties excluded from the percentage",
            (LEFT, 341),
            19,
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
                20,
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
        // Isolated observations remain visible, including a single complete window.
        for point in &data.trend {
            if let Some(rate) = point.rate {
                let color = if rate >= 50.0 { WIN } else { LOSS };
                root.draw(&Circle::new(
                    (x_pixel(point.time.timestamp() as f64), y_pixel(rate)),
                    2,
                    color.filled(),
                ))?;
            }
        }
        text(
            &root,
            "MORE WINS THAN LOSSES",
            (LEFT + 18, TOP + 14),
            18,
            RGBColor(39, 157, 72),
            HPos::Left,
        )?;
        text(
            &root,
            "MORE LOSSES THAN WINS",
            (LEFT + 18, BOTTOM - 36),
            18,
            RGBColor(216, 57, 49),
            HPos::Left,
        )?;
        text(
            &root,
            "EVEN",
            (RIGHT + 16, even_y - 10),
            17,
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
                23,
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
                20,
                MUTED,
                HPos::Center,
            )?;
        }
        text(&root, "EVERY MATCH", (LEFT, 901), 23, INK, HPos::Left)?;
        text(
            &root,
            "Same dates as above · one square per match · blank days mean no games",
            (LEFT, 935),
            19,
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
        for (index, (x, label)) in [(LEFT, "Win"), (LEFT + 130, "Loss"), (LEFT + 260, "Tie")]
            .into_iter()
            .enumerate()
        {
            root.draw(&Rectangle::new(
                [(x, legend_y), (x + 14, legend_y + 14)],
                COLORS[index].filled(),
            ))?;
            text(&root, label, (x + 25, legend_y - 2), 19, MUTED, HPos::Left)?;
        }
        let note = if inactive_gaps {
            "Dotted spans = no matches for over 3 days"
        } else {
            "Matches within each day run top to bottom"
        };
        text(&root, note, (RIGHT, legend_y), 17, MUTED, HPos::Right)?;
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
                game(i, date, i as u32, result)
            })
            .collect();
        let data = collect_results(games, today());
        assert_eq!(data.trend.len(), 2);
        assert!((data.trend[0].rate.unwrap() - 100.0 * 10.0 / 15.0).abs() < 1e-8);
        assert!((data.trend[1].rate.unwrap() - 100.0 * 9.0 / 14.0).abs() < 1e-8);
        assert_eq!(data.totals, [10, 5, 6]);
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
            render_results(&collect_results(games, today()), None).unwrap(),
        )
        .unwrap();
    }
}
