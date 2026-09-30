use crate::services::weather::{format_temperature_line, format_weather_report};
use chrono_tz::Tz;

/// Returns a short temperature line for the configured location.
/// Example: "Location Name now: 7.3°C (cloudy)."
pub async fn temperature(tz: Tz) -> String {
    match format_temperature_line(tz).await {
        Ok(text) => text,
        Err(e) => {
            eprintln!("Failed to fetch temperature from met.no: {e:?}");
            crate::services::failure::message(&e, "Temperature", "/temperature")
        }
    }
}

/// Returns a more detailed weather report for the configured location.
/// Includes temperature, wind/gusts, near-term conditions and precipitation, and tomorrow’s high.
pub async fn weather(tz: Tz) -> String {
    match format_weather_report(tz).await {
        Ok(text) => text,
        Err(e) => {
            eprintln!("Failed to fetch weather from met.no: {e:?}");
            crate::services::failure::message(&e, "Weather", "/weather")
        }
    }
}
