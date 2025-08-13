use crate::services::weather::{format_temperature_line, format_weather_report};

/// Build a user-friendly error message for weather commands.
/// If weather is not configured, we tell the user how to enable it.
/// Otherwise, return a generic fallback.
fn friendly_weather_error_message(error: &str, fallback: &str) -> String {
    let chain_lower = error.to_lowercase();

    // Detect configuration-related issues
    if chain_lower.contains("weather not configured")
        || chain_lower.contains("display_name is empty")
        || chain_lower.contains("settings")
    {
        return "Weather is not configured. Ask an admin to set the [weather] section with latitude, longitude, and display_name in Settings.toml".to_string();
    }

    // Default to a generic fallback
    fallback.to_string()
}

/// Returns a short temperature line for the configured location.
/// Example: "Location Name now: 7.3°C (cloudy)."
pub async fn temperature() -> String {
    match format_temperature_line().await {
        Ok(text) => text,
        Err(e) => {
            eprintln!("Failed to fetch temperature from met.no: {e:?}");
            friendly_weather_error_message(&format!("{e:?}"), "Failed to fetch temperature")
        }
    }
}

/// Returns a more detailed weather report for the configured location.
/// Includes temperature, wind, humidity, clouds, pressure, and short-term precipitation.
pub async fn weather() -> String {
    match format_weather_report().await {
        Ok(text) => text,
        Err(e) => {
            eprintln!("Failed to fetch weather from met.no: {e:?}");
            friendly_weather_error_message(&format!("{e:?}"), "Failed to fetch weather")
        }
    }
}
