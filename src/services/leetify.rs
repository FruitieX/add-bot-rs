use super::failure::{self, DataFailure};
use cached::proc_macro::cached;
use chrono::{DateTime, NaiveDate, Utc};
use chrono_tz::Tz;
use color_eyre::{eyre::eyre, Result};
use futures::StreamExt;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fmt::Display,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering as AtomicOrdering},
    time::Duration,
};

use crate::{
    settings::Settings,
    types::{SteamID, Username},
};

const LEETIFY_API_BASE_URL: &str = "https://api-public.cs-prod.leetify.com";
const MATCH_HYDRATION_MAX_ATTEMPTS: usize = 3;
const MATCH_HYDRATION_MAX_RETRY_DELAY: Duration = Duration::from_secs(30);
static MATCH_CACHE_TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
enum MatchHydrationError {
    RateLimited {
        message: String,
        retry_after: Option<Duration>,
    },
    Transient(String),
    Fatal(String),
}

impl MatchHydrationError {
    fn is_retryable(&self) -> bool {
        matches!(self, Self::RateLimited { .. } | Self::Transient(_))
    }

    fn retry_after(&self) -> Option<Duration> {
        match self {
            Self::RateLimited { retry_after, .. } => *retry_after,
            Self::Transient(_) | Self::Fatal(_) => None,
        }
    }
}

impl Display for MatchHydrationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RateLimited { message, .. } | Self::Transient(message) | Self::Fatal(message) => {
                formatter.write_str(message)
            }
        }
    }
}

impl std::error::Error for MatchHydrationError {}

fn unwrap_or_log<T, E: Display>(result: std::result::Result<T, E>, err_context: &str) -> Option<T> {
    match result {
        Ok(value) => Some(value),
        Err(e) => {
            eprintln!("{err_context}: {e}");
            None
        }
    }
}

#[derive(Clone)]
struct LeetifyClient {
    client: reqwest::Client,
    base_url: String,
    api_key: Option<String>,
}

impl LeetifyClient {
    fn from_settings(settings: &Settings) -> Self {
        let api_key = settings
            .leetify
            .as_ref()
            .and_then(|config| config.api_key.clone())
            .filter(|key| !key.trim().is_empty());

        Self {
            client: reqwest::Client::new(),
            base_url: LEETIFY_API_BASE_URL.to_string(),
            api_key,
        }
    }

    async fn get<T: DeserializeOwned>(&self, path: &str, steam_id: &SteamID) -> Result<T> {
        let url = format!("{}{path}", self.base_url.trim_end_matches('/'));
        let mut request = self
            .client
            .get(&url)
            .query(&[("steam64_id", steam_id.to_string())]);

        if let Some(api_key) = &self.api_key {
            request = request.header("_leetify_key", api_key);
        }

        Ok(request.send().await?.error_for_status()?.json().await?)
    }

