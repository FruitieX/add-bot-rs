use cached::proc_macro::cached;
use std::cmp::{max, min};
use chrono::{DateTime, Duration, Timelike, Utc};
use chrono_tz::Tz;
use color_eyre::{eyre::eyre, Result};
use plotters::{
    chart::{ChartBuilder, LabelAreaPosition},
    prelude::{BitMapBackend, IntoDrawingArea, LineSeries, Rectangle, Text},
    style::{
        self, register_font,
        text_anchor::{HPos, Pos, VPos},
        Color, FontStyle, IntoFont, RGBColor, ShapeStyle, BLACK, RED, WHITE,
    },
};
use serde::Deserialize;

const TZ: Tz = chrono_tz::Europe::Helsinki;

fn fmt_x_axis(x: &DateTime<Tz>) -> String {
    x.format("%_H:00").to_string()
}

fn fmt_y_axis(y: &f32) -> String {
    format!("{y:.0}")
}

#[cached(result = true, time = 1)]
pub async fn get_price_chart() -> Result<Vec<u8>> {
    // Get prices
    let prices = get_latest_prices().await?;

    // We remove 3 hours * 4 15-minute prices to align the chart more nicely.
    let prices: Vec<HourlyPrice> = prices.into_iter().skip(12).collect();

    let max_price = prices
        .iter()
        .fold(f32::NEG_INFINITY, |a, &b| a.max(b.price))
        .max(15.)
        + 5.0;
    let mut min_price = prices.iter().fold(0.0f32, |a, &b| a.min(b.price));

    // Move the min price a bit lower for better visual spacing
    // in case we have negative prices
    if min_price < 0.0 {
        min_price -= 5.0;
    } else {
        min_price = 0.0;
    }

    // Generate a chart
    let width: usize = 1024;
    let height: usize = 768;

    register_font(
        "sans-serif",
        FontStyle::Normal,
        include_bytes!("../../assets/Roboto-Regular.ttf"),
    )
    .map_err(|_| eyre!("Failed to register font"))?;

    let (
        Some(HourlyPrice { start_date, .. }),
        Some(HourlyPrice {
            start_date: end_date,
            ..
        }),
    ) = (prices.first(), prices.last())
    else {
        return Err(eyre!("No prices found"));
    };

    let start_date = start_date.with_timezone(&TZ);
    let end_date = end_date.with_timezone(&TZ);
    let current_date = Utc::now().with_timezone(&TZ);

    let mut buffer = vec![0; width * height * 3];
    // let mut buffer = String::new();
    {
        let root = BitMapBackend::with_buffer(&mut buffer, (width as u32, height as u32))
            .into_drawing_area();
        // let root =
        //     SVGBackend::with_string(&mut buffer, (width as u32, height as u32)).into_drawing_area();
        root.fill(&WHITE)?;

        let mut ctx = ChartBuilder::on(&root)
            .set_label_area_size(LabelAreaPosition::Left, 80)
            .set_label_area_size(LabelAreaPosition::Bottom, 80)
            .caption(
                format!(
                    "Elpris {from}—{to}",
                    from = start_date.format("%d.%m.%Y").to_string(),
                    to = end_date.format("%d.%m.%Y").to_string(),
                ),
                ("sans-serif", 35),
            )
            .margin(20)
            .build_cartesian_2d(start_date..end_date, min_price..max_price)?;

        let y_label_style = style::TextStyle::from(("sans-serif", 28).into_font())
            .pos(Pos::new(HPos::Right, VPos::Bottom));
        let x_label_style = style::TextStyle::from(("sans-serif", 26).into_font());
        let x_label_style_date = style::TextStyle::from(("sans-serif", 26).into_font())
            .pos(Pos::new(HPos::Center, VPos::Top));

        // make a single ShapeStyle representing the gray mesh style (use same stroke width)
        let axis_mesh_style = ShapeStyle::from(&RGBColor(150, 150, 150)).stroke_width(1);

        // determine an x-label count so labels are at most every 4 hours
        let total_hours = ((end_date - start_date).num_seconds() / 3600) as i64;
        let mut x_label_count = (total_hours / 4) as usize + 1; // floor(total_hours/4) + 1
        if x_label_count == 0 {
            x_label_count = 1;
        }
        // clamp to a reasonable maximum to avoid too many labels
        x_label_count = x_label_count.min(16);

        ctx.configure_mesh()
            .set_all_tick_mark_size(10.)
            .x_labels(x_label_count)
            .y_labels(12)
            .x_label_style(x_label_style)
            .y_label_style(y_label_style)
            .x_label_formatter(&fmt_x_axis)
            .y_label_formatter(&fmt_y_axis)
            .y_desc("Price (c/kWh)")
            .x_max_light_lines(4)
            .y_max_light_lines(1)
            .axis_style(axis_mesh_style.clone()) // make axis lines match mesh style
            .draw()?;

       ctx.draw_series(prices.iter().map(|hp| {
            Rectangle::new(
                [
                    (
                        hp.start_date.with_timezone(&chrono_tz::Europe::Helsinki),
                        0.0,
                    ),
                    (
                        (hp.start_date + Duration::minutes(15))
                            .with_timezone(&chrono_tz::Europe::Helsinki),
                        hp.price,
                    ),
                ],
                {
                    let gradient = colorous::VIRIDIS;

                    // We scale up the prices from 0.xx cents to something more usize friendly, as we need
                    // that later to get the color gradient.
                    let scale_up = 100;
                    let current_price = (hp.price * scale_up as f32) as usize;

                    // We "push" values up a bit artificially, so that we can avoid the first part of the color gradient.
                    let tulttans_constant = 0.2;

                    // This determins how dark the darkest price is. In the future, it could maybe be based on the
                    // highest price of the day?
                    let max_price: f32 = (30 * scale_up) as f32;
                    let tulttans_max_price = (max_price * (1.0 + tulttans_constant)) as usize;

                    // We "push" values up a bit artificially, so that we can avoid the first part of the color gradient.
                    let tulttans_fix_to_avoid_spy_color = (tulttans_constant * max_price) as usize;

                    let cor = gradient.eval_rational(
                        tulttans_max_price
                            - min(
                                max(current_price, 0) + tulttans_fix_to_avoid_spy_color,
                                tulttans_max_price,
                            ),
                        tulttans_max_price,
                    );
                    RGBColor(cor.r, cor.g, cor.b).filled()
                },
            )
        }))?;

        // Highlight the step segment that corresponds to the current time in red,
        // and annotate it above the line with a small gray connector.
        if let Some(cur_hp) = prices.iter().rev().find(|hp| Utc::now() > hp.start_date) {
            let seg_start = cur_hp.start_date.with_timezone(&TZ);
            let seg_end = (cur_hp.start_date + Duration::minutes(18)).with_timezone(&TZ);
            let seg_price = cur_hp.price;

            // Draw the red step segment for the current interval
            ctx.draw_series(LineSeries::new(
                vec![(seg_start, seg_price), (seg_end, seg_price)].into_iter(),
                ShapeStyle::from(&RED).stroke_width(2),
            ))?;

            // Annotation position: a bit to the right of the segment and above the line
            let text_offset = Duration::minutes(20);
            let y_offset_val = (max_price - min_price) * 0.06_f32; // vertical offset for the annotation
            let label_pos = (seg_end + text_offset, seg_price + y_offset_val);

            // Draw a thin gray connector from the step (midpoint) up to the annotation
            let seg_mid = seg_start + Duration::seconds((seg_end - seg_start).num_seconds() / 2);
            ctx.draw_series(LineSeries::new(
                vec![
                    (seg_mid, seg_price + y_offset_val / 10.0),
                    (seg_mid, seg_price + y_offset_val),
                ]
                .into_iter(),
                ShapeStyle::from(BLACK.mix(0.3).filled()).stroke_width(1),
            ))?;

            // Draw the annotation text above the connector
            let cur_label = format!("{seg_price:.2}");
            let cur_label_style = style::TextStyle::from(("sans-serif", 26).into_font())
                .pos(Pos::new(HPos::Center, VPos::Bottom)); // bottom anchor -> text sits above the coord
            ctx.draw_series(std::iter::once(Text::new(
                cur_label,
                label_pos,
                cur_label_style,
            )))?;
        }


        ctx.draw_series(std::iter::once(
            // Grey out past price
            Rectangle::new(
                [
                    (start_date, f32::NEG_INFINITY),
                    (current_date, f32::INFINITY),
                ],
                BLACK.mix(0.08).filled(),
            ),
        ))?;

        // Draw date labels under the x-axis

        let mut first = true;
        // position the date labels in the reserved bottom label area (pixel coordinates)
        // ChartBuilder used: margin = 20, left label area = 60, bottom label area = 80
        let margin_px = 20.0;
        let left_label_px = 80.0;
        let bottom_label_px = 60.0;

        // plotting area pixel bounds (approx) — we draw in the root so labels are not clipped by the plot area
        let plot_left_px = margin_px + left_label_px;
        let plot_right_px = (width as f64) - margin_px;
        let plot_width_px = plot_right_px - plot_left_px;

        let duration_secs = (end_date - start_date).num_seconds() as f64;

        // draw date labels under the x-axis in the bottom label area using pixel coordinates on `root`
        for hp in &prices {
            let tick_dt = hp.start_date.with_timezone(&TZ);

            if first || (tick_dt.hour() == 0 && tick_dt.minute() == 0) {
                first = false;

                // fraction across the x-range for this timestamp
                let offset_secs = (tick_dt - start_date).num_seconds() as f64;
                let frac = (offset_secs / duration_secs).clamp(0.0, 1.0);

                // convert fraction to pixel X in root coordinates
                let x_px = (plot_left_px + frac * plot_width_px).round() as i32;

                // place the date label vertically inside the bottom label area (tweak as needed)
                let y_px = (height as i32) - (margin_px as i32) - (bottom_label_px as i32 / 2);

                let date_label = tick_dt.format("%d.%m").to_string();

                // draw directly on the root drawing area using pixel coords so the text appears under the axis
                root.draw(&Text::new(
                    date_label,
                    (x_px, y_px),
                    x_label_style_date.clone(),
                ))?;
            }
        }

        root.present()?;
    }

    // Write to image
    let image = image::RgbImage::from_raw(width as u32, height as u32, buffer)
        .ok_or_else(|| eyre!("Image buffer not large enough"))?;

    let mut bytes: Vec<u8> = Vec::new();
    image.write_to(
        &mut std::io::Cursor::new(&mut bytes),
        image::ImageFormat::Png,
    )?;

    // let bytes = buffer.as_bytes().to_vec();

    Ok(bytes)
}

#[cached(result = true, time = 60)]
async fn get_latest_prices() -> Result<Vec<HourlyPrice>> {
    println!("Fetching latest sahko prices");
    let url = "https://api.porssisahko.net/v2/latest-prices.json";
    let resp: PricesResult = reqwest::get(url).await?.json().await?;

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

    // This test actually calls the remote API and writes a PNG to /tmp.
    // It's ignored by default because it requires network access and the font asset.
    // Run explicitly with: cargo test -- --ignored
    #[tokio::test]
    // #[ignore = "requires network and font asset; run explicitly with --ignored"]
    async fn write_price_chart_to_file() {
        let bytes = get_price_chart().await.expect("get_price_chart failed");
        assert!(!bytes.is_empty(), "returned image buffer was empty");
        fs::write("./porssisahko_test.png", &bytes).expect("failed to write image file");
    }
}
