use cached::proc_macro::cached;
use chrono::{DateTime, Utc};
use color_eyre::{eyre::eyre, Result};
use futures::StreamExt;
use serde::Deserialize;
use std::fmt::Display;

use crate::{
    settings::Settings,
    types::{SteamID, Username},
};

fn unwrap_or_log<T, E: Display>(result: std::result::Result<T, E>, err_context: &str) -> Option<T> {
    match result {
        Ok(value) => Some(value),
        Err(e) => {
            eprintln!("{err_context}: {e}");
            None
        }
    }
}

#[cached(time = 300)]
async fn get_leetify_stats(steam_id: SteamID) -> Option<serde_json::Value> {
    println!("Fetching Leetify stats for SteamID {steam_id}");

    let url = format!("https://api.leetify.com/api/profile/{steam_id}");
    let err_context = format!("Error while fetching {url}");
    let resp = unwrap_or_log(reqwest::get(&url).await, &err_context)?;
    let resp = unwrap_or_log(resp.error_for_status(), &err_context)?;

    unwrap_or_log(resp.json().await, &err_context)?
}

#[cached(time = 300)]
pub async fn get_leetify_mini_profile(steam_id: SteamID) -> Option<LeetifyMiniProfile> {
    println!("Fetching Leetify mini profile for SteamID {steam_id}");

    let url = format!("https://api.leetify.com/api/mini-profiles/{steam_id}");
    let err_context = format!("Error while fetching {url}");
    let resp = unwrap_or_log(reqwest::get(&url).await, &err_context)?;
    let resp = unwrap_or_log(resp.error_for_status(), &err_context)?;

    unwrap_or_log(resp.json().await, &err_context)?
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
    pub own_team_steam64_ids: Vec<SteamID>,
    pub game_finished_at: DateTime<Utc>,
    pub map_name: String,
    pub match_result: String,
    pub scores: (u32, u32),
    pub skill_level: Option<u32>,
}

#[derive(Debug)]
pub struct LastPlayedResult {
    pub game: LeetifyGame,
    pub spree: usize,
}

pub fn last_played_from_leetify_stats(
    settings: &Settings,
    own_steam_id: &SteamID,
    resp: &serde_json::Value,
) -> Result<LastPlayedResult> {
    let games_field = resp
        .get("games")
        .ok_or_else(|| eyre!("Could not find any games in Leetify response"))?;
    let games = serde_json::from_value::<Vec<LeetifyGame>>(games_field.clone())?;

    let team_steam_ids: Vec<&SteamID> = settings
        .players
        .steamid_mappings
        .values()
        .filter(|steam_id| *steam_id != own_steam_id)
        .collect();

    let mut games_with_teammates = games
        .iter()
        .filter(|game| {
            let has_teammate = team_steam_ids
                .iter()
                .any(|steam_id| game.own_team_steam64_ids.contains(steam_id));

            has_teammate
        })
        .collect::<Vec<_>>();

    let last_played_with_teammate = games_with_teammates
        .iter()
        .max_by_key(|game| game.game_finished_at)
        .ok_or_else(|| eyre!("Could not find any games played with teammates"))
        .cloned()?;

    games_with_teammates.dedup_by_key(|g| g.game_finished_at.date_naive());

    let now = Utc::now().date_naive();
    let spree = if (now - last_played_with_teammate.game_finished_at.date_naive()).num_days() > 1 {
        0
    } else {
        games_with_teammates
            .windows(2)
            .enumerate()
            .find(|(_, pair)| {
                let (next, prev) = (pair[0], pair[1]);

                let days_between = (next.game_finished_at.date_naive()
                    - prev.game_finished_at.date_naive())
                .num_days();

                days_between > 1
            })
            .map(|(idx, _)| idx + 1)
            .unwrap_or_default()
    };

    let result = LastPlayedResult {
        game: (*last_played_with_teammate).clone(),
        spree,
    };

    Ok(result)
}

pub async fn last_played(settings: &Settings, username: &Username) -> Result<LeetifyGame> {
    let steamid = steamid_for_username(settings.clone(), username)
        .ok_or_else(|| eyre!(format!("No SteamID configured for user {username}")))?;

    let resp = get_leetify_stats(steamid.clone())
        .await
        .ok_or_else(|| eyre!("Failed to fetch last played stats from Leetify"))?;

    let result = last_played_from_leetify_stats(settings, &steamid, &resp)?;

    Ok(result.game)
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LeetifyStats {
    pub aim: f32,
    pub positioning: f32,
    pub utility: f32,
    pub games_played: u32,
    pub leetify_rating_rounds: u32,
    pub clutch: f32,
    pub ct_leetify: f32,
    pub leetify: f32,
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

    let mini_profile = get_leetify_mini_profile(steamid.clone())
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
    let steamid_mappings = settings.players.steamid_mappings.clone();

    let futures: Vec<_> = steamid_mappings
        .into_iter()
        .map(|(username, steamid)| {
            let settings = settings.clone();

            async move {
                let resp = get_leetify_stats(steamid.clone()).await;

                let Some(resp) = resp else {
                    eprintln!("Failed to fetch Leetify stats for player {username}");

                    return None;
                };

                let result = last_played_from_leetify_stats(&settings, &steamid, &resp);

                match result {
                    Ok(result) => Some(HallOfShameEntry {
                        username: username.clone(),
                        last_played: result.game.game_finished_at,
                        spree: result.spree,
                    }),
                    Err(e) => {
                        eprintln!(
                            "Failed to fetch last played stats from Leetify for player {username}: {:?}",
                            e
                        );

                        None
                    }
                }
            }
        })
        .collect();

    // create a buffered stream that will execute up to 3 futures in parallel
    // (without preserving the order of the results)
    let stream = futures::stream::iter(futures).buffer_unordered(3);

    // wait for all futures to complete
    let tasks_results = stream.collect::<Vec<_>>().await;

    let mut entries: Vec<HallOfShameEntry> = tasks_results.into_iter().flatten().collect();

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

            async move {
                let resp = get_leetify_mini_profile(steamid.clone()).await;

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
        entries.retain(|entry| entry.skill_level > 1000);
    }

    entries.sort_by_key(|entry| entry.skill_level);
    entries.reverse();

    let avg_skill_level =
        entries.iter().map(|entry| entry.skill_level).sum::<u32>() as f32 / entries.len() as f32;

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
