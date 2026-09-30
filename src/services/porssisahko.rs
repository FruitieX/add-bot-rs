use super::{
    chart_style::{self, ChartPanel, ACCENT, BORDER, INK, MUTED, WIDTH},
    chart_text::ChartBackend,
};
use cached::proc_macro::cached;
use chrono::{DateTime, Duration, Timelike, Utc};
use chrono_tz::Tz;
use color_eyre::{eyre::eyre, Result};
use plotters::{
    chart::{ChartBuilder, LabelAreaPosition},
    prelude::*,
    style::text_anchor::{HPos, Pos, VPos},
};
use serde::Deserialize;

const TZ: Tz = chrono_tz::Europe::Helsinki;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EnergyCostEstimate {
    pub energy_kwh: f64,
    pub spot_cost_eur: f64,
    pub distribution_cost_eur: f64,
    pub total_cost_eur: f64,
}

/// Calculates the variable energy cost for a future interval.
///
/// `HourlyPrice::price` is expressed in cents/kWh even though the upstream
/// endpoint uses 15-minute intervals. Costs are prorated when the requested
/// interval does not align with a price slot.
pub fn estimate_energy_cost(
    prices: &[HourlyPrice],
    start: DateTime<Utc>,
    duration_minutes: f64,
    power_watts: f64,
    distribution_cents_per_kwh: f64,
) -> Result<EnergyCostEstimate> {
    if duration_minutes <= 0.0 {
        return Err(eyre!("Match duration must be positive"));
    }
    if power_watts < 0.0 || distribution_cents_per_kwh < 0.0 {
        return Err(eyre!("Power and distribution cost must not be negative"));
    }

    let duration = Duration::milliseconds((duration_minutes * 60_000.0).round() as i64);
    let end = start + duration;
    let requested_seconds = duration.num_seconds();
    let mut covered_seconds = 0i64;
    let mut energy_kwh = 0.0;
    let mut spot_cost_eur = 0.0;
    let power_kw = power_watts / 1_000.0;

    for price in prices {
        let slot_start = price.start_date;
        let slot_end = slot_start + Duration::minutes(15);
        let overlap_start = start.max(slot_start);
        let overlap_end = end.min(slot_end);
        let overlap_seconds = (overlap_end - overlap_start).num_seconds();

        if overlap_seconds <= 0 {
            continue;
        }

        let slot_energy_kwh = power_kw * overlap_seconds as f64 / 3_600.0;
        energy_kwh += slot_energy_kwh;
        spot_cost_eur += slot_energy_kwh * f64::from(price.price) / 100.0;
        covered_seconds += overlap_seconds;
    }

    if covered_seconds < requested_seconds {
        return Err(eyre!(
            "Spot-price data does not cover the complete match interval"
        ));
    }

    let distribution_cost_eur = energy_kwh * distribution_cents_per_kwh / 100.0;

    Ok(EnergyCostEstimate {
        energy_kwh,
        spot_cost_eur,
        distribution_cost_eur,
        total_cost_eur: spot_cost_eur + distribution_cost_eur,
    })
}
#[cached(ttl_secs = 1)]
pub async fn get_price_chart() -> Result<Vec<u8>> {
    render_price_chart(&get_latest_prices().await?, Utc::now(), TZ, None)
}

