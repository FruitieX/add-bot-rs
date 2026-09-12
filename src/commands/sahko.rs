use chrono::Utc;
use color_eyre::Result;
use teloxide::types::{ChatId, InputFile};

use crate::{
    commands::queue::queue_start_at,
    services::{
        leetify::{average_recent_match_duration_for_configured_players, MatchDurationEstimate},
        porssisahko::{estimate_energy_cost, get_latest_prices, EnergyCostEstimate},
    },
    settings::Settings,
    state::State,
    types::QueueId,
};
use chrono_tz::Tz;

pub async fn get_sahko_inputfile() -> Result<InputFile> {
    let price_chart_bytes = crate::services::porssisahko::get_price_chart().await?;
    let inputfile = InputFile::memory(price_chart_bytes);
    Ok(inputfile)
}

/// Creates a forecast for the active queues in one chat.
///
/// No forecast is returned when there are no queues, which keeps `/el`'s
/// existing behavior unchanged for chats that are not queuing.
pub(crate) async fn queue_cost_message(
    settings: &Settings,
    state: &State,
    chat_id: ChatId,
    tz: &Tz,
) -> Option<String> {
    let chat = state.chats.get(&chat_id)?;
    if chat.queues.is_empty() {
        return None;
    }

    let queues: Vec<(QueueId, crate::state::Queue)> = chat.queues.clone().into_iter().collect();
    let now = Utc::now().with_timezone(tz);
    let (duration, prices) = tokio::join!(
        average_recent_match_duration_for_configured_players(settings),
        get_latest_prices()
    );

    let Some(duration) = duration else {
        return Some(
            "Upcoming CS2 queue electricity forecast unavailable: Leetify did not return usable round data from the latest matches."
                .to_string(),
        );
    };

    let prices = match prices {
        Ok(prices) => prices,
        Err(error) => {
            eprintln!("Failed to fetch spot prices for queue cost forecast: {error}");
            return Some(
                "Upcoming CS2 queue electricity forecast unavailable: spot-price data could not be fetched."
                    .to_string(),
            );
        }
    };

    Some(format_queue_cost_message(
        &queues,
        now,
        tz,
        &duration,
        &prices,
        settings.electricity.gaming_pc_power_watts,
        settings.electricity.caruna_espoo_distribution_cents_per_kwh,
    ))
}

fn format_queue_cost_message(
    queues: &[(QueueId, crate::state::Queue)],
    now: chrono::DateTime<Tz>,
    tz: &Tz,
    duration: &MatchDurationEstimate,
    prices: &[crate::services::porssisahko::HourlyPrice],
    power_watts: f64,
    distribution_cents_per_kwh: f64,
) -> String {
    let mut queues = queues.to_vec();
    queues.sort_by_key(|(queue_id, queue)| queue_start_at(queue_id, queue, now).timestamp());

    let mut forecast_lines = Vec::with_capacity(queues.len());
    for (queue_id, queue) in queues {
        let start = queue_start_at(&queue_id, &queue, now);
        let start_utc = start.with_timezone(&Utc);
        let start_label = if queue_id.is_instant_queue() {
            format!("if filled now ({})", format_local_time(start, now, tz))
        } else {
            format!("scheduled for {}", format_local_time(start, now, tz))
        };

        let estimate = estimate_energy_cost(
            prices,
            start_utc,
            duration.minutes,
            power_watts,
            distribution_cents_per_kwh,
        );

        let line = match estimate {
            Ok(estimate) => format_forecast_line(&queue_id, &start_label, &estimate, duration),
            Err(error) => format!("- {queue_id} ({start_label}): unavailable ({error})"),
        };
        forecast_lines.push(line);
    }

    format!(
        "Upcoming CS2 queue electricity forecast:\n\
Estimated match duration: {:.0} minutes, based on {} of the latest {} matches with round data (average {:.1} rounds × {:.1} minutes).\n\
Assumptions: {:.0} W gaming PC; Caruna Espoo Yleissiirto {:.2} c/kWh including VAT. Porssisahko spot prices are used as returned (including VAT). The fixed monthly transfer fee and any separate electricity tax are excluded.\n\
{}",
        duration.minutes,
        duration.matches_used,
        crate::services::leetify::RECENT_MATCHES_LIMIT,
        duration.average_rounds,
        crate::services::leetify::CS2_ESTIMATED_MINUTES_PER_ROUND,
        power_watts,
        distribution_cents_per_kwh,
        forecast_lines.join("\n")
    )
}

fn format_forecast_line(
    queue_id: &QueueId,
    start_label: &str,
    estimate: &EnergyCostEstimate,
    duration: &MatchDurationEstimate,
) -> String {
    format!(
        "- {queue_id} ({start_label}): spot {spot}, Caruna {distribution}, total {total} for {:.2} kWh",
        duration.minutes,
        spot = format_eur(estimate.spot_cost_eur),
        distribution = format_eur(estimate.distribution_cost_eur),
        total = format_eur(estimate.total_cost_eur),
    )
}

fn format_eur(value: f64) -> String {
    format!("€{value:.2}")
}

fn format_local_time(value: chrono::DateTime<Tz>, now: chrono::DateTime<Tz>, tz: &Tz) -> String {
    let value = value.with_timezone(tz);
    if value.date_naive() == now.date_naive() {
        value.format("%H:%M").to_string()
    } else {
        value.format("%d.%m %H:%M").to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveTime, TimeZone};

    #[test]
    fn queue_cost_message_contains_timing_and_cost_breakdown() {
        let tz = chrono_tz::Europe::Helsinki;
        let now = tz.with_ymd_and_hms(2026, 9, 12, 18, 0, 0).unwrap();
        let start = tz.with_ymd_and_hms(2026, 9, 12, 19, 30, 0).unwrap();
        let queue_id = QueueId::new("19:30".to_string());
        let queue = crate::state::Queue::new(
            NaiveTime::from_hms_opt(19, 30, 0).unwrap(),
            "/1930".to_string(),
        );
        let prices = vec![
            crate::services::porssisahko::HourlyPrice {
                price: 10.0,
                start_date: start.with_timezone(&Utc),
            },
            crate::services::porssisahko::HourlyPrice {
                price: 10.0,
                start_date: (start + chrono::Duration::minutes(15)).with_timezone(&Utc),
            },
        ];
        let duration = MatchDurationEstimate {
            matches_used: 30,
            average_rounds: 24.0,
            minutes: 30.0,
        };

        let message = format_queue_cost_message(
            &[(queue_id, queue)],
            now,
            &tz,
            &duration,
            &prices,
            1_000.0,
            2.0,
        );

        assert!(message.contains("Upcoming CS2 queue electricity forecast"));
        assert!(message.contains("scheduled for 19:30"));
        assert!(message.contains("spot €0.05"));
        assert!(message.contains("Caruna €0.01"));
        assert!(message.contains("total €0.06"));
    }
}
