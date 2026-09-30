//! Failures whose cause can be explained without exposing internal diagnostics.
use crate::{types::Username, util::escape_html};
use color_eyre::eyre::Report;
use std::fmt;

#[derive(Debug)]
pub(crate) enum DataFailure {
    Unlinked(Username),
    NoPlayers,
    NoPrices,
    NoMatches(Option<Username>),
    PrivateProfile,
    WeatherNotConfigured,
    NoForecast,
    NoSquadMatch,
    IncompleteRosters,
    Upstream {
        service: &'static str,
        cause: RequestFailure,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RequestFailure {
    Connection,
    Timeout,
    RateLimited,
    NotFound,
    AccessDenied,
    Service,
    InvalidResponse,
}

impl fmt::Display for DataFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for DataFailure {}

pub(crate) fn upstream(error: Report, service: &'static str) -> Report {
    eprintln!("{service} request failed: {error:?}");
    let cause = error
        .downcast_ref::<reqwest::Error>()
        .map(|error| {
            if error.is_timeout() {
                RequestFailure::Timeout
            } else if error.is_connect() {
                RequestFailure::Connection
            } else {
                match error.status().map(|status| status.as_u16()) {
                    Some(429) => RequestFailure::RateLimited,
                    Some(404) => RequestFailure::NotFound,
                    Some(401 | 403) => RequestFailure::AccessDenied,
                    Some(_) => RequestFailure::Service,
                    None => RequestFailure::InvalidResponse,
                }
            }
        })
        .unwrap_or(RequestFailure::InvalidResponse);
    DataFailure::Upstream { service, cause }.into()
}

pub(crate) fn message(error: &Report, label: &str, retry: &str) -> String {
    let label = escape_html(label);
    let retry = escape_html(retry);
    match error.downcast_ref::<DataFailure>() {
        Some(DataFailure::Unlinked(name)) => format!(
            "{} doesn’t have a Steam account linked. Ask an admin to link it.",
            escape_html(&name.to_string())
        ),
        Some(DataFailure::NoPlayers) => {
            "No players are configured. Ask an admin to link the squad’s Steam accounts.".into()
        }
        Some(DataFailure::NoPrices) => {
            "No electricity price data is available right now. Try /el again shortly.".into()
        }
        Some(DataFailure::NoMatches(Some(name))) => format!(
            "No recorded matches found for {}.",
            escape_html(&name.to_string())
        ),
        Some(DataFailure::NoMatches(None)) => {
            "No recorded matches found for the configured players.".into()
        }
        Some(DataFailure::PrivateProfile) => {
            "This Leetify profile is private. Make it public to view its stats.".into()
        }
        Some(DataFailure::WeatherNotConfigured) => {
            "Weather isn’t configured. Ask an admin to set a location.".into()
        }
        Some(DataFailure::NoForecast) => {
            "No weather forecast is available for this location.".into()
        }
        Some(DataFailure::NoSquadMatch) => {
            "No verified squad match found in the available history.".into()
        }
        Some(DataFailure::IncompleteRosters) => format!(
            "{label} unavailable: match rosters couldn’t be verified. Try {retry} again shortly."
        ),
        Some(DataFailure::Upstream { service, cause }) => {
            let reason = match cause {
                RequestFailure::Connection => format!("{service} couldn’t be reached"),
                RequestFailure::Timeout => format!("{service} took too long to respond"),
                RequestFailure::RateLimited => format!("{service} is limiting requests"),
                RequestFailure::NotFound => format!("{service} couldn’t find the requested data"),
                RequestFailure::AccessDenied => {
                    format!("{service} denied access to the requested data")
                }
                RequestFailure::Service => format!("{service} returned a service error"),
                RequestFailure::InvalidResponse => {
                    format!("{service} returned an unreadable response")
                }
            };
            if matches!(
                cause,
                RequestFailure::NotFound | RequestFailure::AccessDenied
            ) {
                format!("{reason}.")
            } else {
                format!("{reason}. Try {retry} again shortly.")
            }
        }
        None => format!("{label} couldn’t be generated. Try {retry} again shortly."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn http_status_and_decode_errors_keep_their_actual_causes() {
        use std::io::{Read, Write};
        for (status, expected) in [
            (403, RequestFailure::AccessDenied),
            (404, RequestFailure::NotFound),
            (429, RequestFailure::RateLimited),
            (503, RequestFailure::Service),
            (200, RequestFailure::InvalidResponse),
        ] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let server = std::thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(std::time::Duration::from_secs(3)))
                    .unwrap();
                let mut request = [0; 4096];
                socket.read(&mut request).unwrap();
                write!(
                    socket,
                    "HTTP/1.1 {status} Test\r\nContent-Length: 1\r\nConnection: close\r\n\r\nx"
                )
                .unwrap();
            });
            let response = reqwest::Client::builder()
                .no_proxy()
                .timeout(std::time::Duration::from_secs(3))
                .build()
                .unwrap()
                .get(format!("http://{address}"))
                .send()
                .await
                .unwrap();
            let error = match response.error_for_status() {
                Err(error) => error,
                Ok(response) => response.json::<serde_json::Value>().await.unwrap_err(),
            };
            let error = upstream(error.into(), "Leetify");
            assert!(
                matches!(error.downcast_ref::<DataFailure>(),Some(DataFailure::Upstream{cause,..}) if *cause==expected)
            );
            let message = message(&error, "Results", "/results");
            assert!(!message.contains("No recorded matches"));
            assert!(!message.contains("127.0.0.1"));
            server.join().unwrap();
        }
    }
    #[test]
    fn failures_keep_empty_unlinked_private_and_upstream_cases_distinct() {
        let name = Username::new("A&B".into());
        assert_eq!(
            message(
                &DataFailure::Unlinked(name.clone()).into(),
                "Results",
                "/results"
            ),
            "A&amp;B doesn’t have a Steam account linked. Ask an admin to link it."
        );
        assert_eq!(
            message(
                &DataFailure::NoMatches(Some(name)).into(),
                "Results",
                "/results"
            ),
            "No recorded matches found for A&amp;B."
        );
        assert!(
            message(&DataFailure::PrivateProfile.into(), "Stats", "/stats").contains("private")
        );
        assert_eq!(
            message(
                &DataFailure::Upstream {
                    service: "Leetify",
                    cause: RequestFailure::RateLimited
                }
                .into(),
                "Results",
                "/results"
            ),
            "Leetify is limiting requests. Try /results again shortly."
        );
        let generic = color_eyre::eyre::eyre!("sensitive internal detail");
        let text = message(&generic, "Results", "/results");
        assert!(!text.contains("sensitive"));
        assert!(!text.contains("No recorded matches"));
    }
}
