use cached::proc_macro::cached;
use chrono::{DateTime, Utc};
use color_eyre::{eyre::eyre, Result};
use futures::StreamExt;
use serde::{de::DeserializeOwned, Deserialize};
use std::{
    collections::{HashMap, HashSet},
    fmt::Display,
};

use crate::{
    settings::Settings,
    types::{SteamID, Username},
};

const LEETIFY_API_BASE_URL: &str = "https://api-public.cs-prod.leetify.com";

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
            request = request.header(reqwest::header::AUTHORIZATION, format!("Bearer {api_key}"));
        }

        Ok(request.send().await?.error_for_status()?.json().await?)
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
    rounds_count: u32,
}

async fn get_leetify_profile(settings: &Settings, steam_id: &SteamID) -> Result<PublicProfile> {
    LeetifyClient::from_settings(settings)
        .get("/v3/profile", steam_id)
        .await
}

pub async fn get_leetify_mini_profile(
    settings: &Settings,
    steam_id: SteamID,
) -> Option<LeetifyMiniProfile> {
    println!("Fetching Leetify profile for SteamID {steam_id}");

    let url = format!("{LEETIFY_API_BASE_URL}/v3/profile?steam64_id={steam_id}");
    let err_context = format!("Error while fetching {url}");
    let profile = unwrap_or_log(get_leetify_profile(settings, &steam_id).await, &err_context)?;

    if profile.privacy_mode != "public" {
        eprintln!("Leetify profile for SteamID {steam_id} is private");
        return None;
    }

    Some(profile.into())
}

fn public_match_to_game(game: PublicMatch, steam_id: &SteamID) -> Option<LeetifyGame> {
    let player = game
        .stats
        .iter()
        .find(|player| player.steam64_id == steam_id.to_string())
        .or_else(|| game.stats.first())?;

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
        rounds_count: Some(player.rounds_count),
    })
}

#[cached(
    time = 300,
    result = true,
    key = "SteamID",
    convert = r#"{ steam_id.clone() }"#
)]
async fn get_leetify_games_cached(
    client: LeetifyClient,
    steam_id: SteamID,
) -> Result<Vec<LeetifyGame>> {
    let url = format!("{LEETIFY_API_BASE_URL}/v3/profile/matches?steam64_id={steam_id}");
    let err_context = format!("Error while fetching {url}");
    let matches = client
        .get::<Vec<PublicMatch>>("/v3/profile/matches", &steam_id)
        .await
        .map_err(|error| eyre!("{err_context}: {error}"))?;

    Ok(matches
        .into_iter()
        .filter_map(|game| public_match_to_game(game, &steam_id))
        .collect())
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
    pub own_team_steam64_ids: Vec<SteamID>,
    pub game_finished_at: DateTime<Utc>,
    pub map_name: String,
    pub match_result: String,
    pub scores: (u32, u32),
    pub skill_level: Option<u32>,
    #[serde(default)]
    pub teammates_flashed: Option<u32>,
    #[serde(default)]
    pub rounds_count: Option<u32>,
}

#[derive(Debug)]
pub struct LastPlayedResult {
    pub game: LeetifyGame,
    pub spree: usize,
}

pub fn last_played_from_leetify_games(games: &[LeetifyGame]) -> Result<LastPlayedResult> {
    let mut games = games.iter().collect::<Vec<_>>();
    games.sort_by_key(|game| std::cmp::Reverse(game.game_finished_at));

    let last_played = games
        .first()
        .map(|game| (*game).clone())
        .ok_or_else(|| eyre!("Could not find any Leetify games"))?;

    let mut unique_dates = Vec::new();
    for game in games {
        let date = game.game_finished_at.date_naive();
        if unique_dates.last() != Some(&date) {
            unique_dates.push(date);
        }
    }

    let today = Utc::now().date_naive();
    let spree = if (today - unique_dates[0]).num_days() > 1 {
        0
    } else {
        unique_dates
            .windows(2)
            .position(|pair| (pair[0] - pair[1]).num_days() > 1)
            .map(|index| index + 1)
            .unwrap_or(unique_dates.len())
    };

    Ok(LastPlayedResult {
        game: last_played,
        spree,
    })
}

pub fn last_played_from_team_games(
    games: &[LeetifyGame],
    teammate_games: &[Vec<LeetifyGame>],
) -> Result<LastPlayedResult> {
    let teammate_match_ids: HashSet<&str> = teammate_games
        .iter()
        .flat_map(|games| games.iter())
        .filter_map(|game| game.id.as_deref())
        .collect();

    let games_with_teammates: Vec<LeetifyGame> = games
        .iter()
        .filter(|game| {
            game.id
                .as_deref()
                .is_some_and(|id| teammate_match_ids.contains(id))
        })
        .cloned()
        .collect();

    if games_with_teammates.is_empty() {
        return Err(eyre!(
            "Could not find any Leetify games played with teammates"
        ));
    }

    last_played_from_leetify_games(&games_with_teammates)
}

