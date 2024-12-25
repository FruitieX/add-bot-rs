use cached::proc_macro::cached;
use chrono::{DateTime, Duration, Timelike, Utc};
use chrono_tz::Tz;
use color_eyre::{eyre::eyre, Result};
use plotters::{
    chart::{ChartBuilder, LabelAreaPosition},
    prelude::{BitMapBackend, IntoDrawingArea, Rectangle},
    series::LineSeries,
    style::{self, register_font, Color, FontStyle, IntoFont, BLACK, BLUE, RED, WHITE},
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
    let max_price = prices
        .iter()
        .fold(f32::NEG_INFINITY, |a, &b| a.max(b.price))
        .max(10.);
    let min_price = prices.iter().fold(0.0f32, |a, &b| a.min(b.price));

    // Simulate a bar chart
    let prices = prices
        .iter()
        .flat_map(|hp| {
            [*hp, {
                let mut hp = *hp;
                hp.start_date += Duration::hours(1) - Duration::nanoseconds(1);
                hp
            }]
        })
        .collect::<Vec<_>>();

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

        ctx.draw_series(LineSeries::new(
            prices.iter().map(|hp| {
                (
                    hp.start_date.with_timezone(&chrono_tz::Europe::Helsinki),
                    hp.price,
                )
            }),
            &BLUE,
        ))?;

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
    let url = "https://api.porssisahko.net/v1/latest-prices.json";
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

    let first = prices.first().ok_or_else(|| eyre!("No prices found"))?;
    let prices = [
        // Hack to workaround the API not returning prices for the first hour of
        // the day, and the chart library misaligning the x-axis labels as a
        // result.
        vec![HourlyPrice {
            price: first.price,
            start_date: first.start_date - chrono::Duration::hours(1),
        }],
        prices,
    ]
    .concat();

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
