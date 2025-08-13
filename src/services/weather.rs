use cached::proc_macro::cached;
use chrono::{DateTime, Utc};
use color_eyre::{eyre::eyre, Result};
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, USER_AGENT};
use serde::Deserialize;

/// Weather location is configured via Settings.weather; if missing, weather commands are disabled.
/// We require latitude, longitude and display_name to be present when the section exists.
fn get_weather_config() -> Result<(f64, f64, String)> {
    let settings = crate::settings::read_settings()?;
    let w = settings
        .weather
        .ok_or_else(|| eyre!("Weather not configured"))?;

    let name = w.display_name.trim().to_string();
    if name.is_empty() {
        return Err(eyre!("Weather display_name is empty"));
    }

    Ok((w.latitude, w.longitude, name))
}

// Use the complete endpoint to get full variables (incl. wind gust in instant)
const METNO_URL: &str = "https://api.met.no/weatherapi/locationforecast/2.0/complete";

#[derive(Debug, Clone, Deserialize)]
pub struct Forecast {
    pub properties: Properties,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Properties {
    pub timeseries: Vec<TimeSeries>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TimeSeries {
    pub time: DateTime<Utc>,
    pub data: Data,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct Data {
    pub instant: Instant,
    #[serde(default)]
    pub next_1_hours: Option<Period>,
    #[serde(default)]
    pub next_6_hours: Option<Period>,
    #[serde(default)]
    pub next_12_hours: Option<Period>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Instant {
    pub details: InstantDetails,
}

#[derive(Debug, Clone, Deserialize)]
pub struct InstantDetails {
    #[serde(default)]
    pub air_temperature: f32,
    #[serde(default)]
    pub wind_speed: Option<f32>,
    #[serde(default)]
    pub wind_speed_of_gust: Option<f32>,
    #[serde(default)]
    pub wind_from_direction: Option<f32>,
    #[serde(default)]
    pub relative_humidity: Option<f32>,
    #[serde(default)]
    pub cloud_area_fraction: Option<f32>,
    #[serde(default)]
    pub air_pressure_at_sea_level: Option<f32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Period {
    #[serde(default)]
    pub summary: Option<PeriodSummary>,
    #[serde(default)]
    pub details: Option<PeriodDetails>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PeriodSummary {
    #[serde(default)]
    pub symbol_code: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct PeriodDetails {
    #[serde(default)]
    pub precipitation_amount: Option<f32>,

    #[serde(default)]
    pub precipitation_amount_min: Option<f32>,

    #[serde(default)]
    pub precipitation_amount_max: Option<f32>,

    #[serde(default)]
    pub probability_of_precipitation: Option<f32>,

    #[serde(default)]
    pub wind_speed_of_gust: Option<f32>,
}

#[allow(dead_code)]
/// A simplified observation extracted from the forecast
#[derive(Debug, Clone)]
pub struct Observation {
    pub observed_at: DateTime<Utc>,
    pub air_temperature_c: f32,
    pub wind_speed_ms: Option<f32>,
    pub wind_gust_ms: Option<f32>,
    pub wind_from_dir_deg: Option<f32>,
    pub rel_humidity_pc: Option<f32>,
    pub cloud_cover_pc: Option<f32>,
    pub pressure_hpa: Option<f32>,

    // Short-term forecast
    pub next_1h_symbol: Option<String>,
    pub next_1h_precip: Option<f32>,

    // Backup longer periods if 1h isn't available
    pub next_6h_symbol: Option<String>,
    pub next_6h_precip: Option<f32>,
}

fn default_user_agent() -> HeaderValue {
    // As recommended by api.met.no, include identification and a way to reach you.
    HeaderValue::from_static("add-bot-rs/0.12.1 (+https://github.com/fruitiex/add-bot-rs)")
}

fn build_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(USER_AGENT, default_user_agent());
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    headers
}

fn pick_relevant_series(series: &[TimeSeries], now: DateTime<Utc>) -> Option<&TimeSeries> {
    if series.is_empty() {
        return None;
    }

    // Choose the entry with the greatest time <= now, or fallback to the first
    // entry if all are in the future.
    let mut best_idx: Option<usize> = None;
    for (i, item) in series.iter().enumerate() {
        if item.time <= now {
            match best_idx {
                None => best_idx = Some(i),
                Some(j) if series[j].time < item.time => best_idx = Some(i),
                _ => {}
            }
        }
    }

    if let Some(idx) = best_idx {
        series.get(idx)
    } else {
        // Fallback: pick the earliest future entry
        series.first()
    }
}

fn extract_observation(ts: &TimeSeries) -> Observation {
    let details = &ts.data.instant.details;

    let (next_1h_symbol, next_1h_precip) = ts
        .data
        .next_1_hours
        .as_ref()
        .map(|p| {
            (
                p.summary.as_ref().map(|s| s.symbol_code.clone()),
                p.details
                    .as_ref()
                    .and_then(|d| d.precipitation_amount)
                    .or_else(|| {
                        // If not present, consider min/max average if both exist
                        match (
                            p.details.as_ref()?.precipitation_amount_min,
                            p.details.as_ref()?.precipitation_amount_max,
                        ) {
                            (Some(min), Some(max)) => Some((min + max) / 2.0),
                            _ => None,
                        }
                    }),
            )
        })
        .unwrap_or((None, None));

    let (next_6h_symbol, next_6h_precip) = ts
        .data
        .next_6_hours
        .as_ref()
        .map(|p| {
            (
                p.summary.as_ref().map(|s| s.symbol_code.clone()),
                p.details
                    .as_ref()
                    .and_then(|d| d.precipitation_amount)
                    .or_else(|| {
                        match (
                            p.details.as_ref()?.precipitation_amount_min,
                            p.details.as_ref()?.precipitation_amount_max,
                        ) {
                            (Some(min), Some(max)) => Some((min + max) / 2.0),
                            _ => None,
                        }
                    }),
            )
        })
        .unwrap_or((None, None));

    Observation {
        observed_at: ts.time,
        air_temperature_c: details.air_temperature,
        wind_speed_ms: details.wind_speed,
        wind_gust_ms: details.wind_speed_of_gust,
        wind_from_dir_deg: details.wind_from_direction,
        rel_humidity_pc: details.relative_humidity,
        cloud_cover_pc: details.cloud_area_fraction,
        pressure_hpa: details.air_pressure_at_sea_level,
        next_1h_symbol,
        next_1h_precip,
        next_6h_symbol,
        next_6h_precip,
    }
}

fn deg_to_cardinal(deg: f32) -> &'static str {
    // 16-wind compass
    let dirs = [
        "N", "NNE", "NE", "ENE", "E", "ESE", "SE", "SSE", "S", "SSW", "SW", "WSW", "W", "WNW",
        "NW", "NNW",
    ];
    let idx = ((deg / 22.5) + 0.5).floor() as i32 % 16;
    let idx = if idx < 0 { idx + 16 } else { idx } as usize;
    dirs[idx]
}

fn fmt_precip(mm: Option<f32>) -> String {
    match mm {
        Some(v) => format!("{:.1} mm", v),
        None => "N/A".to_string(),
    }
}

fn fmt_wind(speed: Option<f32>, dir_deg: Option<f32>) -> String {
    match (speed, dir_deg) {
        (Some(s), Some(d)) => format!("{:.1} m/s {}", s, deg_to_cardinal(d)),
        (Some(s), None) => format!("{:.1} m/s", s),
        _ => "N/A".to_string(),
    }
}

/// Extract tomorrow's forecast data (24-48 hours from now)
fn get_tomorrow_forecast(series: &[TimeSeries], now: DateTime<Utc>) -> Vec<&TimeSeries> {
    let tomorrow_start = now + chrono::Duration::hours(24);
    let tomorrow_end = now + chrono::Duration::hours(48);

    series
        .iter()
        .filter(|ts| ts.time >= tomorrow_start && ts.time < tomorrow_end)
        .collect()
}

/// Find tomorrow's maximum temperature and corresponding observation
fn get_tomorrow_max_temp(series: &[TimeSeries], now: DateTime<Utc>) -> Option<(f32, &TimeSeries)> {
    let tomorrow_entries = get_tomorrow_forecast(series, now);

    tomorrow_entries
        .into_iter()
        .map(|ts| (ts.data.instant.details.air_temperature, ts))
        .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
}

/// Map MET/Yr symbol_code to a human-friendly English description.
///
/// Reference:
/// - Locationforecast docs (Weather icons section): https://api.met.no/weatherapi/locationforecast/2.0/documentation
/// - Symbol list with descriptions: https://nrkno.github.io/yr-weather-symbols/
fn symbol_description(code: &str) -> String {
    // Normalize by removing day/night/polartwilight suffixes
    let base = code
        .strip_suffix("_day")
        .or_else(|| code.strip_suffix("_night"))
        .or_else(|| code.strip_suffix("_polartwilight"))
        .unwrap_or(code);

    let desc = match base {
        "clearsky" => "clear sky",
        "fair" => "fair",
        "partlycloudy" => "partly cloudy",
        "cloudy" => "cloudy",
        "lightrain" => "light rain",
        "rain" => "rain",
        "heavyrain" => "heavy rain",
        "lightrainandthunder" => "light rain and thunder",
        "rainandthunder" => "rain and thunder",
        "heavyrainandthunder" => "heavy rain and thunder",
        "lightsleet" => "light sleet",
        "sleet" => "sleet",
        "heavysleet" => "heavy sleet",
        "lightsleetandthunder" => "light sleet and thunder",
        "sleetandthunder" => "sleet and thunder",
        "heavysleetandthunder" => "heavy sleet and thunder",
        "lightsnow" => "light snow",
        "snow" => "snow",
        "heavysnow" => "heavy snow",
        "lightsnowandthunder" => "light snow and thunder",
        "snowandthunder" => "snow and thunder",
        "heavysnowandthunder" => "heavy snow and thunder",
        "rainshowers" => "rain showers",
        "heavyrainshowers" => "heavy rain showers",
        "lightrainshowers" => "light rain showers",
        "rainshowersandthunder" => "rain showers and thunder",
        "sleetshowers" => "sleet showers",
        "heavysleetshowers" => "heavy sleet showers",
        "lightsleetshowers" => "light sleet showers",
        "sleetshowersandthunder" => "sleet showers and thunder",
        "snowshowers" => "snow showers",
        "heavysnowshowers" => "heavy snow showers",
        "lightsnowshowers" => "light snow showers",
        "snowshowersandthunder" => "snow showers and thunder",
        // Typo kept for backward compatibility, per MET docs note
        "lightssleetshowersandthunder" => "light sleet showers and thunder",
        "lightssnowshowersandthunder" => "light snow showers and thunder",
        "fog" => "fog",
        _ => return base.replace('_', " "),
    };

    desc.to_string()
}

fn fmt_symbol(symbol: &Option<String>) -> String {
    match symbol {
        Some(code) => symbol_description(code),
        None => "N/A".to_string(),
    }
}

/// Fetch and cache the forecast for 1 hour.
///
/// The met.no API guidelines suggest reasonable caching, and 1 hour granularity
/// is a practical compromise for a chat bot while staying within rate limits.
#[cached(time = 3600, result = true)]
pub async fn get_forecast() -> Result<Forecast> {
    let mut url =
        reqwest::Url::parse(METNO_URL).map_err(|e| eyre!("Invalid met.no endpoint URL: {e}"))?;
    let (lat, lon, _label) = get_weather_config()?;
    url.query_pairs_mut()
        .append_pair("lat", &format!("{:.4}", lat))
        .append_pair("lon", &format!("{:.4}", lon));

    let client = reqwest::Client::builder()
        .default_headers(build_headers())
        .build()?;

    let res = client.get(url).send().await?;
    if !res.status().is_success() {
        return Err(eyre!(
            "met.no responded with status {}",
            res.status().as_u16()
        ));
    }

    let forecast = res.json::<Forecast>().await?;
    Ok(forecast)
}

/// Helper for "/temperature" command
/// NOTE: This now maps symbol code to human-readable description.
pub async fn format_temperature_line() -> Result<String> {
    let forecast = get_forecast().await?;
    let now = Utc::now();

    let Some(ts) = pick_relevant_series(&forecast.properties.timeseries, now) else {
        return Err(eyre!("No timeseries data available from met.no"));
    };

    let obs = extract_observation(ts);
    let symbol_desc = obs
        .next_1h_symbol
        .as_ref()
        .or(obs.next_6h_symbol.as_ref())
        .map(|code| symbol_description(code))
        .unwrap_or_else(|| "unknown".to_string());

    let (_, _, label) = get_weather_config()?;

    // Get tomorrow's max temperature
    let tomorrow_info = get_tomorrow_max_temp(&forecast.properties.timeseries, now)
        .map(|(temp, ts)| {
            let tomorrow_obs = extract_observation(ts);
            let tomorrow_desc = tomorrow_obs
                .next_1h_symbol
                .as_ref()
                .or(tomorrow_obs.next_6h_symbol.as_ref())
                .map(|code| symbol_description(code))
                .unwrap_or_else(|| "unknown".to_string());
            format!(" Tomorrow max: {:.1}°C ({}).", temp, tomorrow_desc)
        })
        .unwrap_or_else(|| " Tomorrow: N/A.".to_string());

    Ok(format!(
        "{label} now: {:.1}°C ({}).{}",
        obs.air_temperature_c, symbol_desc, tomorrow_info
    ))
}

/// Helper for "/weather" command with a bit more detail
pub async fn format_weather_report() -> Result<String> {
    let forecast = get_forecast().await?;
    let now = Utc::now();

    let Some(ts) = pick_relevant_series(&forecast.properties.timeseries, now) else {
        return Err(eyre!("No timeseries data available from met.no"));
    };

    let obs = extract_observation(ts);

    let gust = obs
        .wind_gust_ms
        .map(|g| format!("{:.1} m/s", g))
        .unwrap_or_else(|| "N/A".to_string());

    let wind = fmt_wind(obs.wind_speed_ms, obs.wind_from_dir_deg);
    let rh = obs
        .rel_humidity_pc
        .map(|h| format!("{:.0}%", h))
        .unwrap_or_else(|| "N/A".to_string());
    let clouds = obs
        .cloud_cover_pc
        .map(|c| format!("{:.0}%", c))
        .unwrap_or_else(|| "N/A".to_string());
    let pressure = obs
        .pressure_hpa
        .map(|p| format!("{:.0} hPa", p))
        .unwrap_or_else(|| "N/A".to_string());

    // Get temperature and weather info for next 1h and 6h
    let (temp_1h, sym_1h) = {
        let next_1h = now + chrono::Duration::hours(1);
        forecast
            .properties
            .timeseries
            .iter()
            .filter(|ts| ts.time >= next_1h && ts.time < next_1h + chrono::Duration::hours(1))
            .min_by_key(|ts| (ts.time - next_1h).num_seconds().abs())
            .map(|ts| {
                let obs = extract_observation(ts);
                (
                    ts.data.instant.details.air_temperature,
                    fmt_symbol(&obs.next_1h_symbol.or(obs.next_6h_symbol)),
                )
            })
            .unwrap_or((obs.air_temperature_c, fmt_symbol(&obs.next_1h_symbol)))
    };
    let precip_1h = fmt_precip(obs.next_1h_precip);

    let (temp_6h, sym_6h) = {
        let next_6h = now + chrono::Duration::hours(6);
        forecast
            .properties
            .timeseries
            .iter()
            .filter(|ts| ts.time >= next_6h && ts.time < next_6h + chrono::Duration::hours(1))
            .min_by_key(|ts| (ts.time - next_6h).num_seconds().abs())
            .map(|ts| {
                let obs = extract_observation(ts);
                (
                    ts.data.instant.details.air_temperature,
                    fmt_symbol(&obs.next_6h_symbol.or(obs.next_1h_symbol)),
                )
            })
            .unwrap_or((obs.air_temperature_c, fmt_symbol(&obs.next_6h_symbol)))
    };
    let precip_6h = fmt_precip(obs.next_6h_precip);

    // Get tomorrow's max temperature
    let tomorrow_info = get_tomorrow_max_temp(&forecast.properties.timeseries, now)
        .map(|(temp, ts)| {
            let tomorrow_obs = extract_observation(ts);
            let tomorrow_desc = tomorrow_obs
                .next_1h_symbol
                .as_ref()
                .or(tomorrow_obs.next_6h_symbol.as_ref())
                .map(|code| symbol_description(code))
                .unwrap_or_else(|| "unknown".to_string());
            format!("Tomorrow max: {:.1}°C ({})", temp, tomorrow_desc)
        })
        .unwrap_or_else(|| "Tomorrow: N/A".to_string());

    let (_, _, label) = get_weather_config()?;

    Ok(format!(
        "Weather for {label}\n\
         - Now: {:.1}°C\n\
         - Wind: {wind} (gusts: {gust})\n\
         - Humidity: {rh}\n\
         - Cloud cover: {clouds}\n\
         - Pressure: {pressure}\n\
         - Next 1h: {:.1}°C, {sym_1h}, precip: {precip_1h}\n\
         - Next 6h: {:.1}°C, {sym_6h}, precip: {precip_6h}\n\
         - {tomorrow_info}",
        obs.air_temperature_c, temp_1h, temp_6h
    ))
}