pub async fn last_played(settings: &Settings, username: &Username) -> Result<LeetifyGame> {
    let steamid = steamid_for_username(settings.clone(), username)
        .ok_or_else(|| eyre!(format!("No SteamID configured for user {username}")))?;

    let configured_games = get_configured_player_games(settings).await;
    let games = configured_games
        .get(&steamid)
        .ok_or_else(|| eyre!("Failed to fetch last played stats from Leetify"))?;
    let teammate_games = configured_games
        .iter()
        .filter(|(other_steamid, _)| *other_steamid != &steamid)
        .map(|(_, games)| games.clone())
        .collect::<Vec<_>>();

    Ok(last_played_from_team_games(games, &teammate_games)?.game)
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
        .ok_or_else(|| eyre!(format!("No SteamID configured for user {username}")))?;

    let mini_profile = get_leetify_mini_profile(settings, steamid.clone())
        .await
        .ok_or_else(|| eyre!("Failed to fetch last played stats from Leetify"))?;

    Ok(mini_profile)
}

pub struct HallOfShameEntry {
    pub username: Username,
    pub last_played: DateTime<Utc>,
    pub spree: usize,
}

pub async fn hall_of_shame(settings: &Settings) -> Result<Vec<HallOfShameEntry>> {
    let configured_games = get_configured_player_games(settings).await;
    let mut entries = Vec::new();

    for (username, steamid) in &settings.players.steamid_mappings {
        let Some(games) = configured_games.get(steamid) else {
            eprintln!("Failed to fetch Leetify stats for player {username}");
            continue;
        };

        let teammate_games = configured_games
            .iter()
            .filter(|(other_steamid, _)| *other_steamid != steamid)
            .map(|(_, games)| games.clone())
            .collect::<Vec<_>>();

        match last_played_from_team_games(games, &teammate_games) {
            Ok(result) => entries.push(HallOfShameEntry {
                username: username.clone(),
                last_played: result.game.game_finished_at,
                spree: result.spree,
            }),
            Err(e) => eprintln!("Failed to find team match for player {username}: {e}"),
        }
    }

    entries.sort_by_key(|entry| (entry.last_played, entry.spree));

    Ok(entries)
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

/// List top 10 players based on their skill level in their most recent game
pub async fn hall_of_fame(settings: &Settings, rank_type: &String) -> Result<HallOfFame> {
    let steamid_mappings = settings.players.steamid_mappings.clone();

    let futures: Vec<_> = steamid_mappings
        .into_iter()
        .map(|(username, steamid)| {
            let rank_type = rank_type.clone();
            let settings = settings.clone();

            async move {
                let resp = get_leetify_mini_profile(&settings, steamid.clone()).await;

                let Some(resp) = resp else {
                    eprintln!("Failed to fetch Leetify mini profile for player {username}");

                    return None;
                };

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

                    return None;
                };

                Some(HallOfFameEntry {
                    username: username.clone(),
                    skill_level,
                })
            }
        })
        .collect();

    // create a buffered stream that will execute up to 3 futures in parallel
    // (without preserving the order of the results)
    let stream = futures::stream::iter(futures).buffer_unordered(3);

    // wait for all futures to complete
    let tasks_results = stream.collect::<Vec<_>>().await;

    let mut entries: Vec<HallOfFameEntry> = tasks_results.into_iter().flatten().collect();

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

    let median_skill_level = entries
        .get(entries.len() / 2)
        .map(|entry| entry.skill_level)
        .unwrap_or(0);

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

/// List top 10 players based on a specific stat (aim, positioning, utility, opening, clutch)
pub async fn stat_leaderboard(settings: &Settings, stat_type: &str) -> Result<StatLeaderboard> {
    let steamid_mappings = settings.players.steamid_mappings.clone();

    let futures: Vec<_> = steamid_mappings
        .into_iter()
        .map(|(username, steamid)| {
            let stat_type = stat_type.to_string();
            let settings = settings.clone();

            async move {
                let resp = get_leetify_mini_profile(&settings, steamid.clone()).await;

                let Some(resp) = resp else {
                    eprintln!("Failed to fetch Leetify mini profile for player {username}");

                    return None;
                };

                let stat_value = match stat_type.as_str() {
                    "aim" => resp.ratings.aim,
                    "positioning" => resp.ratings.positioning,
                    "utility" => resp.ratings.utility,
                    "opening" => resp.ratings.opening,
                    "clutch" => resp.ratings.clutch,
                    _ => return None,
                };

                Some(StatLeaderboardEntry {
                    username: username.clone(),
                    stat_value,
                })
            }
        })
        .collect();

    // create a buffered stream that will execute up to 3 futures in parallel
    // (without preserving the order of the results)
    let stream = futures::stream::iter(futures).buffer_unordered(3);

    // wait for all futures to complete
    let tasks_results = stream.collect::<Vec<_>>().await;

    let mut entries: Vec<StatLeaderboardEntry> = tasks_results.into_iter().flatten().collect();

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

    let median = if stat_values.is_empty() {
        0.0
    } else {
        stat_values[stat_values.len() / 2]
    };

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
    pub teammates_flashed_per_round: f32,
}

pub struct TeamFlashLeaderboard {
    pub entries: Vec<TeamFlashEntry>,
    pub avg: f32,
}

/// List players ranked by teammates flashed per round (highest = most team flashes = worst)
pub async fn team_flash_leaderboard(settings: &Settings) -> Result<TeamFlashLeaderboard> {
    let steamid_mappings = settings.players.steamid_mappings.clone();

    let futures: Vec<_> = steamid_mappings
        .into_iter()
        .map(|(username, steamid)| {
            let settings = settings.clone();

            async move {
                let games = get_leetify_games(&settings, &steamid).await;

                let Some(games) = games else {
                    eprintln!("Failed to fetch Leetify stats for player {username}");
                    return None;
                };

                // The public API exposes total friendly flash hits and round counts per match.
                let (flashes, rounds) =
                    games.iter().fold((0u32, 0u32), |(flashes, rounds), game| {
                        (
                            flashes + game.teammates_flashed.unwrap_or_default(),
                            rounds + game.rounds_count.unwrap_or_default(),
                        )
                    });
                let teammates_flashed = (rounds > 0).then(|| flashes as f32 / rounds as f32);

                let Some(teammates_flashed_per_round) = teammates_flashed else {
                    eprintln!("Failed to find teammatesFlashedPerRound for player {username}");
                    return None;
                };

                Some(TeamFlashEntry {
                    username: username.clone(),
                    teammates_flashed_per_round,
                })
            }
        })
        .collect();

    let stream = futures::stream::iter(futures).buffer_unordered(3);
    let tasks_results = stream.collect::<Vec<_>>().await;

    let mut entries: Vec<TeamFlashEntry> = tasks_results.into_iter().flatten().collect();

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
    let avg = if values.is_empty() {
        0.0
    } else {
        values.iter().sum::<f32>() / values.len() as f32
    };

    Ok(TeamFlashLeaderboard { entries, avg })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{PlayersSettings, TeloxideSettings};
    use std::collections::HashMap;

    const PUBLIC_TEST_STEAM_ID: &str = "76561198016607756";

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
        assert_eq!(game.rounds_count, Some(22));
    }

    #[test]
    fn last_played_uses_newest_match_and_calculates_spree() {
        let make_game = |finished_at: DateTime<Utc>| LeetifyGame {
            id: None,
            own_team_steam64_ids: vec![],
            game_finished_at: finished_at,
            map_name: "de_nuke".to_string(),
            match_result: "win".to_string(),
            scores: (13, 9),
            skill_level: None,
            teammates_flashed: None,
            rounds_count: None,
        };
        let today = Utc::now().date_naive();
        let games = vec![
            make_game(today.and_hms_opt(12, 0, 0).unwrap().and_utc()),
            make_game(
                (today - chrono::Days::new(2))
                    .and_hms_opt(12, 0, 0)
                    .unwrap()
                    .and_utc(),
            ),
            make_game(
                (today - chrono::Days::new(1))
                    .and_hms_opt(12, 0, 0)
                    .unwrap()
                    .and_utc(),
            ),
        ];

        let result = last_played_from_leetify_games(&games).unwrap();

        assert_eq!(result.game.game_finished_at.date_naive(), today);
        assert_eq!(result.spree, 3);
    }

    #[test]
    fn last_played_with_team_matches_public_match_ids() {
        let make_game = |id: &str, days_ago: u64| LeetifyGame {
            id: Some(id.to_string()),
            own_team_steam64_ids: vec![],
            game_finished_at: (Utc::now().date_naive() - chrono::Days::new(days_ago))
                .and_hms_opt(12, 0, 0)
                .unwrap()
                .and_utc(),
            map_name: "de_nuke".to_string(),
            match_result: "win".to_string(),
            scores: (13, 9),
            skill_level: None,
            teammates_flashed: None,
            rounds_count: None,
        };

        let own_games = vec![
            make_game("solo-match", 0),
            make_game("team-match-new", 1),
            make_game("team-match-old", 3),
        ];
        let teammate_games = vec![make_game("team-match-new", 1)];

        let result = last_played_from_team_games(&own_games, &[teammate_games]).unwrap();

        assert_eq!(result.game.id.as_deref(), Some("team-match-new"));
    }

    #[test]
    fn last_played_with_team_rejects_games_without_a_shared_match() {
        let make_game = |id: &str| LeetifyGame {
            id: Some(id.to_string()),
            own_team_steam64_ids: vec![],
            game_finished_at: Utc::now(),
            map_name: "de_nuke".to_string(),
            match_result: "win".to_string(),
            scores: (13, 9),
            skill_level: None,
            teammates_flashed: None,
            rounds_count: None,
        };

        let result = last_played_from_team_games(
            &[make_game("own-match")],
            &[vec![make_game("teammate-match")]],
        );

        assert!(result.is_err());
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
    }
}
