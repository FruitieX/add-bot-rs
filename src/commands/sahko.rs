use crate::{
    commands::queue::queue_start_at,
    services::{
        chart_style::ChartPanel,
        leetify::{average_recent_match_duration_for_configured_players, MatchDurationEstimate},
        porssisahko::{estimate_energy_cost, get_latest_prices, render_price_chart, HourlyPrice},
    },
    settings::Settings,
    state::{Queue, State},
    types::QueueId,
};
use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use color_eyre::Result;
use teloxide::types::{ChatId, InputFile};

/// Render the price plot and this chat's queue estimates as a single image.
pub async fn get_sahko_inputfile(
    settings: &Settings,
    state: &State,
    chat_id: ChatId,
    tz: &Tz,
) -> Result<InputFile> {
    let queues = state
        .chats
        .get(&chat_id)
        .map(|chat| chat.queues.clone().into_iter().collect::<Vec<_>>())
        .unwrap_or_default();
    let duration = async {
        if queues.is_empty() {
            None
        } else {
            average_recent_match_duration_for_configured_players(settings).await
        }
    };
    let (prices, duration) = tokio::join!(get_latest_prices(), duration);
    let prices = prices?;
    let now = Utc::now();
    let panel = (!queues.is_empty()).then(|| {
        queue_cost_panel(
            &queues,
            now.with_timezone(tz),
            &prices,
            duration.as_ref(),
            settings.electricity.gaming_pc_power_watts,
            settings.electricity.caruna_espoo_distribution_cents_per_kwh,
        )
    });
    Ok(InputFile::memory(render_price_chart(
        &prices,
        now,
        *tz,
        panel.as_ref(),
    )?))
}

fn queue_cost_panel(
    queues: &[(QueueId, Queue)],
    now: DateTime<Tz>,
    prices: &[HourlyPrice],
    duration: Option<&MatchDurationEstimate>,
    power_watts: f64,
    distribution_cents_per_kwh: f64,
) -> ChartPanel {
    let mut queues = queues.to_vec();
    queues.sort_by_key(|(id, queue)| (queue_start_at(id, queue, now).timestamp(), id.to_string()));
    let rows = queues
        .into_iter()
        .map(|(id, queue)| {
            let start = queue_start_at(&id, &queue, now);
            let label = if id.is_instant_queue() {
                "Instant queue · If filled now".into()
            } else {
                let day = if start.date_naive() == now.date_naive() {
                    "Today".into()
                } else if Some(start.date_naive()) == now.date_naive().succ_opt() {
                    "Tomorrow".into()
                } else {
                    start.format("%-d %b").to_string()
                };
                format!("{day} {}", start.format("%H:%M"))
            };
            let cost = duration
                .and_then(|duration| {
                    estimate_energy_cost(
                        prices,
                        start.with_timezone(&Utc),
                        duration.minutes,
                        power_watts,
                        distribution_cents_per_kwh,
                    )
                    .map_err(|error| {
                        eprintln!("Queue {id} electricity estimate unavailable: {error}");
                        error
                    })
                    .ok()
                })
                .map(|estimate| format!("€{:.2}", estimate.total_cost_eur))
                .unwrap_or_else(|| "Unavailable".into());
            (label, cost)
        })
        .collect();
    let notes = match duration {
        Some(duration) => vec![format!("{power_watts:.0} W PC · Estimated {:.0}-minute match from {} recent matches' rounds", duration.minutes, duration.matches_used),
            "Includes spot energy + variable transfer; excludes fixed fees and electricity tax".into(),
            "Unavailable = missing price coverage or unusable match-estimate inputs".into()],
        None => vec!["Estimates unavailable: no usable round data in recent match history".into()],
    };
    let mut panel = ChartPanel { rows, notes };
    if duration.is_some() && panel.rows.iter().all(|(_, cost)| cost != "Unavailable") {
        panel.notes.pop();
    }
    panel
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, NaiveTime, TimeZone};
    #[test]
    fn queue_panel_labels_per_pc_cost_timing_and_assumptions() {
        let tz = chrono_tz::Europe::Helsinki;
        let now = tz.with_ymd_and_hms(2026, 9, 30, 18, 0, 0).unwrap();
        let start = now + Duration::minutes(90);
        let queue = Queue::new(NaiveTime::from_hms_opt(19, 30, 0).unwrap(), "/1930".into());
        let queues = vec![(QueueId::new("19:30".into()), queue)];
        let prices = (0..2)
            .map(|i| HourlyPrice {
                price: 10.0,
                start_date: (start + Duration::minutes(i * 15)).with_timezone(&Utc),
            })
            .collect::<Vec<_>>();
        let duration = MatchDurationEstimate {
            matches_used: 30,
            average_rounds: 15.0,
            minutes: 30.0,
        };
        let panel = queue_cost_panel(&queues, now, &prices, Some(&duration), 1000.0, 2.0);
        assert_eq!(panel.rows, vec![("Today 19:30".into(), "€0.06".into())]);
        assert!(panel.notes[0].contains("1000 W PC · Estimated 30-minute match"));
        assert_eq!(panel.notes.len(), 2);
        let panel = queue_cost_panel(&queues, now, &[], Some(&duration), 500.0, 2.0);
        assert_eq!(panel.rows[0].1, "Unavailable");
        assert!(panel.notes[2].contains("missing price coverage"));
        let panel = queue_cost_panel(&queues, now, &prices, None, 500.0, 2.0);
        assert_eq!(panel.rows[0].1, "Unavailable");
        assert!(panel.notes[0].contains("no usable round data"));
        let instant = vec![(QueueId::new(String::new()), queues[0].1.clone())];
        let panel = queue_cost_panel(&instant, now, &prices, None, 500.0, 2.0);
        assert_eq!(panel.rows[0].0, "Instant queue · If filled now");
        let panel = queue_cost_panel(&queues, now + Duration::hours(3), &prices, None, 500.0, 2.0);
        assert_eq!(panel.rows[0].0, "Tomorrow 19:30");
    }
}
