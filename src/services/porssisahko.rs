use anyhow::{anyhow, Result};
use cached::proc_macro::cached;
use chrono::{DateTime, Timelike, Utc};
use plotters::{
    chart::{ChartBuilder, LabelAreaPosition},
    prelude::{BitMapBackend, IntoDrawingArea},
    series::LineSeries,
    style::{register_font, FontStyle, BLUE, WHITE},
};
use serde::Deserialize;

fn fmt_x_axis(x: &DateTime<Utc>) -> String {
    if x.hour() == 0 {
        x.format("%d.%m %H:%M").to_string()
    } else {
        x.format("%H:%M").to_string()
    }
}

#[cached(result = true, time = 60)]
pub async fn get_price_chart() -> Result<Vec<u8>> {
    // Get prices
    let prices = get_latest_prices().await?;
    let max_price = prices
        .iter()
        .fold(f32::NEG_INFINITY, |a, &b| a.max(b.price));
    let min_price = prices.iter().fold(0.0f32, |a, &b| a.min(b.price));

    // Generate a chart
    let width: usize = 1024;
    let height: usize = 768;

    register_font(
        "sans-serif",
        FontStyle::Normal,
        include_bytes!("../../assets/Roboto-Regular.ttf"),
    )
    .map_err(|_| anyhow!("Failed to register font"))?;

    let (
        Some(HourlyPrice { start_date, .. }),
        Some(HourlyPrice {
            start_date: end_date,
            ..
        }),
    ) = (prices.last().cloned(), prices.first().cloned())
    else {
        return Err(anyhow!("No prices found"));
    };

    let mut buffer = vec![0; width * height * 3];
    {
        let root = BitMapBackend::with_buffer(&mut buffer, (width as u32, height as u32))
            .into_drawing_area();
        root.fill(&WHITE)?;

        let mut ctx = ChartBuilder::on(&root)
            .set_label_area_size(LabelAreaPosition::Left, 40)
            .set_label_area_size(LabelAreaPosition::Bottom, 40)
            .caption(
                format!("Elpriser från {start_date} till {end_date}"),
                ("sans-serif", 35),
            )
            .margin(10)
            .build_cartesian_2d(start_date..end_date, min_price..(max_price + 1.0))?;

        ctx.configure_mesh().x_label_formatter(&fmt_x_axis).draw()?;

        ctx.draw_series(LineSeries::new(
            prices.iter().rev().map(|hp| (hp.start_date, hp.price)),
            &BLUE,
        ))?;

        root.present()?;
    }

    // Write to image
    let image = image::RgbImage::from_raw(width as u32, height as u32, buffer)
        .ok_or_else(|| anyhow!("Image buffer not large enough"))?;

    let mut bytes: Vec<u8> = Vec::new();
    image.write_to(
        &mut std::io::Cursor::new(&mut bytes),
        image::ImageFormat::Png,
    )?;

    Ok(bytes)
}

#[cached(result = true, time = 60)]
async fn get_latest_prices() -> Result<Vec<HourlyPrice>> {
    println!("Fetching latest sahko prices");
    let url = "https://api.porssisahko.net/v1/latest-prices.json";
    let resp: PricesResult = reqwest::get(url).await?.json().await?;

    Ok(resp.prices)
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
