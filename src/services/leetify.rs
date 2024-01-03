use anyhow::{Context, Result};
use cached::proc_macro::cached;
use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::{
    settings::Settings,
    types::{SteamID, Username},
};

#[cached(result = true, time = 3600, sync_writes = true)]
pub async fn get_leetify_stats(steam_id: SteamID) -> Result<serde_json::Value> {
    println!("Fetching Leetify stats for SteamID {steam_id}");
    let url = format!("https://api.leetify.com/api/profile/{steam_id}");
    let resp = reqwest::get(&url).await?.json().await?;
    Ok(resp)
}

pub fn steamid_for_username(settings: Settings, username: &Username) -> Option<SteamID> {
    let steamid_mappings = settings.players.steamid_mappings;
    let steamid = steamid_mappings.get(username);
    steamid.cloned()
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LeetifyGame {
    pub own_team_steam64_ids: Vec<SteamID>,
    pub game_finished_at: DateTime<Utc>,
    pub map_name: String,
    pub match_result: String,
    pub scores: (u32, u32),
    pub skill_level: Option<u32>,
}

pub fn last_played_from_leetify_stats(
    settings: &Settings,
    own_steam_id: &SteamID,
    resp: &serde_json::Value,
) -> Result<LeetifyGame> {
    let games_field = resp
        .get("games")
        .context("Could not find any games in Leetify response")?;
    let games = serde_json::from_value::<Vec<LeetifyGame>>(games_field.clone())?;

    let team_steam_ids: Vec<&SteamID> = settings
        .players
        .steamid_mappings
        .values()
        .filter(|steam_id| *steam_id != own_steam_id)
        .collect();

    let last_played_with_teammate = games
        .iter()
        .filter(|game| {
            let has_teammate = team_steam_ids
                .iter()
                .any(|steam_id| game.own_team_steam64_ids.contains(steam_id));

            has_teammate
        })
        .max_by_key(|game| game.game_finished_at)
        .context("Could not find any games played with teammates")?;

    Ok(last_played_with_teammate.clone())
}

pub async fn last_played(settings: &Settings, username: &Username) -> Result<LeetifyGame> {
    let steamid = steamid_for_username(settings.clone(), username)
        .context(format!("No SteamID configured for user {username}"))?;

    let resp = get_leetify_stats(steamid.clone())
        .await
        .context("Failed to fetch last played stats from Leetify")?;

    let game = last_played_from_leetify_stats(settings, &steamid, &resp)?;

    Ok(game)
}

#[derive(Clone, Deserialize)]
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

pub async fn player_stats(settings: &Settings, username: &Username) -> Result<LeetifyStats> {
    let steamid = steamid_for_username(settings.clone(), username)
        .context(format!("No SteamID configured for user {username}"))?;

    let resp = get_leetify_stats(steamid.clone())
        .await
        .context("Failed to fetch Leetify stats")?;

    let stats_value = resp
        .get("recentGameRatings")
        .context("No recent Leetify stats found")?;
    let mut stats = serde_json::from_value::<LeetifyStats>(stats_value.clone())?;
    let game = last_played_from_leetify_stats(settings, &steamid, &resp);

    if let Ok(game) = game {
        stats.skill_level = game.skill_level;
    }

    Ok(stats)
}

pub struct HallOfShameEntry {
    pub username: Username,
    pub last_played: DateTime<Utc>,
}

pub async fn hall_of_shame(settings: &Settings) -> Result<Vec<HallOfShameEntry>> {
    let mut entries = vec![];

    for (username, steamid) in settings.players.steamid_mappings.iter() {
        let resp = get_leetify_stats(steamid.clone()).await;

        let Ok(resp) = resp else {
            eprintln!(
                "Failed to fetch Leetify stats for player {username}: {:?}",
                resp
            );
            continue;
        };

        let game = last_played_from_leetify_stats(settings, steamid, &resp);

        match game {
            Ok(game) => {
                entries.push(HallOfShameEntry {
                    username: username.clone(),
                    last_played: game.game_finished_at,
                });
            }
            Err(e) => {
                eprintln!(
                    "Failed to fetch last played stats from Leetify for player {username}: {:?}",
                    e
                );
            }
        }
    }

    entries.sort_by_key(|entry| entry.last_played);

    Ok(entries)
}

pub struct HallOfFameEntry {
    pub username: Username,
    pub last_played: DateTime<Utc>,
    pub skill_level: u32,
}

pub struct HallOfFame {
    pub entries: Vec<HallOfFameEntry>,
    pub avg_skill_level: f32,
    pub median_skill_level: u32,
}

/// List top 10 players based on their skill level in their most recent game
pub async fn hall_of_fame(settings: &Settings) -> Result<HallOfFame> {
    let mut entries = vec![];

    for (username, steamid) in settings.players.steamid_mappings.iter() {
        let resp = get_leetify_stats(steamid.clone()).await;

        let Ok(resp) = resp else {
            eprintln!(
                "Failed to fetch Leetify stats for player {username}: {:?}",
                resp
            );
            continue;
        };

        let game = last_played_from_leetify_stats(settings, steamid, &resp);

        match game {
            Ok(game) => {
                entries.push(HallOfFameEntry {
                    username: username.clone(),
                    last_played: game.game_finished_at,
                    skill_level: game.skill_level.unwrap_or(0),
                });
            }
            Err(e) => {
                eprintln!(
                    "Failed to fetch last played stats from Leetify for player {username}: {:?}",
                    e
                );
            }
        }
    }

    // Don't include players with no rank or old CSGO premier rank
    entries.retain(|entry| entry.skill_level > 1000);

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
