use std::collections::{HashMap, HashSet};

use chrono::{Datelike, Duration, NaiveDate, Utc};
use color_eyre::{eyre::eyre, Result};
use plotters::{
    chart::{ChartBuilder, LabelAreaPosition},
    prelude::{BitMapBackend, IntoDrawingArea, PathElement, Rectangle, Text},
    style::{
        self, register_font,
        text_anchor::{HPos, Pos, VPos},
        Color, IntoFont, RGBColor, TextStyle, BLACK, WHITE,
    },
};

use crate::{
    services::leetify::{get_leetify_games, LeetifyGame},
    settings::Settings,
    types::Username,
};

const DAYS_SHOWN: i64 = 90;

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

pub async fn get_results_chart(
    settings: &Settings,
    filter_user: Option<&Username>,
) -> Result<Vec<u8>> {
    let requests =
        settings
            .players
            .steamid_mappings
            .clone()
            .into_iter()
            .map(|(username, steam_id)| {
                let settings = settings.clone();
                async move { (username, get_leetify_games(&settings, &steam_id).await) }
            });
    let player_results = futures::future::join_all(requests).await;

    let mut games = Vec::new();
    for (username, maybe_games) in player_results {
        let include = filter_user.map(|user| *user == username).unwrap_or(true);
        if include {
            if let Some(player_games) = maybe_games {
                games.extend(player_games);
            }
        }
    }
    if games.is_empty() {
        return Err(eyre!("No games found for any configured player"));
    }

    let today = Utc::now().date_naive();
    let start = today - Duration::days(DAYS_SHOWN);
    let mut counts: HashMap<NaiveDate, [u32; 3]> = HashMap::new();
    let mut seen = HashSet::new();
    for game in games {
        if !seen.insert(game_key(&game)) {
            continue;
        }

        let date = game.game_finished_at.date_naive();
        if date < start || date > today {
            continue;
        }

        if let Some(index) = result_index(&game.match_result.to_ascii_lowercase()) {
            counts.entry(date).or_insert([0; 3])[index] += 1;
        }
    }

    let mut dates = Vec::new();
    let mut date = start;
    while date <= today {
        dates.push(date);
        date = date.succ_opt().unwrap();
    }

    let max_count = counts
        .values()
        .map(|daily| daily.iter().sum::<u32>())
        .max()
        .unwrap_or(1)
        .max(5);

    let width = 1024usize;
    let height = 720usize;
    register_font(
        "sans-serif",
        style::FontStyle::Normal,
        include_bytes!("../../assets/Roboto-Regular.ttf"),
    )
    .map_err(|_| eyre!("Failed to register font"))?;

    let mut buffer = vec![0; width * height * 3];
    {
        let root = BitMapBackend::with_buffer(&mut buffer, (width as u32, height as u32))
            .into_drawing_area();
        root.fill(&WHITE)?;

        let caption = match filter_user {
            Some(user) => format!("{user} — wins, losses & ties (last {DAYS_SHOWN} days)"),
            None => format!("Configured players — wins, losses & ties (last {DAYS_SHOWN} days)"),
        };
        let mut chart = ChartBuilder::on(&root)
            .set_label_area_size(LabelAreaPosition::Left, 70)
            .set_label_area_size(LabelAreaPosition::Bottom, 70)
            .caption(caption, ("sans-serif", 40))
            .margin(20)
            .build_cartesian_2d(
                start..today.succ_opt().unwrap_or(today),
                0f32..(max_count as f32 + 1.0),
            )?;

        chart
            .configure_mesh()
            .x_label_style(style::TextStyle::from(("sans-serif", 25).into_font()))
            .y_label_style(style::TextStyle::from(("sans-serif", 30).into_font()))
            .x_desc("Date")
            .y_desc("Matches")
            .y_labels(10)
            .x_labels(0)
            .x_label_formatter(&|_| String::new())
            .y_label_formatter(&|value| format!("{value:.0}"))
            .disable_mesh()
            .draw()?;

        let mut month_ticks = Vec::new();
        let mut month = NaiveDate::from_ymd_opt(start.year(), start.month(), 1).unwrap_or(start);
        while month <= today {
            month_ticks.push(month);
            let (year, month_number) = (month.year(), month.month());
            let (next_year, next_month) = if month_number == 12 {
                (year + 1, 1)
            } else {
                (year, month_number + 1)
            };
            if let Some(next) = NaiveDate::from_ymd_opt(next_year, next_month, 1) {
                month = next;
            } else {
                break;
            }
        }

        for &tick in &month_ticks {
            chart.draw_series(std::iter::once(PathElement::new(
                vec![(tick, 0f32), (tick, max_count as f32 + 1.0)],
                RGBColor(200, 200, 200).stroke_width(1),
            )))?;

            let days_from_start = (tick - start).num_days() as f64;
            let total_days = ((today - start).num_days() + 1) as f64;
            let x_ratio = days_from_start / total_days;
            let chart_left = 90i32;
            let chart_right = width as i32 - 20;
            let chart_bottom = height as i32 - 75;
            let x_pixel = chart_left + ((chart_right - chart_left) as f64 * x_ratio) as i32;

            root.draw(&Text::new(
                tick.format("%Y-%m").to_string(),
                (x_pixel, chart_bottom),
                TextStyle::from(("sans-serif", 25)).pos(Pos::new(HPos::Center, VPos::Top)),
            ))?;
        }

        let series = [
            ("Wins", RGBColor(52, 199, 89)),
            ("Losses", RGBColor(255, 59, 48)),
            ("Ties", RGBColor(255, 214, 10)),
        ];

        for date in &dates {
            let Some(daily_counts) = counts.get(date) else {
                continue;
            };
            let next_date = date.succ_opt().unwrap_or(*date);
            let mut bottom = 0u32;
            for (index, (_, color)) in series.iter().enumerate() {
                let count = daily_counts[index];
                if count > 0 {
                    chart.draw_series(std::iter::once(Rectangle::new(
                        [(*date, bottom as f32), (next_date, (bottom + count) as f32)],
                        color.filled(),
                    )))?;
                }
                bottom += count;
            }
        }

        for (label, color) in series {
            chart
                .draw_series(std::iter::once(Rectangle::new(
                    [(start, 0f32), (start, 0f32)],
                    color,
                )))?
                .label(label)
                .legend(move |(x, y)| {
                    Rectangle::new([(x, y - 15), (x + 15, y + 5)], color.filled())
                });
        }

        chart
            .configure_series_labels()
            .border_style(BLACK)
            .background_style(WHITE.mix(0.8))
            .position(plotters::chart::SeriesLabelPosition::UpperLeft)
            .label_font(("sans-serif", 20))
            .draw()?;

        root.present()?;
    }

    let image = image::RgbImage::from_raw(width as u32, height as u32, buffer)
        .ok_or_else(|| eyre!("Image buffer not large enough"))?;
    let mut bytes = Vec::new();
    image.write_to(
        &mut std::io::Cursor::new(&mut bytes),
        image::ImageFormat::Png,
    )?;

    Ok(bytes)
}