pub(crate) fn render_price_chart(
    prices: &[HourlyPrice],
    now: DateTime<Utc>,
    tz: Tz,
    panel: Option<&ChartPanel>,
) -> Result<Vec<u8>> {
    // Preserve the existing three-hour trim, including all remaining 15-minute slots.
    let prices = &prices[if prices.len() > 12 { 12 } else { 0 }..];
    let first = prices
        .first()
        .ok_or(super::failure::DataFailure::NoPrices)?;
    let last = prices.last().unwrap();
    let start = first.start_date.with_timezone(&tz);
    let end = (last.start_date + Duration::minutes(15)).with_timezone(&tz);
    let current = prices
        .iter()
        .find(|p| p.start_date <= now && now < p.start_date + Duration::minutes(15));
    let low = prices.iter().map(|p| p.price).fold(f32::INFINITY, f32::min);
    let high = prices
        .iter()
        .map(|p| p.price)
        .fold(f32::NEG_INFINITY, f32::max);
    let padding = ((high - low) * 0.12).max(2.0);
    let minimum = if low < 0.0 { low - padding } else { 0.0 };
    let maximum = high.max(5.0) + padding;
    let height = 930 + panel.map_or(0, ChartPanel::height);
    let mut buffer = vec![0; WIDTH as usize * height as usize * 3];
    {
        let root = ChartBackend(BitMapBackend::with_buffer(&mut buffer, (WIDTH, height)))
            .into_drawing_area();
        chart_style::frame(&root)?;
        chart_style::text(&root, "Electricity prices", (100, 86), 46, INK, true)?;
        chart_style::text(
            &root,
            &format!(
                "{} – {}  ·  {} time  ·  15-minute prices",
                start.format("%-d %b %Y"),
                end.format("%-d %b %Y"),
                tz
            ),
            (100, 150),
            22,
            MUTED,
            false,
        )?;
        for (x, value, label) in [
            (
                100,
                current
                    .map(|p| format!("{:.2}", p.price))
                    .unwrap_or_else(|| "—".into()),
                "NOW · c/kWh",
            ),
            (590, format!("{low:.2}"), "PERIOD LOW · c/kWh"),
            (1080, format!("{high:.2}"), "PERIOD HIGH · c/kWh"),
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
            .build_cartesian_2d(start..end, minimum..maximum)?;
        ctx.configure_mesh()
            // Plotters' datetime tick generator overflows for a zero tick budget.
            // Native labels are hidden; the readable date/time labels are below.
            .x_labels(2)
            .x_label_formatter(&|_| String::new())
            .y_labels(6)
            .disable_x_mesh()
            .axis_style(WHITE)
            .bold_line_style(BORDER)
            .light_line_style(WHITE)
            .y_label_style(("sans-serif", 22).into_font().color(&MUTED))
            .y_label_formatter(&|v| format!("{v:.0}"))
            .set_all_tick_mark_size(0)
            .draw()?;
        // Calm teal bars; higher-priced intervals become progressively darker.
        ctx.draw_series(prices.iter().map(|p| {
            let t = ((p.price - low) / (high - low).max(1.0)).clamp(0.0, 1.0);
            let color = RGBColor(
                (91.0 - 45.0 * t) as u8,
                (183.0 - 70.0 * t) as u8,
                (181.0 - 47.0 * t) as u8,
            );
            Rectangle::new(
                [
                    (p.start_date.with_timezone(&tz), 0.0),
                    (
                        (p.start_date + Duration::minutes(15)).with_timezone(&tz),
                        p.price,
                    ),
                ],
                color.filled(),
            )
        }))?;
        if minimum < 0.0 {
            ctx.draw_series(std::iter::once(PathElement::new(
                vec![(start, 0.0), (end, 0.0)],
                MUTED.mix(0.5),
            )))?;
        }
        if now >= first.start_date && now < last.start_date + Duration::minutes(15) {
            let local_now = now.with_timezone(&tz);
            ctx.draw_series(std::iter::once(PathElement::new(
                vec![(local_now, minimum), (local_now, maximum)],
                INK.mix(0.45).stroke_width(2),
            )))?;
            let point = ctx.backend_coord(&(local_now, maximum));
            chart_style::text(
                &root,
                "NOW",
                (point.0.clamp(130, 1350) - 18, 310),
                18,
                INK,
                true,
            )?;
        }
        let duration = (end - start).num_seconds() as f64;
        let mut last_label_x = -1000;
        for (index, p) in prices.iter().enumerate() {
            let time = p.start_date.with_timezone(&tz);
            if index != 0 && (time.minute() != 0 || time.hour() % 6 != 0) {
                continue;
            }
            let coord = ctx.backend_coord(&(time, minimum));
            if coord.0 - last_label_x < 110
                || (index != 0 && (end - time).num_seconds() as f64 / duration < 0.035)
            {
                continue;
            }
            last_label_x = coord.0;
            let label = TextStyle::from(("sans-serif", 22))
                .color(&MUTED)
                .pos(Pos::new(HPos::Center, VPos::Top));
            root.draw(&Text::new(
                time.format("%H:%M").to_string(),
                (coord.0, 835),
                label.clone(),
            ))?;
            root.draw(&Text::new(
                time.format("%-d %b").to_string(),
                (coord.0, 867),
                label,
            ))?;
        }
        if let Some(panel) = panel {
            root.draw(&PathElement::new(vec![(100, 925), (1400, 925)], BORDER))?;
            chart_style::text(
                &root,
                "ESTIMATED ELECTRICITY · PER PC / MATCH",
                (100, 955),
                23,
                INK,
                true,
            )?;
            let mut y = 1005;
            for (label, cost) in &panel.rows {
                chart_style::text(
                    &root,
                    &chart_style::fit_text(label, 24, 980),
                    (100, y),
                    24,
                    INK,
                    false,
                )?;
                chart_style::text(
                    &root,
                    &chart_style::fit_text(cost, 24, 260),
                    (1120, y),
                    24,
                    ACCENT,
                    true,
                )?;
                y += 44;
            }
            y += 15;
            for note in &panel.notes {
                chart_style::text(
                    &root,
                    &chart_style::fit_text(note, 20, 1300),
                    (100, y),
                    20,
                    MUTED,
                    false,
                )?;
                y += 30;
            }
        }
        root.present()?;
    }
    let image = image::RgbImage::from_raw(WIDTH, height, buffer)
        .ok_or_else(|| eyre!("Invalid image buffer"))?;
    let mut bytes = Vec::new();
    image.write_to(
        &mut std::io::Cursor::new(&mut bytes),
        image::ImageFormat::Png,
    )?;
    Ok(bytes)
}

#[cached(ttl_secs = 60)]
pub(crate) async fn get_latest_prices() -> Result<Vec<HourlyPrice>> {
    println!("Fetching latest sahko prices");
    let url = "https://api.porssisahko.net/v2/latest-prices.json";
    let response = reqwest::get(url)
        .await
        .map_err(|error| super::failure::upstream(error.into(), "Electricity price service"))?
        .error_for_status()
        .map_err(|error| super::failure::upstream(error.into(), "Electricity price service"))?;
    let resp: PricesResult = response
        .json()
        .await
        .map_err(|error| super::failure::upstream(error.into(), "Electricity price service"))?;

    // In case it is relevant to filter only the recent 24 hours
    // let current_date = Utc::now().with_timezone(&TZ);

    // For some reason the API returns the prices in reverse order
    let prices: Vec<HourlyPrice> = resp
        .prices
        .into_iter()
        // Uncomment to keep only the last 24 hours
        // .filter(|hp| hp.start_date >= current_date - Duration::hours(24))
        // Uncomment to test how chart behaves with larger numbers
        // .map(|x| HourlyPrice {
        //     price: x.price + 170.,
        //     ..x
        // })
        .rev()
        .collect();

    Ok(prices)
}

#[derive(Debug, Copy, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HourlyPrice {
    pub price: f32,
    pub start_date: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PricesResult {
    pub prices: Vec<HourlyPrice>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn preview_prices() -> (DateTime<Utc>, Vec<HourlyPrice>) {
        use chrono::TimeZone;
        let start = Utc.with_ymd_and_hms(2026, 9, 30, 0, 0, 0).unwrap();
        let prices = (0..192)
            .map(|index| HourlyPrice {
                start_date: start + Duration::minutes(index * 15),
                price: (index as f32 * 0.06).sin() * 8.0 + 6.0,
            })
            .collect();
        (start + Duration::hours(14), prices)
    }

    #[test]
    fn price_render_handles_negative_flat_and_missing_prices_and_growing_queue_panels() {
        let (now, prices) = preview_prices();
        let panel = ChartPanel {
            rows: (0..12)
                .map(|index| (format!("Queue {index} · If filled now"), "€0.06".into()))
                .collect(),
            notes: vec![
                "500 W PC · Estimated 48-minute match".into(),
                "Includes spot energy + variable transfer".into(),
            ],
        };
        for (prices, panel) in [
            (&prices[..], None),
            (&prices[..], Some(&panel)),
            (&prices[..1], None),
        ] {
            let bytes = render_price_chart(prices, now, TZ, panel).unwrap();
            let image = image::load_from_memory(&bytes).unwrap().to_rgb8();
            assert_eq!(image.width(), WIDTH);
            assert_eq!(image.height(), 930 + panel.map_or(0, ChartPanel::height));
            assert_eq!(image.get_pixel(0, 0).0, [243, 246, 249]);
            assert_eq!(image.get_pixel(100, image.height() - 40).0, [255, 255, 255]);
            if let Some(panel) = panel {
                let y = 1005 + (panel.rows.len() - 1) as u32 * 44;
                assert!(
                    (y..y + 30).any(|y| (1120..1350).any(|x| image.get_pixel(x, y).0 != [255; 3]))
                );
            }
        }
        assert!(render_price_chart(&[], now, TZ, None).is_err());
    }

    #[test]
    #[ignore = "writes deterministic electricity previews for visual review"]
    fn write_electricity_preview() {
        let (now, prices) = preview_prices();
        let panel = ChartPanel {
            rows: vec![
                ("Today 19:30".into(), "€0.06".into()),
                ("Instant queue · If filled now".into(), "€0.05".into()),
            ],
            notes: vec![
                "500 W PC · Estimated 48-minute match from 30 recent matches' rounds".into(),
                "Includes spot energy + variable transfer; excludes fixed fees and electricity tax"
                    .into(),
            ],
        };
        fs::write(
            "target/electricity-preview.png",
            render_price_chart(&prices, now, TZ, Some(&panel)).unwrap(),
        )
        .unwrap();
        fs::write(
            "target/electricity-no-queues-preview.png",
            render_price_chart(&prices, now, TZ, None).unwrap(),
        )
        .unwrap();
        let panel = ChartPanel {
            rows: (0..12)
                .map(|index| (format!("Queue {index:02}"), "Unavailable".into()))
                .collect(),
            notes: vec![
                "Estimates unavailable: no usable round data in recent match history".into(),
            ],
        };
        fs::write(
            "target/electricity-unavailable-preview.png",
            render_price_chart(&prices, now, TZ, Some(&panel)).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn energy_cost_prorates_spot_prices_and_adds_distribution() {
        let start = chrono::DateTime::parse_from_rfc3339("2026-09-12T18:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let prices = vec![
            HourlyPrice {
                price: 10.0,
                start_date: start,
            },
            HourlyPrice {
                price: 20.0,
                start_date: start + Duration::minutes(15),
            },
        ];

        let estimate = estimate_energy_cost(&prices, start, 30.0, 1_000.0, 2.0)
            .expect("price data should cover the match");

        assert!((estimate.energy_kwh - 0.5).abs() < 1e-9);
        assert!((estimate.spot_cost_eur - 0.075).abs() < 1e-9);
        assert!((estimate.distribution_cost_eur - 0.01).abs() < 1e-9);
        assert!((estimate.total_cost_eur - 0.085).abs() < 1e-9);
    }

    #[test]
    fn energy_cost_rejects_a_match_outside_available_price_data() {
        let start = chrono::DateTime::parse_from_rfc3339("2026-09-12T18:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let prices = vec![HourlyPrice {
            price: 10.0,
            start_date: start,
        }];

        assert!(estimate_energy_cost(&prices, start, 30.0, 500.0, 2.0).is_err());
    }

    // This test actually calls the remote API and writes a PNG to /tmp.
    // It's ignored by default because it requires network access and the font asset.
    // Run explicitly with: cargo test -- --ignored
    #[tokio::test]
    #[ignore = "requires live price API; deterministic rendering is tested separately"]
    async fn write_price_chart_to_file() {
        let bytes = get_price_chart().await.expect("get_price_chart failed");
        assert!(!bytes.is_empty(), "returned image buffer was empty");
        fs::write("./porssisahko_test.png", &bytes).expect("failed to write image file");
    }
}