    #[cfg(test)]
    async fn get_path<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        Ok(self
            .get_path_response(path)
            .await?
            .error_for_status()?
            .json()
            .await?)
    }

    async fn get_path_response(&self, path: &str) -> Result<reqwest::Response> {
        let url = format!("{}{path}", self.base_url.trim_end_matches('/'));
        let mut request = self.client.get(&url);

        if let Some(api_key) = &self.api_key {
            request = request.header("_leetify_key", api_key);
        }

        Ok(request.send().await?)
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PublicProfile {
    pub privacy_mode: String,
    pub total_matches: u32,
    pub ranks: PublicRanks,
    pub rating: PublicRating,
    pub recent_matches: Vec<PublicRecentMatch>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PublicRanks {
    pub leetify: Option<f32>,
    pub premier: Option<f32>,
    pub faceit: Option<f32>,
    pub faceit_elo: Option<f32>,
    pub wingman: Option<f32>,
    pub renown: Option<f32>,
    #[serde(default)]
    pub competitive: Vec<PublicCompetitiveRank>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PublicCompetitiveRank {
    pub map_name: String,
    pub rank: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PublicRating {
    pub aim: f32,
    pub positioning: f32,
    pub utility: f32,
    pub clutch: f32,
    pub opening: f32,
    pub ct_leetify: f32,
    pub t_leetify: f32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PublicRecentMatch {
    pub outcome: MatchResult,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
struct PublicMatch {
    id: String,
    finished_at: DateTime<Utc>,
    map_name: String,
    team_scores: Vec<PublicTeamScore>,
    stats: Vec<PublicPlayerMatchStats>,
}

#[derive(Debug, Clone, Deserialize)]
struct PublicTeamScore {
    team_number: u32,
    score: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
struct PublicPlayerMatchStats {
    steam64_id: String,
    initial_team_number: u32,
    #[serde(default)]
    flashbang_hit_friend: u32,
    #[serde(default)]
    flashbang_thrown: u32,
    #[serde(default)]
    rounds_count: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct LeetifyMatchTeam {
    pub steam64_ids: Vec<SteamID>,
    pub score: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct LeetifyMatch {
    pub id: String,
    pub game_finished_at: DateTime<Utc>,
    pub map_name: String,
    pub teams: Vec<LeetifyMatchTeam>,
}

#[derive(Debug, Default)]
pub(crate) struct LeetifyMatchHydration {
    pub matches: HashMap<String, LeetifyMatch>,
    pub failed_match_ids: HashSet<String>,
}

async fn get_leetify_profile(settings: &Settings, steam_id: &SteamID) -> Result<PublicProfile> {
    LeetifyClient::from_settings(settings)
        .get("/v3/profile", steam_id)
        .await
}

async fn get_leetify_mini_profile_checked(
    settings: &Settings,
    steam_id: &SteamID,
) -> Result<LeetifyMiniProfile> {
    let profile = get_leetify_profile(settings, steam_id)
        .await
        .map_err(|error| failure::upstream(error, "Leetify"))?;
    if profile.privacy_mode != "public" {
        return Err(DataFailure::PrivateProfile.into());
    }
    Ok(profile.into())
}

fn public_match_to_game(game: PublicMatch, steam_id: &SteamID) -> Option<LeetifyGame> {
    let player = game
        .stats
        .iter()
        .find(|player| player.steam64_id == steam_id.to_string())?;

    // `/v3/profile/matches` is player-scoped, so this only records teammates
    // present in that response. Consumers needing an authoritative roster must
    // hydrate the match through `/v2/matches/{id}`.
    let own_team_steam64_ids = game
        .stats
        .iter()
        .filter(|other| other.initial_team_number == player.initial_team_number)
        .map(|other| SteamID::new(other.steam64_id.clone()))
        .collect();

    let own_score = game
        .team_scores
        .iter()
        .find(|score| score.team_number == player.initial_team_number)
        .map(|score| score.score)
        .unwrap_or_default();
    let opponent_score = game
        .team_scores
        .iter()
        .filter(|score| score.team_number != player.initial_team_number)
        .map(|score| score.score)
        .next()
        .unwrap_or_default();
    let match_result = match own_score.cmp(&opponent_score) {
        std::cmp::Ordering::Less => "loss",
        std::cmp::Ordering::Equal => "tie",
        std::cmp::Ordering::Greater => "win",
    };

    Some(LeetifyGame {
        id: Some(game.id),
        own_team_steam64_ids,
        game_finished_at: game.finished_at,
        map_name: game.map_name,
        match_result: match_result.to_string(),
        scores: (own_score, opponent_score),
        skill_level: None,
        teammates_flashed: Some(player.flashbang_hit_friend),
        flashbangs_thrown: Some(player.flashbang_thrown),
        rounds_count: Some(player.rounds_count),
    })
}

fn public_match_to_match(game: PublicMatch) -> Option<LeetifyMatch> {
    if game.team_scores.len() != 2 {
        return None;
    }

    let mut team_numbers = HashSet::new();
    let mut all_players = HashSet::new();
    let mut teams = Vec::with_capacity(2);

    for team_score in &game.team_scores {
        if !team_numbers.insert(team_score.team_number) {
            return None;
        }

        let mut steam64_ids = game
            .stats
            .iter()
            .filter(|player| player.initial_team_number == team_score.team_number)
            .map(|player| SteamID::new(player.steam64_id.clone()))
            .collect::<Vec<_>>();
        steam64_ids.sort_by_key(ToString::to_string);
        steam64_ids.dedup();

        // Prediction context cannot be inferred from a player-scoped or
        // otherwise incomplete response. Only hydrate complete CS2 sides.
        if steam64_ids.len() != 5
            || steam64_ids
                .iter()
                .any(|steam_id| steam_id.to_string().trim().is_empty())
            || !steam64_ids
                .iter()
                .all(|steam_id| all_players.insert(steam_id.clone()))
        {
            return None;
        }

        teams.push(LeetifyMatchTeam {
            steam64_ids,
            score: team_score.score,
        });
    }

    Some(LeetifyMatch {
        id: game.id,
        game_finished_at: game.finished_at,
        map_name: game.map_name,
        teams,
    })
}

#[cached(ttl_secs = 300, key = "SteamID", convert = r#"{ steam_id.clone() }"#)]
async fn get_leetify_games_cached(
    client: LeetifyClient,
    steam_id: SteamID,
) -> Result<Vec<LeetifyGame>> {
    let matches = client
        .get::<Vec<PublicMatch>>("/v3/profile/matches", &steam_id)
        .await
        .map_err(|error| failure::upstream(error, "Leetify"))?;

    let count = matches.len();
    let games = matches
        .into_iter()
        .filter_map(|game| public_match_to_game(game, &steam_id))
        .collect::<Vec<_>>();
    if games.len() != count {
        eprintln!("Leetify match history for {steam_id} contains incomplete player records");
        return Err(DataFailure::Upstream {
            service: "Leetify",
            cause: failure::RequestFailure::InvalidResponse,
        }
        .into());
    }
    Ok(games)
}

async fn get_leetify_games_with_client(
    client: &LeetifyClient,
    steam_id: &SteamID,
) -> Option<Vec<LeetifyGame>> {
    let url = format!("{LEETIFY_API_BASE_URL}/v3/profile/matches?steam64_id={steam_id}");
    let err_context = format!("Error while fetching {url}");
    unwrap_or_log(
        get_leetify_games_cached(client.clone(), steam_id.clone()).await,
        &err_context,
    )
}

pub(crate) async fn get_leetify_games(
    settings: &Settings,
    steam_id: &SteamID,
) -> Option<Vec<LeetifyGame>> {
    let client = LeetifyClient::from_settings(settings);
    get_leetify_games_with_client(&client, steam_id).await
}

pub(crate) async fn get_leetify_games_checked(
    settings: &Settings,
    steam_id: &SteamID,
) -> Result<Vec<LeetifyGame>> {
    get_leetify_games_cached(LeetifyClient::from_settings(settings), steam_id.clone()).await
}

pub(crate) struct PlayerHistories {
    pub games: Vec<(String, Vec<LeetifyGame>)>,
    pub unavailable: Vec<String>,
}

pub(crate) fn collect_player_histories(
    results: Vec<(Username, Result<Vec<LeetifyGame>>)>,
    required: Option<&Username>,
) -> Result<PlayerHistories> {
    let mut histories = PlayerHistories {
        games: Vec::new(),
        unavailable: Vec::new(),
    };
    let mut first_error = None;
    for (name, result) in results {
        match result {
            Ok(games) => histories.games.push((name.to_string(), games)),
            Err(error) => {
                if required == Some(&name) {
                    return Err(error);
                }
                eprintln!("Match history unavailable for {name}: {error:?}");
                histories.unavailable.push(name.to_string());
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }
    }
    let has_games = histories.games.iter().any(|(name, games)| {
        !games.is_empty() && required.is_none_or(|user| name == &user.to_string())
    });
    if !has_games {
        if required.is_some() {
            return Err(DataFailure::NoMatches(required.cloned()).into());
        }
        if let Some(error) = first_error {
            return Err(error);
        }
        return Err(DataFailure::NoMatches(required.cloned()).into());
    }
    histories.unavailable.sort();
    Ok(histories)
}

fn match_cache_file_path(cache_path: &Path, match_id: &str) -> Option<PathBuf> {
    match_id
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
        .then(|| cache_path.join(format!("{match_id}.json")))
}

async fn read_match_from_disk(cache_path: &Path, match_id: &str) -> Result<Option<LeetifyMatch>> {
    let Some(path) = match_cache_file_path(cache_path, match_id) else {
        return Err(eyre!("invalid Leetify match ID for cache path: {match_id}"));
    };

    let bytes = match tokio::fs::read(&path).await {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let game = serde_json::from_slice::<LeetifyMatch>(&bytes)?;
    if game.id != match_id {
        return Err(eyre!(
            "cached Leetify match ID {} did not match requested ID {match_id}",
            game.id
        ));
    }
    Ok(Some(game))
}

async fn write_match_to_disk(cache_path: &Path, game: &LeetifyMatch) -> Result<()> {
    let Some(path) = match_cache_file_path(cache_path, &game.id) else {
        return Err(eyre!(
            "invalid Leetify match ID for cache path: {}",
            game.id
        ));
    };
    tokio::fs::create_dir_all(cache_path).await?;

    let counter = MATCH_CACHE_TEMP_FILE_COUNTER.fetch_add(1, AtomicOrdering::Relaxed);
    let temporary_path = cache_path.join(format!(
        ".{}.{}.{}.tmp",
        game.id,
        std::process::id(),
        counter
    ));
    tokio::fs::write(&temporary_path, serde_json::to_vec(game)?).await?;
    tokio::fs::rename(&temporary_path, path).await?;
    Ok(())
}

#[cached(
    max_size = 10000,
    key = "(String, String, Option<PathBuf>)",
    convert = r#"{ (client.base_url.clone(), match_id.clone(), match_cache_path.clone()) }"#
)]
async fn get_leetify_match_cached(
    client: LeetifyClient,
    match_id: String,
    match_cache_path: Option<PathBuf>,
) -> std::result::Result<Option<LeetifyMatch>, MatchHydrationError> {
    if let Some(cache_path) = &match_cache_path {
        match read_match_from_disk(cache_path, &match_id).await {
            Ok(Some(game)) => return Ok(Some(game)),
            Ok(None) => {}
            Err(error) => eprintln!(
                "Failed to read cached Leetify match {match_id} from {}: {error}",
                cache_path.display()
            ),
        }
    }

    let path = format!("/v2/matches/{match_id}");
    let response = client
        .get_path_response(&path)
        .await
        .map_err(|error| MatchHydrationError::Transient(error.to_string()))?;
    let status = response.status();
    let url = response.url().clone();
    if matches!(
        status,
        reqwest::StatusCode::FORBIDDEN | reqwest::StatusCode::NOT_FOUND | reqwest::StatusCode::GONE
    ) {
        return Ok(None);
    }
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        let retry_after = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
            .map(Duration::from_secs);
        return Err(MatchHydrationError::RateLimited {
            message: format!("HTTP status {status} for url ({url})"),
            retry_after,
        });
    }
    if status.is_server_error() {
        return Err(MatchHydrationError::Transient(format!(
            "HTTP status {status} for url ({url})"
        )));
    }
    if !status.is_success() {
        return Err(MatchHydrationError::Fatal(format!(
            "HTTP status {status} for url ({url})"
        )));
    }
    let public_match = response
        .json::<PublicMatch>()
        .await
        .map_err(|error| MatchHydrationError::Transient(error.to_string()))?;
    let game = public_match_to_match(public_match);

    if let (Some(cache_path), Some(game)) = (&match_cache_path, &game) {
        if let Err(error) = write_match_to_disk(cache_path, game).await {
            eprintln!(
                "Failed to persist Leetify match {match_id} to {}: {error}",
                cache_path.display()
            );
        }
    }

    Ok(game)
}

fn hydration_retry_delay(attempt: usize, retry_after: Option<Duration>) -> Option<Duration> {
    let fallback = Duration::from_secs(2_u64.pow(attempt as u32 + 1));
    let delay = retry_after.unwrap_or(fallback);
    (delay <= MATCH_HYDRATION_MAX_RETRY_DELAY).then_some(delay)
}

pub(crate) async fn get_leetify_matches(
    settings: &Settings,
    match_ids: HashSet<String>,
) -> LeetifyMatchHydration {
    let client = LeetifyClient::from_settings(settings);
    let match_cache_path = settings
        .leetify
        .as_ref()
        .and_then(|config| config.match_cache_path.clone());
    let mut match_ids = match_ids.into_iter().collect::<Vec<_>>();
    match_ids.sort();
    let mut hydration = LeetifyMatchHydration::default();

    // Match-detail responses are immutable and cached individually. Hydrate
    // uncached IDs sequentially so a Leetify rate limit only delays the one
    // request that hit it. A batch-wide retry budget would otherwise defer all
    // remaining IDs and can exhaust its attempts while making little progress.
    for match_id in match_ids {
        for attempt in 0..MATCH_HYDRATION_MAX_ATTEMPTS {
            let result = get_leetify_match_cached(
                client.clone(),
                match_id.clone(),
                match_cache_path.clone(),
            )
            .await;

            match result {
                Ok(Some(game)) => {
                    hydration.matches.insert(match_id.clone(), game);
                    break;
                }
                Ok(None) => {
                    eprintln!("Skipping unavailable or incomplete Leetify match {match_id}");
                    break;
                }
                Err(error) if error.is_retryable() => {
                    eprintln!(
                        "Leetify match {match_id} hydration attempt {} failed and may be retried: {error}",
                        attempt + 1
                    );

                    if attempt + 1 == MATCH_HYDRATION_MAX_ATTEMPTS {
                        hydration.failed_match_ids.insert(match_id.clone());
                        break;
                    }

                    let Some(delay) = hydration_retry_delay(attempt, error.retry_after()) else {
                        eprintln!(
                            "Leetify requested a retry delay longer than {} seconds; deferring match {match_id}",
                            MATCH_HYDRATION_MAX_RETRY_DELAY.as_secs()
                        );
                        hydration.failed_match_ids.insert(match_id.clone());
                        break;
                    };
                    eprintln!(
                        "Retrying Leetify match {match_id} in {} seconds",
                        delay.as_secs()
                    );
                    tokio::time::sleep(delay).await;
                }
                Err(error) => {
                    eprintln!("Failed to hydrate Leetify match {match_id}: {error}");
                    hydration.failed_match_ids.insert(match_id.clone());
                    break;
                }
            }
        }
    }
    hydration
}

async fn get_configured_player_games(settings: &Settings) -> HashMap<SteamID, Vec<LeetifyGame>> {
    let steam_ids: HashSet<SteamID> = settings
        .players
        .steamid_mappings
        .values()
        .cloned()
        .collect();
    let client = LeetifyClient::from_settings(settings);

    let requests = steam_ids.into_iter().map(|steam_id| {
        let client = client.clone();
        async move {
            let games = get_leetify_games_with_client(&client, &steam_id).await;
            (steam_id, games)
        }
    });

    futures::stream::iter(requests)
        .buffer_unordered(5)
        .filter_map(|(steam_id, games)| async move { games.map(|games| (steam_id, games)) })
        .collect::<HashMap<_, _>>()
        .await
}

impl From<PublicProfile> for LeetifyMiniProfile {
    fn from(profile: PublicProfile) -> Self {
        let mut ranks = Vec::new();
        let rank = |r#type: &str, data_source: &str, skill_level: Option<f32>| {
            skill_level.map(|skill_level| LeetifyRank {
                r#type: Some(r#type.to_string()),
                data_source: Some(data_source.to_string()),
                skill_level: Some(skill_level as u32),
            })
        };

        if let Some(rank) = rank("premier", "matchmaking", profile.ranks.premier) {
            ranks.push(rank);
        }
        if let Some(rank) = rank("wingman", "matchmaking_wingman", profile.ranks.wingman) {
            ranks.push(rank);
        }
        if let Some(rank) = rank("faceit", "faceit", profile.ranks.faceit) {
            ranks.push(rank);
        }
        if let Some(rank) = rank("faceit_elo", "faceit", profile.ranks.faceit_elo) {
            ranks.push(rank);
        }
        if let Some(rank) = rank("leetify", "leetify", profile.ranks.leetify) {
            ranks.push(rank);
        }
        if let Some(rank) = rank("renown", "renown", profile.ranks.renown) {
            ranks.push(rank);
        }
        ranks.extend(
            profile
                .ranks
                .competitive
                .into_iter()
                .map(|rank| LeetifyRank {
                    r#type: Some(rank.map_name),
                    data_source: Some("matchmaking".to_string()),
                    skill_level: Some(rank.rank),
                }),
        );

        Self {
            ratings: LeetifyStats {
                aim: profile.rating.aim,
                positioning: profile.rating.positioning,
                utility: profile.rating.utility,
                games_played: profile.total_matches,
                clutch: profile.rating.clutch,
                ct_leetify: profile.rating.ct_leetify,
                opening: profile.rating.opening,
                t_leetify: profile.rating.t_leetify,
                skill_level: profile.ranks.premier.map(|rank| rank as u32),
            },
            ranks,
            recent_matches: profile
                .recent_matches
                .into_iter()
                .map(|game| RecentMatch {
                    result: game.outcome,
                })
                .collect(),
        }
    }
}

pub fn steamid_for_username(settings: Settings, username: &Username) -> Option<SteamID> {
    let steamid_mappings = settings.players.steamid_mappings;
    let steamid = steamid_mappings.get(username);
    steamid.cloned()
}

#[derive(Clone, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct LeetifyGame {
    #[serde(default)]
    pub id: Option<String>,
    /// Player-side IDs present in the source response. Profile-match responses
    /// may contain only the queried player, so this is not always a full roster.
    pub own_team_steam64_ids: Vec<SteamID>,
    pub game_finished_at: DateTime<Utc>,
    pub map_name: String,
    pub match_result: String,
    pub scores: (u32, u32),
    pub skill_level: Option<u32>,
    #[serde(default)]
    pub teammates_flashed: Option<u32>,
    #[serde(default)]
    pub flashbangs_thrown: Option<u32>,
    #[serde(default)]
    pub rounds_count: Option<u32>,
}

pub const CS2_ESTIMATED_MINUTES_PER_ROUND: f64 = 2.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MatchDurationEstimate {
    pub matches_used: usize,
    pub average_rounds: f64,
    pub minutes: f64,
}

/// Estimates a typical match length from the newest matches with round data.
///
/// Leetify's public match endpoint exposes the number of rounds but not start
/// time or duration. Two minutes per round includes the round and freeze/buy
/// phases and is intentionally kept as a visible, tunable assumption.
pub fn average_recent_match_duration(games: &[LeetifyGame]) -> Option<MatchDurationEstimate> {
    let mut games = games.iter().collect::<Vec<_>>();
    games.sort_by_key(|game| std::cmp::Reverse(game.game_finished_at));

    let rounds: Vec<u32> = games
        .into_iter()
        .take(RECENT_MATCHES_LIMIT)
        .filter_map(|game| game.rounds_count.filter(|rounds| *rounds > 0))
        .collect();
    let matches_used = rounds.len();

    (matches_used > 0).then(|| {
        let average_rounds =
            rounds.iter().map(|rounds| *rounds as f64).sum::<f64>() / matches_used as f64;
        MatchDurationEstimate {
            matches_used,
            average_rounds,
            minutes: average_rounds * CS2_ESTIMATED_MINUTES_PER_ROUND,
        }
    })
}

/// Fetches configured players' matches and estimates one shared recent-match
/// duration, deduplicating matches that appear in multiple player histories.
pub(crate) async fn average_recent_match_duration_for_configured_players(
    settings: &Settings,
) -> Option<MatchDurationEstimate> {
    let configured_games = get_configured_player_games(settings).await;
    let mut seen_match_ids = HashSet::new();
    let mut games = Vec::new();

    for player_games in configured_games.values() {
        for game in player_games {
            if let Some(id) = &game.id {
                if !seen_match_ids.insert(id.clone()) {
                    continue;
                }
            }
            games.push(game.clone());
        }
    }

    average_recent_match_duration(&games)
}

#[derive(Debug, Clone)]
pub struct LastPlayedResult {
    pub game: LeetifyGame,
    pub teammates: Vec<Username>,
    pub spree: Option<usize>,
}

fn verified_squad_game(
    history: &LeetifyGame,
    details: &LeetifyMatch,
    own: &SteamID,
    mappings: &HashMap<Username, SteamID>,
) -> Result<Option<LastPlayedResult>> {
    if details.teams.len() != 2 {
        return Err(eyre!("Incomplete roster for {}", details.id));
    }
    let team = details
        .teams
        .iter()
        .find(|team| team.steam64_ids.contains(own))
        .ok_or_else(|| eyre!("Player missing from roster for {}", details.id))?;
    let opponents = details
        .teams
        .iter()
        .find(|team| !team.steam64_ids.contains(own))
        .ok_or_else(|| eyre!("Opposing roster missing for {}", details.id))?;
    let mut names = mappings
        .iter()
        .filter(|(_, id)| *id != own && team.steam64_ids.contains(id))
        .collect::<Vec<_>>();
    names.sort_by_key(|(name, _)| name.to_string());
    let mut seen = HashSet::new();
    let teammates = names
        .into_iter()
        .filter(|(_, id)| seen.insert(*id))
        .map(|(name, _)| name.clone())
        .collect::<Vec<_>>();
    if teammates.is_empty() {
        return Ok(None);
    }
    let mut game = history.clone();
    game.id = Some(details.id.clone());
    game.game_finished_at = details.game_finished_at;
    game.map_name = details.map_name.clone();
    game.own_team_steam64_ids = team.steam64_ids.clone();
    game.scores = (team.score, opponents.score);
    game.match_result = match team.score.cmp(&opponents.score) {
        std::cmp::Ordering::Greater => "win",
        std::cmp::Ordering::Less => "loss",
        std::cmp::Ordering::Equal => "tie",
    }
    .to_string();
    Ok(Some(LastPlayedResult {
        game,
        teammates,
        spree: None,
    }))
}

fn squad_playing_streak(mut dates: Vec<NaiveDate>, today: NaiveDate) -> usize {
    dates.sort_by(|a, b| b.cmp(a));
    dates.dedup();
    let Some(latest) = dates.first() else {
        return 0;
    };
    if (today - *latest).num_days() > 1 || *latest > today {
        return 0;
    }
    dates
        .windows(2)
        .take_while(|pair| (pair[0] - pair[1]).num_days() == 1)
        .count()
        + 1
}

async fn find_last_squad_match(
    settings: &Settings,
    own: &SteamID,
    games: &[LeetifyGame],
    now: DateTime<Tz>,
    with_streak: bool,
) -> Result<LastPlayedResult> {
    let mut games = games.iter().collect::<Vec<_>>();
    games.sort_by_key(|game| (std::cmp::Reverse(game.game_finished_at), game.id.clone()));
    let mut seen = HashSet::new();
    let mut latest = None;
    let mut dates: Vec<NaiveDate> = Vec::new();
    let mut streak_known = true;
    for game in games {
        if let Some(last_date) = dates.last() {
            // Older matches cannot extend this active consecutive-day streak.
            if game
                .game_finished_at
                .with_timezone(&now.timezone())
                .date_naive()
                < *last_date - chrono::Days::new(1)
            {
                break;
            }
        }
        let Some(id) = &game.id else {
            if latest.is_none() {
                return Err(DataFailure::IncompleteRosters.into());
            }
            streak_known = false;
            break;
        };
        if !seen.insert(id) {
            continue;
        }
        // Hydrate only as far back as needed, reusing immutable match caches.
        let hydration = get_leetify_matches(settings, HashSet::from([id.clone()])).await;
        let verified = hydration
            .matches
            .get(id)
            .ok_or_else(|| eyre!("Roster unavailable for {id}"))
            .and_then(|details| {
                verified_squad_game(game, details, own, &settings.players.steamid_mappings)
            });
        let verified = match verified {
            Ok(value) => value,
            Err(error) if latest.is_none() => {
                eprintln!("Last squad roster verification failed: {error:?}");
                return Err(DataFailure::IncompleteRosters.into());
            }
            Err(error) => {
                eprintln!("Cannot verify older streak history: {error}");
                streak_known = false;
                break;
            }
        };
        if let Some(result) = verified {
            let date = result
                .game
                .game_finished_at
                .with_timezone(&now.timezone())
                .date_naive();
            if dates.last() != Some(&date) {
                dates.push(date);
            }
            if latest.is_none() {
                latest = Some(result);
            }
            if !with_streak || (now.date_naive() - dates[0]).num_days() > 1 {
                break;
            }
        }
    }
    let mut latest = latest.ok_or(DataFailure::NoSquadMatch)?;
    latest.spree =
        (with_streak && streak_known).then(|| squad_playing_streak(dates, now.date_naive()));
    Ok(latest)
}

pub async fn last_played(
    settings: &Settings,
    username: &Username,
    tz: Tz,
) -> Result<LastPlayedResult> {
    let steamid = steamid_for_username(settings.clone(), username)
        .ok_or_else(|| DataFailure::Unlinked(username.clone()))?;
    let games = get_leetify_games_checked(settings, &steamid).await?;
    if games.is_empty() {
        return Err(DataFailure::NoMatches(Some(username.clone())).into());
    }
    find_last_squad_match(
        settings,
        &steamid,
        &games,
        Utc::now().with_timezone(&tz),
        false,
    )
    .await
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LeetifyStats {
    pub aim: f32,
    pub positioning: f32,
    pub utility: f32,
    pub games_played: u32,
    pub clutch: f32,
    pub ct_leetify: f32,
    pub opening: f32,
    pub t_leetify: f32,
    pub skill_level: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LeetifyRank {
    pub r#type: Option<String>,
    pub data_source: Option<String>,
    pub skill_level: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MatchResult {
    Loss,
    Win,
    Tie,
}

impl Display for MatchResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MatchResult::Loss => write!(f, "L"),
            MatchResult::Win => write!(f, "W"),
            MatchResult::Tie => write!(f, "T"),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct RecentMatch {
    pub result: MatchResult,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LeetifyMiniProfile {
    pub ratings: LeetifyStats,
    pub ranks: Vec<LeetifyRank>,
    pub recent_matches: Vec<RecentMatch>,
}

pub async fn player_stats(settings: &Settings, username: &Username) -> Result<LeetifyMiniProfile> {
    let steamid = steamid_for_username(settings.clone(), username)
        .ok_or_else(|| DataFailure::Unlinked(username.clone()))?;
    get_leetify_mini_profile_checked(settings, &steamid).await
}

pub const RECENT_MATCHES_LIMIT: usize = 30;
const TEAMMATE_RECORD_LIMIT: usize = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeammateStatsEntry {
    pub username: Username,
    pub wins: usize,
    pub losses: usize,
    pub ties: usize,
}

pub struct TeammateStats {
    pub entries: Vec<TeammateStatsEntry>,
    pub matches_considered: usize,
}

impl TeammateStatsEntry {
    pub fn games(&self) -> usize {
        self.wins + self.losses + self.ties
    }
}

fn teammate_stats_from_matches(
    games: &[LeetifyGame],
    matches: &HashMap<String, LeetifyMatch>,
    steamid_mappings: &HashMap<Username, SteamID>,
    own_steamid: &SteamID,
) -> TeammateStats {
    let recent_games = recent_teammate_games(games);
    let matches_considered = recent_games.len();
    let mut entries = HashMap::<SteamID, TeammateStatsEntry>::new();
    // Stable alias selection when several configured names map to one Steam ID.
    let mut players = steamid_mappings.iter().collect::<Vec<_>>();
    players.sort_by_key(|(name, _)| name.to_string());

    for game in recent_games {
        let Some(details) = game.id.as_ref().and_then(|id| matches.get(id)) else {
            continue;
        };
        let Some(own_team) = details
            .teams
            .iter()
            .find(|team| team.steam64_ids.contains(own_steamid))
        else {
            continue;
        };
        let Some(opponents) = details
            .teams
            .iter()
            .find(|team| !team.steam64_ids.contains(own_steamid))
        else {
            continue;
        };
        let mut seen_players = HashSet::new();
        for &(username, teammate_steamid) in &players {
            if teammate_steamid == own_steamid
                || !own_team.steam64_ids.contains(teammate_steamid)
                || !seen_players.insert(teammate_steamid)
            {
                continue;
            }

            let entry =
                entries
                    .entry(teammate_steamid.clone())
                    .or_insert_with(|| TeammateStatsEntry {
                        username: username.clone(),
                        wins: 0,
                        losses: 0,
                        ties: 0,
                    });

            match own_team.score.cmp(&opponents.score) {
                std::cmp::Ordering::Greater => entry.wins += 1,
                std::cmp::Ordering::Less => entry.losses += 1,
                std::cmp::Ordering::Equal => entry.ties += 1,
            }
        }
    }

    let mut entries: Vec<TeammateStatsEntry> = entries.into_values().collect();
    entries.sort_by(|a, b| {
        b.games()
            .cmp(&a.games())
            .then_with(|| a.username.to_string().cmp(&b.username.to_string()))
    });
    entries.truncate(TEAMMATE_RECORD_LIMIT);
    TeammateStats {
        entries,
        matches_considered,
    }
}

fn recent_teammate_games(games: &[LeetifyGame]) -> Vec<&LeetifyGame> {
    let mut games = games.iter().collect::<Vec<_>>();
    games.sort_by_key(|game| (std::cmp::Reverse(game.game_finished_at), game.id.clone()));
    let mut seen = HashSet::new();
    games.retain(|game| game.id.as_ref().is_none_or(|id| seen.insert(id)));
    games.truncate(RECENT_MATCHES_LIMIT);
    games
}

pub async fn teammate_stats(settings: &Settings, username: &Username) -> Result<TeammateStats> {
    let own_steamid = steamid_for_username(settings.clone(), username)
        .ok_or_else(|| DataFailure::Unlinked(username.clone()))?;
    let games = get_leetify_games_checked(settings, &own_steamid).await?;
    let recent = recent_teammate_games(&games);
    let match_ids = recent
        .iter()
        .filter_map(|game| game.id.clone())
        .collect::<HashSet<_>>();
    if match_ids.len() != recent.len() {
        return Err(DataFailure::IncompleteRosters.into());
    }
    let hydration = get_leetify_matches(settings, match_ids.clone()).await;
    // A partial sample could misleadingly undercount games with a teammate.
    if match_ids.iter().any(|id| {
        hydration.matches.get(id).is_none_or(|game| {
            !game
                .teams
                .iter()
                .any(|team| team.steam64_ids.contains(&own_steamid))
        })
    }) {
        return Err(DataFailure::IncompleteRosters.into());
    }

    Ok(teammate_stats_from_matches(
        &games,
        &hydration.matches,
        &settings.players.steamid_mappings,
        &own_steamid,
    ))
}

pub struct HallOfShameEntry {
    pub username: Username,
    pub last_played: DateTime<Utc>,
    pub spree: Option<usize>,
}

pub struct HallOfShame {
    pub entries: Vec<HallOfShameEntry>,
    pub unavailable: Vec<Username>,
}

pub async fn hall_of_shame(settings: &Settings, tz: Tz) -> Result<HallOfShame> {
    if settings.players.steamid_mappings.is_empty() {
        return Err(DataFailure::NoPlayers.into());
    }
    let configured_games = get_configured_player_games(settings).await;
    let mut entries = Vec::new();
    let mut unavailable = Vec::new();
    let now = Utc::now().with_timezone(&tz);
    let mut players = settings.players.steamid_mappings.iter().collect::<Vec<_>>();
    players.sort_by_key(|(name, _)| name.to_string());
    let mut resolved = HashMap::<SteamID, Option<LastPlayedResult>>::new();
    for (username, steamid) in players {
        if !resolved.contains_key(steamid) {
            let result = if let Some(games) = configured_games.get(steamid) {
                match find_last_squad_match(settings, steamid, games, now, true).await {
                    Ok(result) => Some(result),
                    Err(error) => {
                        eprintln!("Failed to verify squad history for {username}: {error}");
                        None
                    }
                }
            } else {
                None
            };
            resolved.insert(steamid.clone(), result);
        }
        if let Some(result) = resolved.get(steamid).and_then(Option::as_ref) {
            entries.push(HallOfShameEntry {
                username: username.clone(),
                last_played: result.game.game_finished_at,
                spree: result.spree,
            });
        } else {
            unavailable.push(username.clone());
        }
    }
    entries.sort_by(|a, b| {
        a.last_played
            .cmp(&b.last_played)
            .then_with(|| a.username.to_string().cmp(&b.username.to_string()))
    });
    Ok(HallOfShame {
        entries,
        unavailable,
    })
}

#[derive(Debug)]
pub struct HallOfFameEntry {
    pub username: Username,
    pub skill_level: u32,
}

pub struct HallOfFame {
    pub entries: Vec<HallOfFameEntry>,
    pub avg_skill_level: f32,
    pub median_skill_level: u32,
}

/// Keep an all-failed lookup distinct from a successful lookup with no entries.
fn collect_player_entries<T>(results: Vec<Result<Option<T>>>) -> Result<Vec<T>> {
    let mut entries = Vec::new();
    let mut failure = None;
    for result in results {
        match result {
            Ok(Some(entry)) => entries.push(entry),
            Ok(None) => {}
            Err(error) => {
                eprintln!("Player data unavailable: {error:?}");
                if failure.is_none() {
                    failure = Some(error);
                }
            }
        }
    }
    if entries.is_empty() {
        if let Some(error) = failure {
            return Err(error);
        }
    }
    Ok(entries)
}

/// List top 10 players based on their skill level in their most recent game
pub async fn hall_of_fame(settings: &Settings, rank_type: &String) -> Result<HallOfFame> {
    if settings.players.steamid_mappings.is_empty() {
        return Err(DataFailure::NoPlayers.into());
    }
    let steamid_mappings = settings.players.steamid_mappings.clone();

    let futures: Vec<_> = steamid_mappings
        .into_iter()
        .map(|(username, steamid)| {
            let rank_type = rank_type.clone();
            let settings = settings.clone();

            async move {
                let resp = get_leetify_mini_profile_checked(&settings, &steamid).await?;

                let leetify_rank = resp.ranks.iter().find(|r| {
                    if rank_type == "wingman" {
                        r.data_source.as_deref() == Some("matchmaking_wingman")
                    } else {
                        r.data_source.as_deref() == Some("matchmaking")
                            && r.r#type.as_ref() == Some(&rank_type)
                    }
                });
                let skill_level = leetify_rank.and_then(|r| r.skill_level);

                let Some(skill_level) = skill_level else {
                    eprintln!("Failed to find {rank_type} rank for player {username}");

                    return Ok(None);
                };

                Ok(Some(HallOfFameEntry {
                    username: username.clone(),
                    skill_level,
                }))
            }
        })
        .collect();

    // create a buffered stream that will execute up to 3 futures in parallel
    // (without preserving the order of the results)
    let stream = futures::stream::iter(futures).buffer_unordered(3);

    // wait for all futures to complete
    let tasks_results = stream.collect::<Vec<_>>().await;

    let mut entries: Vec<HallOfFameEntry> = collect_player_entries(tasks_results)?;

    // Don't include players with no rank
    entries.retain(|entry| entry.skill_level != 0);

    if rank_type == "premier" {
        // Don't include players with old CSGO premier rank
        entries.retain(|entry| entry.skill_level >= 1000);
    }

    entries.sort_by_key(|entry| entry.skill_level);
    entries.reverse();

    let avg_skill_level = if entries.is_empty() {
        0.0
    } else {
        entries.iter().map(|entry| entry.skill_level).sum::<u32>() as f32 / entries.len() as f32
    };

    let median_skill_level = if rank_type == "premier" {
        numeric_median(
            &entries
                .iter()
                .map(|entry| entry.skill_level as f32)
                .collect::<Vec<_>>(),
        )
        .round() as u32
    } else {
        entries
            .get(entries.len() / 2)
            .map(|entry| entry.skill_level)
            .unwrap_or(0)
    };

    Ok(HallOfFame {
        avg_skill_level,
        median_skill_level,
        entries,
    })
}

#[derive(Debug)]
pub struct StatLeaderboardEntry {
    pub username: Username,
    pub stat_value: f32,
}

#[allow(dead_code)]
pub struct StatLeaderboard {
    pub stat_type: String,
    pub entries: Vec<StatLeaderboardEntry>,
    pub avg: f32,
    pub median: f32,
}

/// Median of values sorted in either ascending or descending order.
fn numeric_median(sorted: &[f32]) -> f32 {
    if sorted.is_empty() {
        return 0.0;
    }
    let middle = sorted.len() / 2;
    if sorted.len().is_multiple_of(2) {
        (sorted[middle - 1] + sorted[middle]) / 2.0
    } else {
        sorted[middle]
    }
}

/// List players based on a specific stat (aim, positioning, utility, opening, clutch).
pub async fn stat_leaderboard(settings: &Settings, stat_type: &str) -> Result<StatLeaderboard> {
    if settings.players.steamid_mappings.is_empty() {
        return Err(DataFailure::NoPlayers.into());
    }
    let steamid_mappings = settings.players.steamid_mappings.clone();

    let futures: Vec<_> = steamid_mappings
        .into_iter()
        .map(|(username, steamid)| {
            let stat_type = stat_type.to_string();
            let settings = settings.clone();

            async move {
                let resp = get_leetify_mini_profile_checked(&settings, &steamid).await?;

                let stat_value = match stat_type.as_str() {
                    "aim" => resp.ratings.aim,
                    "positioning" => resp.ratings.positioning,
                    "utility" => resp.ratings.utility,
                    "opening" => resp.ratings.opening,
                    "clutch" => resp.ratings.clutch,
                    _ => return Ok(None),
                };

                Ok(Some(StatLeaderboardEntry {
                    username: username.clone(),
                    stat_value,
                }))
            }
        })
        .collect();

    // create a buffered stream that will execute up to 3 futures in parallel
    // (without preserving the order of the results)
    let stream = futures::stream::iter(futures).buffer_unordered(3);

    // wait for all futures to complete
    let tasks_results = stream.collect::<Vec<_>>().await;

    let mut entries: Vec<StatLeaderboardEntry> = collect_player_entries(tasks_results)?;

    // Sort by stat value, highest first
    entries.sort_by(|a, b| {
        b.stat_value
            .partial_cmp(&a.stat_value)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let stat_values: Vec<f32> = entries.iter().map(|e| e.stat_value).collect();
    let avg = if stat_values.is_empty() {
        0.0
    } else {
        stat_values.iter().sum::<f32>() / stat_values.len() as f32
    };

    let median = numeric_median(&stat_values);

    Ok(StatLeaderboard {
        stat_type: stat_type.to_string(),
        entries,
        avg,
        median,
    })
}

#[derive(Debug)]
pub struct TeamFlashEntry {
    pub username: Username,
    pub flashbangs_thrown_per_round: f32,
    pub teammates_flashed_per_round: f32,
    pub teammates_flashed_per_flash: f32,
}

pub struct TeamFlashLeaderboard {
    pub entries: Vec<TeamFlashEntry>,
    pub avg_flashbangs_thrown: f32,
    pub avg: f32,
    pub avg_teammates_flashed_per_flash: f32,
}

/// List players ranked by teammates flashed per round (highest = most team flashes = worst)
pub async fn team_flash_leaderboard(settings: &Settings) -> Result<TeamFlashLeaderboard> {
    if settings.players.steamid_mappings.is_empty() {
        return Err(DataFailure::NoPlayers.into());
    }
    let steamid_mappings = settings.players.steamid_mappings.clone();

    let futures: Vec<_> = steamid_mappings
        .into_iter()
        .map(|(username, steamid)| {
            let settings = settings.clone();

            async move {
                let games = get_leetify_games_checked(&settings, &steamid).await?;

                // The public API exposes total flashbangs thrown, friendly flash hits, and round counts per match.
                let (thrown, flashes, rounds) =
                    games
                        .iter()
                        .fold((0u32, 0u32, 0u32), |(thrown, flashes, rounds), game| {
                            (
                                thrown + game.flashbangs_thrown.unwrap_or_default(),
                                flashes + game.teammates_flashed.unwrap_or_default(),
                                rounds + game.rounds_count.unwrap_or_default(),
                            )
                        });
                let rates = (rounds > 0).then(|| {
                    (
                        thrown as f32 / rounds as f32,
                        flashes as f32 / rounds as f32,
                        if thrown > 0 {
                            flashes as f32 / thrown as f32
                        } else {
                            0.0
                        },
                    )
                });

                let Some((
                    flashbangs_thrown_per_round,
                    teammates_flashed_per_round,
                    teammates_flashed_per_flash,
                )) = rates
                else {
                    eprintln!("Failed to find flashbang rates for player {username}");
                    return Ok(None);
                };

                Ok(Some(TeamFlashEntry {
                    username: username.clone(),
                    flashbangs_thrown_per_round,
                    teammates_flashed_per_round,
                    teammates_flashed_per_flash,
                }))
            }
        })
        .collect();

    let stream = futures::stream::iter(futures).buffer_unordered(3);
    let tasks_results = stream.collect::<Vec<_>>().await;

    let mut entries: Vec<TeamFlashEntry> = collect_player_entries(tasks_results)?;

    // Sort by teammates flashed, highest first (most team flashes = "winner" of hall of shame)
    entries.sort_by(|a, b| {
        b.teammates_flashed_per_round
            .partial_cmp(&a.teammates_flashed_per_round)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let values: Vec<f32> = entries
        .iter()
        .map(|e| e.teammates_flashed_per_round)
        .collect();
    let thrown_values: Vec<f32> = entries
        .iter()
        .map(|e| e.flashbangs_thrown_per_round)
        .collect();
    let per_flash_values: Vec<f32> = entries
        .iter()
        .map(|e| e.teammates_flashed_per_flash)
        .collect();
    let avg = if values.is_empty() {
        0.0
    } else {
        values.iter().sum::<f32>() / values.len() as f32
    };
    let avg_flashbangs_thrown = if thrown_values.is_empty() {
        0.0
    } else {
        thrown_values.iter().sum::<f32>() / thrown_values.len() as f32
    };
    let avg_teammates_flashed_per_flash = if per_flash_values.is_empty() {
        0.0
    } else {
        per_flash_values.iter().sum::<f32>() / per_flash_values.len() as f32
    };

    Ok(TeamFlashLeaderboard {
        entries,
        avg_flashbangs_thrown,
        avg,
        avg_teammates_flashed_per_flash,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{ElectricitySettings, PlayersSettings, TeloxideSettings};
    use std::collections::HashMap;

    const PUBLIC_TEST_STEAM_ID: &str = "76561198016607756";

    #[test]
    fn failed_player_lookups_are_not_reported_as_successful_empty_leaderboards() {
        assert!(
            collect_player_entries::<u32>(vec![Err(DataFailure::PrivateProfile.into())]).is_err()
        );
        assert!(collect_player_entries::<u32>(vec![Ok(None)])
            .unwrap()
            .is_empty());
        assert_eq!(
            collect_player_entries(vec![Ok(Some(42)), Err(DataFailure::PrivateProfile.into())])
                .unwrap(),
            vec![42]
        );
        assert!(collect_player_entries::<u32>(vec![
            Ok(None),
            Err(DataFailure::PrivateProfile.into())
        ])
        .is_err());
    }

    #[test]
    fn partial_histories_keep_available_games_and_never_invent_empty_history() {
        let alice = Username::new("Alice".into());
        let bob = Username::new("Bob".into());
        let game = squad_fixture("partial-history", Utc::now(), true).0;
        let history = collect_player_histories(
            vec![
                (alice.clone(), Ok(vec![game.clone()])),
                (bob.clone(), Err(DataFailure::PrivateProfile.into())),
            ],
            None,
        )
        .unwrap();
        assert_eq!(history.games.len(), 1);
        assert_eq!(history.unavailable, vec!["Bob"]);
        let all_failed = collect_player_histories(
            vec![(bob.clone(), Err(DataFailure::PrivateProfile.into()))],
            None,
        )
        .err()
        .unwrap();
        assert!(matches!(
            all_failed.downcast_ref::<DataFailure>(),
            Some(DataFailure::PrivateProfile)
        ));
        assert!(collect_player_histories(
            vec![
                (alice.clone(), Ok(vec![game])),
                (bob.clone(), Err(DataFailure::PrivateProfile.into()))
            ],
            Some(&bob)
        )
        .is_err());
        let empty = collect_player_histories(
            vec![
                (alice.clone(), Ok(vec![])),
                (bob, Err(DataFailure::PrivateProfile.into())),
            ],
            Some(&alice),
        )
        .err()
        .unwrap();
        assert!(matches!(
            empty.downcast_ref::<DataFailure>(),
            Some(DataFailure::NoMatches(Some(_)))
        ));
    }

    #[tokio::test]
    async fn unlinked_targets_and_unconfigured_groups_fail_before_network_requests() {
        let (mut settings, _) = cached_squad_fixtures(&[]).await;
        let name = Username::new("Unlinked".into());
        for error in [
            player_stats(&settings, &name).await.unwrap_err(),
            last_played(&settings, &name, chrono_tz::UTC)
                .await
                .unwrap_err(),
            crate::services::results::get_results_chart(&settings, Some(&name))
                .await
                .unwrap_err(),
            crate::services::activity::get_activity_chart(&settings, Some(&name))
                .await
                .unwrap_err(),
        ] {
            assert!(matches!(
                error.downcast_ref::<DataFailure>(),
                Some(DataFailure::Unlinked(_))
            ));
        }
        settings.players.steamid_mappings.clear();
        for error in [
            crate::services::results::get_results_chart(&settings, None)
                .await
                .unwrap_err(),
            crate::services::activity::get_activity_chart(&settings, None)
                .await
                .unwrap_err(),
        ] {
            assert!(matches!(
                error.downcast_ref::<DataFailure>(),
                Some(DataFailure::NoPlayers)
            ));
        }
    }

    fn squad_fixture(
        id: &str,
        finished: DateTime<Utc>,
        friend_on_own_team: bool,
    ) -> (LeetifyGame, LeetifyMatch) {
        let own = if friend_on_own_team {
            ["own", "friend", "bob", "u1", "u2"]
        } else {
            ["own", "u1", "u2", "u3", "u4"]
        };
        let other = if friend_on_own_team {
            ["rival", "o1", "o2", "o3", "o4"]
        } else {
            ["friend", "rival", "o1", "o2", "o3"]
        };
        let game = LeetifyGame {
            id: Some(id.into()),
            own_team_steam64_ids: vec![],
            game_finished_at: finished,
            map_name: "de_nuke".into(),
            match_result: "loss".into(),
            scores: (1, 99),
            skill_level: None,
            teammates_flashed: None,
            flashbangs_thrown: None,
            rounds_count: None,
        };
        let details = LeetifyMatch {
            id: id.into(),
            game_finished_at: finished,
            map_name: "de_mirage".into(),
            teams: vec![
                LeetifyMatchTeam {
                    steam64_ids: own.map(|id| SteamID::new(id.into())).to_vec(),
                    score: 13,
                },
                LeetifyMatchTeam {
                    steam64_ids: other.map(|id| SteamID::new(id.into())).to_vec(),
                    score: 9,
                },
            ],
        };
        (game, details)
    }

    async fn cached_squad_fixtures(
        fixtures: &[(LeetifyGame, LeetifyMatch)],
    ) -> (Settings, PathBuf) {
        let directory = std::env::temp_dir().join(format!(
            "add-bot-squad-test-{}-{}",
            std::process::id(),
            MATCH_CACHE_TEMP_FILE_COUNTER.fetch_add(1, AtomicOrdering::Relaxed)
        ));
        for (_, details) in fixtures {
            write_match_to_disk(&directory, details).await.unwrap();
        }
        let settings = Settings {
            teloxide: TeloxideSettings {
                bot_api_token: "test".into(),
            },
            players: PlayersSettings {
                steamid_mappings: [
                    ("Player", "own"),
                    ("Alice", "friend"),
                    ("AliceAlias", "friend"),
                    ("Bob", "bob"),
                    ("Rival", "rival"),
                ]
                .into_iter()
                .map(|(name, id)| (Username::new(name.into()), SteamID::new(id.into())))
                .collect(),
            },
            weather: None,
            electricity: ElectricitySettings::default(),
            leetify: Some(crate::settings::LeetifySettings {
                api_key: None,
                match_cache_path: Some(directory.clone()),
            }),
        };
        (settings, directory)
    }

    #[tokio::test]
    async fn last_squad_match_ignores_opponents_and_stops_before_unused_older_history() {
        use chrono::TimeZone;
        let now = chrono_tz::Europe::Helsinki
            .with_ymd_and_hms(2026, 9, 30, 23, 0, 0)
            .unwrap();
        let fixtures = vec![
            squad_fixture(
                "squad-find-newer-opponents",
                now.with_timezone(&Utc) - chrono::Duration::hours(1),
                false,
            ),
            squad_fixture(
                "squad-find-last-teammates",
                now.with_timezone(&Utc) - chrono::Duration::days(1),
                true,
            ),
        ];
        let (settings, directory) = cached_squad_fixtures(&fixtures).await;
        let mut games = fixtures
            .iter()
            .map(|(game, _)| game.clone())
            .collect::<Vec<_>>();
        let mut older = games[1].clone();
        older.id = None;
        older.game_finished_at -= chrono::Duration::days(1);
        games.insert(0, older);
        let result =
            find_last_squad_match(&settings, &SteamID::new("own".into()), &games, now, false)
                .await
                .unwrap();
        assert_eq!(result.game.id.as_deref(), Some("squad-find-last-teammates"));
        assert_eq!(
            result
                .teammates
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            ["Alice", "Bob"]
        );
        assert_eq!(
            (
                result.game.match_result.as_str(),
                result.game.scores,
                result.game.map_name.as_str()
            ),
            ("win", (13, 9), "de_mirage")
        );
        assert_eq!(result.spree, None);
        tokio::fs::remove_dir_all(directory).await.unwrap();
    }

    #[tokio::test]
    async fn squad_streak_uses_local_days_deduplicates_days_and_stops_at_a_gap() {
        use chrono::TimeZone;
        let now = chrono_tz::Europe::Helsinki
            .with_ymd_and_hms(2026, 10, 1, 0, 30, 0)
            .unwrap();
        let timestamps = [
            "2026-09-30T20:00:00Z",
            "2026-09-29T21:30:00Z",
            "2026-09-28T21:30:00Z",
            "2026-09-27T21:30:00Z",
        ];
        let fixtures = timestamps
            .iter()
            .enumerate()
            .map(|(index, time)| {
                squad_fixture(
                    &format!("squad-local-streak-{index}"),
                    DateTime::parse_from_rfc3339(time)
                        .unwrap()
                        .with_timezone(&Utc),
                    true,
                )
            })
            .collect::<Vec<_>>();
        let (settings, directory) = cached_squad_fixtures(&fixtures).await;
        let mut games = fixtures
            .iter()
            .map(|(game, _)| game.clone())
            .collect::<Vec<_>>();
        games.push(games[0].clone());
        let mut gap = games[0].clone();
        gap.id = None;
        gap.game_finished_at = DateTime::parse_from_rfc3339("2026-09-25T21:30:00Z")
            .unwrap()
            .with_timezone(&Utc);
        games.push(gap);
        let result =
            find_last_squad_match(&settings, &SteamID::new("own".into()), &games, now, true)
                .await
                .unwrap();
        assert_eq!(result.spree, Some(3));
        tokio::fs::remove_dir_all(directory).await.unwrap();
    }

    #[tokio::test]
    async fn unverified_newer_match_blocks_latest_but_unverified_older_match_only_hides_streak() {
        use chrono::TimeZone;
        let now = chrono_tz::Europe::Helsinki
            .with_ymd_and_hms(2026, 9, 30, 23, 0, 0)
            .unwrap();
        let fixture = squad_fixture(
            "squad-partial-known-latest",
            now.with_timezone(&Utc) - chrono::Duration::hours(1),
            true,
        );
        let (settings, directory) = cached_squad_fixtures(std::slice::from_ref(&fixture)).await;
        let mut unknown = fixture.0.clone();
        unknown.id = None;
        unknown.game_finished_at += chrono::Duration::minutes(30);
        assert!(find_last_squad_match(
            &settings,
            &SteamID::new("own".into()),
            &[fixture.0.clone(), unknown.clone()],
            now,
            true
        )
        .await
        .is_err());
        unknown.game_finished_at = fixture.0.game_finished_at - chrono::Duration::days(1);
        let result = find_last_squad_match(
            &settings,
            &SteamID::new("own".into()),
            &[fixture.0, unknown],
            now,
            true,
        )
        .await
        .unwrap();
        assert_eq!(result.spree, None);
        tokio::fs::remove_dir_all(directory).await.unwrap();
    }

    #[test]
    fn numeric_median_handles_even_and_odd_groups() {
        assert_eq!(numeric_median(&[80.0, 60.0, 20.0]), 60.0);
        assert_eq!(numeric_median(&[80.0, 60.0, 40.0, 20.0]), 50.0);
        assert_eq!(numeric_median(&[]), 0.0);
    }

    #[tokio::test]
    async fn immutable_match_cache_round_trips_minimal_match_data() {
        let cache_path = std::env::temp_dir().join(format!(
            "add-bot-leetify-cache-test-{}-{}",
            std::process::id(),
            MATCH_CACHE_TEMP_FILE_COUNTER.fetch_add(1, AtomicOrdering::Relaxed)
        ));
        let game = LeetifyMatch {
            id: "cache-test-match".to_string(),
            game_finished_at: Utc::now(),
            map_name: "de_cache".to_string(),
            teams: vec![
                LeetifyMatchTeam {
                    steam64_ids: (1..=5).map(|id| SteamID::new(id.to_string())).collect(),
                    score: 13,
                },
                LeetifyMatchTeam {
                    steam64_ids: (6..=10).map(|id| SteamID::new(id.to_string())).collect(),
                    score: 8,
                },
            ],
        };

        write_match_to_disk(&cache_path, &game).await.unwrap();
        let restored = read_match_from_disk(&cache_path, &game.id)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(restored, game);
        tokio::fs::remove_dir_all(&cache_path).await.unwrap();
    }

    #[test]
    fn match_cache_rejects_unsafe_match_ids() {
        assert!(match_cache_file_path(Path::new("cache"), "../match").is_none());
        assert!(match_cache_file_path(Path::new("cache"), "match/id").is_none());
        assert!(match_cache_file_path(Path::new("cache"), "safe-match_id").is_some());
    }

    #[test]
    fn match_hydration_retry_delay_is_bounded_and_honors_server_hint() {
        assert_eq!(hydration_retry_delay(0, None), Some(Duration::from_secs(2)));
        assert_eq!(hydration_retry_delay(1, None), Some(Duration::from_secs(4)));
        assert_eq!(
            hydration_retry_delay(0, Some(Duration::from_secs(12))),
            Some(Duration::from_secs(12))
        );
        assert_eq!(
            hydration_retry_delay(0, Some(Duration::from_secs(31))),
            None
        );
    }

    #[test]
    fn public_profile_maps_to_existing_stats_model() {
        let profile: PublicProfile = serde_json::from_str(
            r#"
            {
              "privacy_mode": "public",
              "total_matches": 2361,
              "ranks": {
                "leetify": 2,
                "premier": 19309,
                "faceit": 9,
                "faceit_elo": null,
                "wingman": 17,
                "renown": 16482,
                "competitive": [{"map_name": "de_nuke", "rank": 14}]
              },
              "rating": {
                "aim": 60.2568,
                "positioning": 57.1424,
                "utility": 70.1944,
                "clutch": 0.0938,
                "opening": -0.0024,
                "ct_leetify": 0.0176,
                "t_leetify": 0.0248
              },
              "recent_matches": [{
                "id": "match-1",
                "finished_at": "2026-01-01T12:00:00Z",
                "data_source": "matchmaking",
                "outcome": "win",
                "rank": 19309,
                "rank_type": 11,
                "map_name": "de_nuke",
                "leetify_rating": 0.05,
                "score": [13, 9],
                "preaim": 10.0,
                "reaction_time_ms": 500.0,
                "accuracy_enemy_spotted": 30.0,
                "accuracy_head": 20.0,
                "spray_accuracy": 40.0
              }]
            }
            "#,
        )
        .expect("public profile fixture should deserialize");

        let mini: LeetifyMiniProfile = profile.into();

        assert_eq!(mini.ratings.games_played, 2361);
        assert_eq!(mini.ratings.aim, 60.2568);
        assert_eq!(mini.ratings.ct_leetify, 0.0176);
        assert_eq!(mini.ratings.t_leetify, 0.0248);
        assert_eq!(mini.ranks[0].r#type.as_deref(), Some("premier"));
        assert_eq!(mini.ranks[0].skill_level, Some(19309));
        assert_eq!(
            mini.ranks[1].data_source.as_deref(),
            Some("matchmaking_wingman")
        );
        assert!(matches!(mini.recent_matches[0].result, MatchResult::Win));
    }

    #[test]
    fn public_match_maps_scores_and_flash_fields() {
        let game: PublicMatch = serde_json::from_str(
            r#"
            {
              "id": "match-2",
              "finished_at": "2026-01-02T12:00:00Z",
              "data_source": "matchmaking",
              "data_source_match_id": "share-code",
              "map_name": "de_mirage",
              "has_banned_player": false,
              "team_scores": [
                {"team_number": 2, "score": 13},
                {"team_number": 3, "score": 9}
              ],
              "stats": [{
                "steam64_id": "76561198016607756",
                "name": "fixture",
                "initial_team_number": 3,
                "flashbang_hit_friend": 4,
                "flashbang_thrown": 8,
                "rounds_count": 22
              }]
            }
            "#,
        )
        .expect("public match fixture should deserialize");

        let game = public_match_to_game(game, &SteamID::new(PUBLIC_TEST_STEAM_ID.to_string()))
            .expect("match should contain player stats");

        assert_eq!(game.id.as_deref(), Some("match-2"));
        assert_eq!(game.scores, (9, 13));
        assert_eq!(game.match_result, "loss");
        assert_eq!(game.teammates_flashed, Some(4));
        assert_eq!(game.flashbangs_thrown, Some(8));
        assert_eq!(game.rounds_count, Some(22));
    }

    #[test]
    fn public_match_rejects_a_missing_requested_player() {
        let game = PublicMatch {
            id: "match-without-requested-player".to_string(),
            finished_at: Utc::now(),
            map_name: "de_mirage".to_string(),
            team_scores: vec![
                PublicTeamScore {
                    team_number: 2,
                    score: 13,
                },
                PublicTeamScore {
                    team_number: 3,
                    score: 9,
                },
            ],
            stats: vec![PublicPlayerMatchStats {
                steam64_id: "another-player".to_string(),
                initial_team_number: 2,
                flashbang_hit_friend: 0,
                flashbang_thrown: 0,
                rounds_count: 22,
            }],
        };

        assert!(
            public_match_to_game(game, &SteamID::new("requested-player".to_string())).is_none()
        );
    }

    #[test]
    fn public_match_hydrates_two_complete_teams() {
        let stats = (0..5)
            .flat_map(|index| {
                [("a", 2), ("b", 3)].map(|(prefix, initial_team_number)| PublicPlayerMatchStats {
                    steam64_id: format!("{prefix}-{index}"),
                    initial_team_number,
                    flashbang_hit_friend: 0,
                    flashbang_thrown: 0,
                    rounds_count: 22,
                })
            })
            .collect();
        let game = PublicMatch {
            id: "complete-match".to_string(),
            finished_at: Utc::now(),
            map_name: "de_mirage".to_string(),
            team_scores: vec![
                PublicTeamScore {
                    team_number: 2,
                    score: 13,
                },
                PublicTeamScore {
                    team_number: 3,
                    score: 9,
                },
            ],
            stats,
        };

        let hydrated = public_match_to_match(game).expect("complete match should hydrate");

        assert_eq!(hydrated.teams.len(), 2);
        assert_eq!(hydrated.teams[0].steam64_ids.len(), 5);
        assert_eq!(hydrated.teams[0].score, 13);
        assert_eq!(hydrated.teams[1].steam64_ids.len(), 5);
        assert_eq!(hydrated.teams[1].score, 9);
    }

    #[test]
    fn public_match_does_not_treat_player_scoped_stats_as_a_complete_roster() {
        let game = PublicMatch {
            id: "player-scoped-match".to_string(),
            finished_at: Utc::now(),
            map_name: "de_mirage".to_string(),
            team_scores: vec![
                PublicTeamScore {
                    team_number: 2,
                    score: 13,
                },
                PublicTeamScore {
                    team_number: 3,
                    score: 9,
                },
            ],
            stats: vec![PublicPlayerMatchStats {
                steam64_id: PUBLIC_TEST_STEAM_ID.to_string(),
                initial_team_number: 2,
                flashbang_hit_friend: 0,
                flashbang_thrown: 0,
                rounds_count: 22,
            }],
        };

        assert!(public_match_to_match(game).is_none());
    }

    #[test]
    fn average_recent_match_duration_uses_the_newest_thirty_matches() {
        let make_game = |id: String, minutes_ago: i64, rounds_count: u32| LeetifyGame {
            id: Some(id),
            own_team_steam64_ids: vec![],
            game_finished_at: Utc::now() - chrono::Duration::minutes(minutes_ago),
            map_name: "de_nuke".to_string(),
            match_result: "win".to_string(),
            scores: (13, 9),
            skill_level: None,
            teammates_flashed: None,
            flashbangs_thrown: None,
            rounds_count: Some(rounds_count),
        };

        let mut games: Vec<_> = (0..30)
            .map(|index| make_game(format!("recent-{index}"), index, 24))
            .collect();
        games.push(make_game("old-outlier".to_string(), 31, 99));

        let estimate = average_recent_match_duration(&games).expect("recent matches should exist");

        assert_eq!(estimate.matches_used, RECENT_MATCHES_LIMIT);
        assert_eq!(estimate.average_rounds, 24.0);
        assert_eq!(estimate.minutes, 48.0);
    }

    #[test]
    fn average_recent_match_duration_ignores_matches_without_round_data() {
        let game = LeetifyGame {
            id: Some("missing-rounds".to_string()),
            own_team_steam64_ids: vec![],
            game_finished_at: Utc::now(),
            map_name: "de_nuke".to_string(),
            match_result: "win".to_string(),
            scores: (13, 9),
            skill_level: None,
            teammates_flashed: None,
            flashbangs_thrown: None,
            rounds_count: None,
        };

        assert!(average_recent_match_duration(&[game]).is_none());
    }

    #[test]
    fn teammate_records_use_own_roster_count_ties_deduplicate_and_limit_to_five() {
        let own = SteamID::new("own".into());
        let mut mappings = HashMap::from([
            (Username::new("player".into()), own.clone()),
            (Username::new("Rival".into()), SteamID::new("rival".into())),
        ]);
        let names = [
            "Alpha", "Beta", "Charlie", "Delta", "Echo", "Foxtrot", "Golf",
        ];
        for name in names {
            mappings.insert(Username::new(name.into()), SteamID::new(name.into()));
        }
        let mut games = Vec::new();
        let mut matches = HashMap::new();
        for i in 0..35 {
            let id = format!("match-{i}");
            let finished = Utc::now() - chrono::Duration::days(i);
            let mut team = vec![own.clone(), SteamID::new("Alpha".into())];
            if i < 10 {
                team.push(SteamID::new("Beta".into()));
            }
            if i < 2 {
                team.push(SteamID::new("Charlie".into()));
            }
            if (3..=6).contains(&i) {
                team.push(SteamID::new(names[i as usize].into()));
            }
            while team.len() < 5 {
                team.push(SteamID::new(format!("unknown-{}", team.len())));
            }
            let own_score = match i % 3 {
                0 => 12,
                1 => 9,
                _ => 13,
            };
            games.push(LeetifyGame {
                id: Some(id.clone()),
                own_team_steam64_ids: vec![],
                game_finished_at: finished,
                map_name: "de_nuke".into(),
                match_result: "win".into(),
                scores: (13, 9),
                skill_level: None,
                teammates_flashed: None,
                flashbangs_thrown: None,
                rounds_count: None,
            });
            matches.insert(
                id.clone(),
                LeetifyMatch {
                    id,
                    game_finished_at: finished,
                    map_name: "de_nuke".into(),
                    teams: vec![
                        LeetifyMatchTeam {
                            steam64_ids: team,
                            score: own_score,
                        },
                        LeetifyMatchTeam {
                            steam64_ids: ["rival", "o1", "o2", "o3", "o4"]
                                .map(|name| SteamID::new(name.into()))
                                .to_vec(),
                            score: 12,
                        },
                    ],
                },
            );
        }
        // A repeated profile-history entry must not consume a window slot or be counted twice.
        games.push(games[0].clone());
        let stats = teammate_stats_from_matches(&games, &matches, &mappings, &own);
        assert_eq!(stats.matches_considered, 30);
        assert_eq!(
            stats
                .entries
                .iter()
                .map(|entry| entry.username.to_string())
                .collect::<Vec<_>>(),
            names[..5]
        );
        let first = &stats.entries[0];
        assert_eq!(
            (first.wins, first.losses, first.ties, first.games()),
            (10, 10, 10, 30)
        );
        assert_eq!(stats.entries[1].games(), 10);
        assert_eq!(stats.entries[2].games(), 2);
        assert_eq!((stats.entries[3].ties, stats.entries[3].games()), (1, 1));
        assert!(!stats
            .entries
            .iter()
            .any(|entry| entry.username.to_string() == "Rival"));
    }

    #[tokio::test]
    #[ignore = "requires network access to Leetify"]
    async fn public_api_returns_profile_and_matches_for_test_account() {
        let settings = Settings {
            teloxide: TeloxideSettings {
                bot_api_token: "test".to_string(),
            },
            players: PlayersSettings {
                steamid_mappings: HashMap::new(),
            },
            weather: None,
            leetify: None,
            electricity: ElectricitySettings::default(),
        };
        let steam_id = SteamID::new(PUBLIC_TEST_STEAM_ID.to_string());

        let profile = get_leetify_profile(&settings, &steam_id)
            .await
            .expect("public profile request should succeed");
        assert_eq!(profile.privacy_mode, "public");

        let matches = LeetifyClient::from_settings(&settings)
            .get::<Vec<PublicMatch>>("/v3/profile/matches", &steam_id)
            .await
            .expect("public match history request should succeed");
        assert!(!matches.is_empty());

        let match_id = matches[0].id.clone();
        let details = LeetifyClient::from_settings(&settings)
            .get_path::<PublicMatch>(&format!("/v2/matches/{match_id}"))
            .await
            .expect("public match detail request should succeed");
        let hydrated = public_match_to_match(details)
            .expect("public match detail should contain two complete teams");
        assert_eq!(hydrated.id, match_id);
    }
}
