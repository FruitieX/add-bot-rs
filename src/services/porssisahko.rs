use std::cmp::{max, min};

use cached::proc_macro::cached;
use chrono::{DateTime, Duration, Timelike, Utc};
use chrono_tz::Tz;
use color_eyre::{eyre::eyre, Result};
use plotters::{
    chart::{ChartBuilder, LabelAreaPosition},
    prelude::{BitMapBackend, IntoDrawingArea, Rectangle},
    style::{self, register_font, Color, FontStyle, IntoFont, RGBColor, BLACK, RED, WHITE},
};
use serde::Deserialize;

fn fmt_x_axis(x: &DateTime<Tz>) -> String {
    if x.hour() == 0 {
        x.format("%d.%m").to_string()
    } else {
        x.format("%H:%M").to_string()
    }
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
        .max(10.);
    let min_price = prices.iter().fold(0.0f32, |a, &b| a.min(b.price));

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

    let start_date = start_date.with_timezone(&chrono_tz::Europe::Helsinki);
    let end_date = end_date.with_timezone(&chrono_tz::Europe::Helsinki);
    let current_date = Utc::now().with_timezone(&chrono_tz::Europe::Helsinki);

    let current_price = prices
        .iter()
        .rev()
        .find(|hp| Utc::now() > hp.start_date)
        .map(|x| x.price)
        .unwrap_or_default();

    let mut buffer = vec![0; width * height * 3];
    // let mut buffer = String::new();
    {
        let root = BitMapBackend::with_buffer(&mut buffer, (width as u32, height as u32))
            .into_drawing_area();
        // let root =
        //     SVGBackend::with_string(&mut buffer, (width as u32, height as u32)).into_drawing_area();
        root.fill(&WHITE)?;

        let mut ctx = ChartBuilder::on(&root)
            .set_label_area_size(LabelAreaPosition::Left, 60)
            .set_label_area_size(LabelAreaPosition::Bottom, 80)
            .caption(
                format!(
                    "       Elpriser {from} och {to} (nuvarande pris: {current_price:.2} c/kWh)",
                    from = start_date.date_naive(),
                    to = end_date.to_utc().date_naive(),
                ),
                ("sans-serif", 35),
            )
            .margin(20)
            .build_cartesian_2d(start_date..end_date, min_price..(max_price + 1.0))?;

        let x_label_style = style::TextStyle::from(("sans-serif", 25).into_font());
        let y_label_style = style::TextStyle::from(("sans-serif", 35).into_font());

        ctx.configure_mesh()
            .set_all_tick_mark_size(10.)
            .x_labels(16)
            .y_labels(16)
            .x_label_style(x_label_style)
            .y_label_style(y_label_style)
            .x_label_formatter(&fmt_x_axis)
            .y_label_formatter(&fmt_y_axis)
            .x_max_light_lines(3)
            .y_max_light_lines(2)
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

        let cur_price_line_thickness = Duration::minutes(6);

        ctx.draw_series([
            // Grey out past prices
            Rectangle::new(
                [
                    (start_date, f32::NEG_INFINITY),
                    (current_date - cur_price_line_thickness, f32::INFINITY),
                ],
                BLACK.mix(0.1).filled(),
            ),
            // Draw a line at current price
            Rectangle::new(
                [
                    (current_date - cur_price_line_thickness, f32::NEG_INFINITY),
                    (current_date + cur_price_line_thickness, f32::INFINITY),
                ],
                RED.mix(0.5).filled(),
            ),
        ])?;

        // Draw some extra ticks on the x-axis for each hour.
        let hour_tick_thickness = Duration::minutes(3);
        let hour_tick_height = max_price / 100.0;

        ctx.draw_series(
            prices
                .iter()
                .filter(|hp| hp.start_date.minute() == 0)
                .map(|hp| {
                    Rectangle::new(
                        [
                            (
                                hp.start_date.with_timezone(&chrono_tz::Europe::Helsinki),
                                hour_tick_height,
                            ),
                            (
                                (hp.start_date + hour_tick_thickness)
                                    .with_timezone(&chrono_tz::Europe::Helsinki),
                                0.0 - hour_tick_height,
                            ),
                        ],
                        BLACK.filled(),
                    )
                }),
        )?;

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

    // For some reason the API returns the prices in reverse order
    let prices: Vec<HourlyPrice> = resp
        .prices
        .into_iter()
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
