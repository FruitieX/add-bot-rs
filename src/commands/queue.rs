use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use chrono::{DateTime, NaiveTime, TimeZone, Timelike, Utc};
use chrono_tz::Tz;
use futures::StreamExt;
use teloxide::{types::ChatId, Bot};

use crate::{
    services,
    settings::Settings,
    state::{AddRemovePlayerOp, AddRemovePlayerResult, Queue, State, QUEUE_SIZE},
    state_container::StateContainer,
    types::{QueueId, SteamID, Username},
    util::{fmt_naive_time, mk_players_str, mk_queue_status_msg, send_msg},
};

static INSTANT_QUEUE_TIMEOUT_MINUTES: i64 = 30;

/// Called on timed out queues. Removes the chat queue and sends an
/// informational Telegram message.
async fn handle_queue_timeout(
    sc: &StateContainer,
    bot: &Bot,
    chat_id: &ChatId,
    queue_id: &QueueId,
) -> Option<()> {
    let state = sc.read().await;

    // Remove chat queue and write new state.
    let (state, removed_queue) = state.rm_chat_queue(chat_id, queue_id);
    sc.write(state).await;

    let removed_queue = removed_queue?;

    // Inform players on Telegram about the timeout.
    let text = if removed_queue.is_full() {
        let players_str = mk_players_str(&removed_queue, true, false);
        format!("{} queue: It's time to play!\n{}", queue_id, players_str)
    } else {
        let players_str = mk_players_str(&removed_queue, false, false);
        format!("{} queue timed out!\n{}", queue_id, players_str)
    };

    send_msg(bot, chat_id, &text, false).await;

    Some(())
}

/// Task that polls and takes action for any queues that have timed out.
pub async fn poll_for_timeouts(sc: StateContainer, tz: Tz, bot: Bot) {
    loop {
        let state = sc.read().await;
        let t = fmt_naive_time(&Utc::now().with_timezone(&tz).time());

        // Traverse all chat queues and look for timed out queues.
        for (chat_id, chat) in &state.chats {
            for (queue_id, queue) in &chat.queues {
                // Note that we compare only HH:MM timestamps here and poll
                // every second, so we shouldn't miss any timeouts.
                if t == fmt_naive_time(&queue.timeout) {
                    handle_queue_timeout(&sc, &bot, chat_id, queue_id).await;
                }
            }
        }

        // Poll again after 1 second.
        tokio::time::sleep(std::time::Duration::from_secs(1)).await
    }
}

/// Takes a sorted list of queues and returns human-readable strings with queue
/// details.
fn make_queue_strings(queues: Vec<(QueueId, Queue)>) -> Vec<String> {
    queues
        .iter()
        .map(|(queue_id, queue)| {
            let players_str = mk_players_str(queue, false, true);

            format!("{} {} {}", queue_id, players_str, queue.add_cmd)
        })
        .collect()
}

pub async fn add_remove(
    username: Username,
    state: State,
    chat_id: ChatId,
    tz: &Tz,
    time: Option<NaiveTime>,
    sc: &StateContainer,
) -> String {
    // Current time without seconds
    let t_now = NaiveTime::from_hms_opt(
        Utc::now().with_timezone(tz).time().hour(),
        Utc::now().with_timezone(tz).time().minute(),
        0,
    )
    .unwrap();

    // Construct queue_id, timeout and add_cmd based on whether command
    // targeted a timed queue or not.
    let (queue_id, timeout, add_cmd) = match time {
        Some(time) if time != t_now => {
            let queue_id = QueueId::new(fmt_naive_time(&time));
            let add_cmd = time.format("/%H%M").to_string();
            (queue_id, time, add_cmd)
        }
        // Catch current or missing minute commands and redirect to instant queue
        _ => {
            let queue_id = QueueId::new(String::from(""));
            let timeout = Utc::now().with_timezone(tz).time()
                + chrono::Duration::minutes(INSTANT_QUEUE_TIMEOUT_MINUTES);
            let add_cmd = String::from("/add");
            (queue_id, timeout, add_cmd)
        }
    };

    // Add player and update state.
    let (state, result, op) =
        state.add_remove_player(&chat_id, &queue_id, add_cmd, timeout, username);
    sc.write(state.clone()).await;

    // Construct message based on whether the queue is now full or not.
    match result {
        AddRemovePlayerResult::QueueFull(queue) if queue_id.is_instant_queue() => {
            let players_str = mk_players_str(&queue, true, false);
            format!("Match ready in {} queue! {}", queue_id, players_str)
        }
        AddRemovePlayerResult::PlayerQueued(queue)
        | AddRemovePlayerResult::QueueFull(queue)
        | AddRemovePlayerResult::QueueEmpty(queue) => mk_queue_status_msg(&queue, &queue_id, &op),
    }
}

pub async fn remove_all(
    username: Username,
    state: State,
    chat_id: ChatId,
    sc: &StateContainer,
) -> String {
    // Remove player and update state.
    let (state, affected_queues) = state.rm_player(&chat_id, &username);
    sc.write(state.clone()).await;

    // Send queue status message for all affected queues.
    affected_queues
        .iter()
        .map(|(queue_id, queue)| {
            mk_queue_status_msg(
                queue,
                queue_id,
                &AddRemovePlayerOp::PlayerRemoved(username.clone()),
            )
        })
        .collect::<Vec<String>>()
        .join("\n")
}

pub fn list(state: State, chat_id: ChatId, tz: &Tz) -> String {
    let chat = state.chats.get(&chat_id);
    let queues = chat.map(|chat| chat.queues.clone());

    match queues {
        Some(queues) if !queues.is_empty() => {
            let current_time = Utc::now().with_timezone(tz).time();

            let mut queues: Vec<(QueueId, Queue)> = queues.into_iter().collect();
            queues.sort_by(|(_, a), (_, b)| {
                let a_next_day = a.timeout < current_time;
                let b_next_day = b.timeout < current_time;

                if a_next_day == b_next_day {
                    a.timeout.cmp(&b.timeout)
                } else if a_next_day {
                    Ordering::Greater
                } else {
                    Ordering::Less
                }
            });

            make_queue_strings(queues).join("\n")
        }
        _ => String::from("No active queues."),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PredictionResult {
    Win,
    Loss,
    Tie,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TeamObservation {
    match_id: String,
    queued_side: Vec<SteamID>,
    result: PredictionResult,
    current_overlap: usize,
    historical_configured_count: usize,
    configured_outside_count: usize,
    unconfigured_count: usize,
    historical_roster: Vec<SteamID>,
    weight: usize,
    game_finished_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct TeamObservationKey {
    match_id: String,
    queued_side: Vec<SteamID>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct PredictionStats {
    // Composition-weighted totals used by the lineup-aware estimate.
    wins: usize,
    losses: usize,
    // Unweighted totals for the deduplicated historical team observations.
    observed_wins: usize,
    observed_losses: usize,
    observed_ties: usize,
    unique_observations: usize,
    unique_matches: usize,
}

fn observation_weight(
    current_queue_size: usize,
    current_overlap: usize,
    historical_configured_count: usize,
) -> usize {
    let context_score =
        QUEUE_SIZE.saturating_sub(historical_configured_count.abs_diff(current_queue_size));
    current_overlap * context_score
}

#[cfg(test)]
fn select_recent_games(
    games: &[services::leetify::LeetifyGame],
) -> Vec<services::leetify::LeetifyGame> {
    select_recent_games_with_limit(games, services::leetify::RECENT_MATCHES_LIMIT)
}

fn select_recent_games_with_limit(
    games: &[services::leetify::LeetifyGame],
    match_count: usize,
) -> Vec<services::leetify::LeetifyGame> {
    let mut selected = games.to_vec();
    selected.sort_by_key(|game| std::cmp::Reverse(game.game_finished_at));
    selected.truncate(match_count);
    selected
}

fn queued_side_sort_key(side: &[SteamID]) -> String {
    side.iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

#[cfg(test)]
fn test_hydrated_matches(
    histories: &HashMap<SteamID, Vec<services::leetify::LeetifyGame>>,
) -> HashMap<String, services::leetify::LeetifyMatch> {
    let mut games_by_id = HashMap::<String, Vec<services::leetify::LeetifyGame>>::new();
    for games in histories.values() {
        for game in games {
            if let Some(match_id) = game.id.clone() {
                games_by_id.entry(match_id).or_default().push(game.clone());
            }
        }
    }

    games_by_id
        .into_iter()
        .filter_map(|(match_id, games)| {
            let reference = games.first()?.clone();
            let mut sides = Vec::<services::leetify::LeetifyGame>::new();

            for game in games {
                let mut roster = game.own_team_steam64_ids.clone();
                roster.sort_by_key(ToString::to_string);
                roster.dedup();

                if let Some(existing) = sides.iter_mut().find(|existing| {
                    let mut existing_roster = existing.own_team_steam64_ids.clone();
                    existing_roster.sort_by_key(ToString::to_string);
                    existing_roster.dedup();
                    roster.iter().all(|id| existing_roster.contains(id))
                        || existing_roster.iter().all(|id| roster.contains(id))
                }) {
                    if existing.match_result != game.match_result
                        || existing.game_finished_at != game.game_finished_at
                        || existing.map_name != game.map_name
                        || existing.scores != game.scores
                    {
                        return None;
                    }
                    if roster.len() > existing.own_team_steam64_ids.len() {
                        *existing = game;
                    }
                } else {
                    sides.push(game);
                }
            }

            if sides.len() > 2 {
                return None;
            }
            if sides.len() == 2
                && (sides[0].scores.0 != sides[1].scores.1
                    || sides[0].scores.1 != sides[1].scores.0)
            {
                return None;
            }

            let mut teams = sides
                .iter()
                .enumerate()
                .map(|(side_index, side)| {
                    let mut steam64_ids = side.own_team_steam64_ids.clone();
                    steam64_ids.sort_by_key(ToString::to_string);
                    steam64_ids.dedup();
                    while steam64_ids.len() < QUEUE_SIZE {
                        steam64_ids.push(SteamID::new(format!(
                            "test-filler-{match_id}-{side_index}-{}",
                            steam64_ids.len()
                        )));
                    }
                    services::leetify::LeetifyMatchTeam {
                        steam64_ids,
                        score: side.scores.0,
                    }
                })
                .collect::<Vec<_>>();

            if teams.len() == 1 {
                teams.push(services::leetify::LeetifyMatchTeam {
                    steam64_ids: (0..QUEUE_SIZE)
                        .map(|index| SteamID::new(format!("test-opponent-{match_id}-{index}")))
                        .collect(),
                    score: reference.scores.1,
                });
            }

            Some((
                match_id.clone(),
                services::leetify::LeetifyMatch {
                    id: match_id,
                    game_finished_at: reference.game_finished_at,
                    map_name: reference.map_name.clone(),
                    teams,
                },
            ))
        })
        .collect()
}

#[cfg(test)]
fn build_team_observations(
    queue_steam_ids: &[SteamID],
    histories: &HashMap<SteamID, Vec<services::leetify::LeetifyGame>>,
) -> Vec<TeamObservation> {
    let configured_steam_ids = queue_steam_ids.iter().cloned().collect();
    build_team_observations_with_configured(
        queue_steam_ids.len(),
        queue_steam_ids,
        &configured_steam_ids,
        histories,
    )
}

#[cfg(test)]
fn build_team_observations_with_configured(
    current_queue_size: usize,
    queue_steam_ids: &[SteamID],
    configured_steam_ids: &HashSet<SteamID>,
    histories: &HashMap<SteamID, Vec<services::leetify::LeetifyGame>>,
) -> Vec<TeamObservation> {
    let candidate_match_ids = select_candidate_match_ids(
        queue_steam_ids,
        histories,
        services::leetify::RECENT_MATCHES_LIMIT,
    );
    let hydrated_matches = test_hydrated_matches(histories);
    build_team_observations_from_matches(
        current_queue_size,
        queue_steam_ids,
        configured_steam_ids,
        &candidate_match_ids,
        &hydrated_matches,
    )
}

#[cfg(test)]
fn build_team_observations_with_limit(
    current_queue_size: usize,
    queue_steam_ids: &[SteamID],
    configured_steam_ids: &HashSet<SteamID>,
    histories: &HashMap<SteamID, Vec<services::leetify::LeetifyGame>>,
    match_count: usize,
) -> Vec<TeamObservation> {
    let candidate_match_ids = select_candidate_match_ids(queue_steam_ids, histories, match_count);
    let hydrated_matches = test_hydrated_matches(histories);
    build_team_observations_from_matches(
        current_queue_size,
        queue_steam_ids,
        configured_steam_ids,
        &candidate_match_ids,
        &hydrated_matches,
    )
}

/// Select the union of match IDs from each queued player's newest match
/// window. Histories fetched for other active queues cannot introduce matches
/// into this queue's candidate set.
fn select_candidate_match_ids(
    queue_steam_ids: &[SteamID],
    histories: &HashMap<SteamID, Vec<services::leetify::LeetifyGame>>,
    match_count: usize,
) -> HashSet<String> {
    let mut candidate_match_ids = HashSet::new();

    for queried_steam_id in queue_steam_ids {
        let Some(games) = histories.get(queried_steam_id) else {
            continue;
        };
        for game in select_recent_games_with_limit(games, match_count) {
            // A history entry must describe the requested player's perspective.
            // public_match_to_game() enforces this for API data, but keeping the
            // invariant here also protects candidate selection from malformed input.
            if !game.own_team_steam64_ids.contains(queried_steam_id) {
                continue;
            }

            if let Some(match_id) = game.id.filter(|id| !id.is_empty()) {
                candidate_match_ids.insert(match_id);
            }
        }
    }

    candidate_match_ids
}

/// Build one observation per `(match_id, queued_side)` from hydrated match
/// details. The profile-history windows only select candidate IDs; complete
/// historical sides determine lineup overlap and configured-player context.
/// A queued player can therefore contribute to overlap even when their own
/// history did not select or could not fetch that match.
fn build_team_observations_from_matches(
    current_queue_size: usize,
    queue_steam_ids: &[SteamID],
    configured_steam_ids: &HashSet<SteamID>,
    candidate_match_ids: &HashSet<String>,
    hydrated_matches: &HashMap<String, services::leetify::LeetifyMatch>,
) -> Vec<TeamObservation> {
    let queue_steam_ids: HashSet<SteamID> = queue_steam_ids.iter().cloned().collect();
    let mut candidates: HashMap<TeamObservationKey, Vec<TeamObservation>> = HashMap::new();
    let mut sorted_match_ids = candidate_match_ids.iter().collect::<Vec<_>>();
    sorted_match_ids.sort();

    for match_id in sorted_match_ids {
        let Some(game) = hydrated_matches.get(match_id) else {
            continue;
        };
        if game.id != *match_id || game.map_name.is_empty() || game.teams.len() != 2 {
            continue;
        }

        let all_players = game
            .teams
            .iter()
            .flat_map(|team| team.steam64_ids.iter())
            .collect::<HashSet<_>>();
        if game
            .teams
            .iter()
            .any(|team| team.steam64_ids.len() != QUEUE_SIZE)
            || all_players.len() != QUEUE_SIZE * 2
        {
            continue;
        }

        for (team_index, team) in game.teams.iter().enumerate() {
            let opponent = &game.teams[1 - team_index];
            let result = match team.score.cmp(&opponent.score) {
                Ordering::Greater => PredictionResult::Win,
                Ordering::Less => PredictionResult::Loss,
                Ordering::Equal => PredictionResult::Tie,
            };

            let mut historical_roster = team.steam64_ids.clone();
            historical_roster.sort_by_key(ToString::to_string);
            historical_roster.dedup();

            let mut queued_side = historical_roster
                .iter()
                .filter(|steam_id| queue_steam_ids.contains(*steam_id))
                .cloned()
                .collect::<Vec<_>>();
            queued_side.sort_by_key(ToString::to_string);
            if queued_side.is_empty() {
                continue;
            }

            let key = TeamObservationKey {
                match_id: game.id.clone(),
                queued_side: queued_side.clone(),
            };
            let current_overlap = key.queued_side.len();
            let historical_configured_count = historical_roster
                .iter()
                .filter(|steam_id| configured_steam_ids.contains(*steam_id))
                .count();
            let configured_outside_count =
                historical_configured_count.saturating_sub(current_overlap);
            let unconfigured_count = historical_roster
                .len()
                .saturating_sub(historical_configured_count);
            candidates.entry(key).or_default().push(TeamObservation {
                match_id: game.id.clone(),
                queued_side,
                result,
                current_overlap,
                historical_configured_count,
                configured_outside_count,
                unconfigured_count,
                historical_roster,
                weight: observation_weight(
                    current_queue_size,
                    current_overlap,
                    historical_configured_count,
                ),
                game_finished_at: game.game_finished_at,
            });
        }
    }

    let mut observations = candidates
        .into_values()
        .filter_map(|candidates| {
            let reference = candidates.first()?;
            candidates
                .iter()
                .all(|candidate| candidate == reference)
                .then(|| reference.clone())
        })
        .collect::<Vec<_>>();

    observations.sort_by(|left, right| {
        left.match_id
            .cmp(&right.match_id)
            .then_with(|| {
                queued_side_sort_key(&left.queued_side)
                    .cmp(&queued_side_sort_key(&right.queued_side))
            })
            .then_with(|| left.game_finished_at.cmp(&right.game_finished_at))
    });
    observations
}

fn aggregate_prediction(observations: &[TeamObservation]) -> Option<PredictionStats> {
    let mut stats = PredictionStats::default();
    let mut unique_matches = HashSet::new();

    for observation in observations {
        match observation.result {
            PredictionResult::Win => {
                stats.observed_wins += 1;
                stats.wins += observation.weight;
            }
            PredictionResult::Loss => {
                stats.observed_losses += 1;
                stats.losses += observation.weight;
            }
            PredictionResult::Tie => {
                stats.observed_ties += 1;
                continue;
            }
        }

        stats.unique_observations += 1;
        unique_matches.insert(observation.match_id.clone());
    }

    if stats.wins + stats.losses == 0 {
        return None;
    }

    stats.unique_matches = unique_matches.len();
    Some(stats)
}

fn win_percentage(wins: usize, losses: usize) -> f32 {
    if wins + losses == 0 {
        0.0
    } else {
        wins as f32 / (wins + losses) as f32 * 100.0
    }
}

fn format_prediction_line(
    queue_id: &QueueId,
    queue_size: usize,
    player_count: usize,
    available_histories: usize,
    stats: Option<PredictionStats>,
) -> String {
    let coverage = if available_histories < player_count {
        format!(" · {available_histories}/{player_count} histories available")
    } else {
        String::new()
    };

    match stats {
        None => format!(
            "- <b>{queue_id}</b> · <b>{player_count}/{queue_size} players</b>{coverage}\n  Prediction unavailable"
        ),
        Some(stats) => {
            let observed_win_percentage = win_percentage(stats.observed_wins, stats.observed_losses);
            let lineup_aware_win_percentage = win_percentage(stats.wins, stats.losses);

            format!(
                "- <b>{queue_id}</b> · <b>{player_count}/{queue_size} players</b>{coverage}\n  <b>Recent results:</b> {}W / {}L / {}T ({observed_win_percentage:.0}%)\n  <b>Predicted win rate:</b> {lineup_aware_win_percentage:.0}%",
                stats.observed_wins,
                stats.observed_losses,
                stats.observed_ties,
            )
        }
    }
}

async fn fetch_prediction_histories(
    settings: &Settings,
    steam_ids: HashSet<SteamID>,
) -> HashMap<SteamID, Vec<services::leetify::LeetifyGame>> {
    let requests = steam_ids.into_iter().map(|steam_id| {
        let settings = settings.clone();
        async move {
            let games = services::leetify::get_leetify_games(&settings, &steam_id).await;
            if games.is_none() {
                eprintln!("Failed to fetch prediction matches for SteamID {steam_id}");
            }
            (steam_id, games)
        }
    });

    futures::stream::iter(requests)
        .buffer_unordered(5)
        .filter_map(|(steam_id, games)| async move { games.map(|games| (steam_id, games)) })
        .collect::<HashMap<_, _>>()
        .await
}

pub(crate) fn queue_start_at(
    queue_id: &QueueId,
    queue: &Queue,
    now: chrono::DateTime<Tz>,
) -> chrono::DateTime<Tz> {
    if queue_id.is_instant_queue() {
        return now;
    }

    let scheduled = now.date_naive().and_time(queue.timeout);
    let scheduled = now
        .timezone()
        .from_local_datetime(&scheduled)
        .earliest()
        .unwrap_or(now);

    if scheduled < now {
        scheduled + chrono::Duration::days(1)
    } else {
        scheduled
    }
}

pub async fn predictions(
    settings: &Settings,
    state: State,
    chat_id: ChatId,
    tz: &Tz,
    match_count: usize,
) -> String {
    let Some(chat) = state.chats.get(&chat_id) else {
        return "No active queues.".to_string();
    };

    if chat.queues.is_empty() {
        return "No active queues.".to_string();
    }

    let mut queues: Vec<(QueueId, Queue)> = chat
        .queues
        .iter()
        .map(|(queue_id, queue)| (queue_id.clone(), queue.clone()))
        .collect();
    let current_time = Utc::now().with_timezone(tz).time();
    queues.sort_by(|(_, a), (_, b)| {
        let a_next_day = a.timeout < current_time;
        let b_next_day = b.timeout < current_time;

        if a_next_day == b_next_day {
            a.timeout.cmp(&b.timeout)
        } else if a_next_day {
            Ordering::Greater
        } else {
            Ordering::Less
        }
    });

    let usernames: HashSet<Username> = queues
        .iter()
        .flat_map(|(_, queue)| queue.get_players().0)
        .collect();
    let player_steam_ids: HashMap<Username, SteamID> = usernames
        .iter()
        .filter_map(|username| {
            settings
                .players
                .steamid_mappings
                .get(username)
                .cloned()
                .map(|steam_id| (username.clone(), steam_id))
        })
        .collect();
    for username in usernames
        .iter()
        .filter(|username| !player_steam_ids.contains_key(*username))
    {
        eprintln!("No SteamID configured for prediction player {username}");
    }

    let steam_ids = player_steam_ids.values().cloned().collect();
    let histories = fetch_prediction_histories(settings, steam_ids).await;
    let configured_steam_ids = settings
        .players
        .steamid_mappings
        .values()
        .cloned()
        .collect::<HashSet<_>>();

    let queue_prediction_inputs = queues
        .iter()
        .map(|(_, queue)| {
            let queue_steam_ids = queue
                .get_players()
                .0
                .iter()
                .filter_map(|username| player_steam_ids.get(username).cloned())
                .collect::<Vec<_>>();
            let candidate_match_ids =
                select_candidate_match_ids(&queue_steam_ids, &histories, match_count);
            (queue_steam_ids, candidate_match_ids)
        })
        .collect::<Vec<_>>();
    let candidate_match_ids = queue_prediction_inputs
        .iter()
        .flat_map(|(_, match_ids)| match_ids.iter().cloned())
        .collect::<HashSet<_>>();
    let hydrated_matches =
        services::leetify::get_leetify_matches(settings, candidate_match_ids).await;

    let queue_lines = queues
        .iter()
        .zip(queue_prediction_inputs.iter())
        .map(
            |((queue_id, queue), (queue_steam_ids, candidate_match_ids))| {
                let players = queue.get_players().0;
                let observations = build_team_observations_from_matches(
                    players.len(),
                    queue_steam_ids,
                    &configured_steam_ids,
                    candidate_match_ids,
                    &hydrated_matches,
                );
                let available_histories = players
                    .iter()
                    .filter(|username| {
                        player_steam_ids
                            .get(*username)
                            .is_some_and(|steam_id| histories.contains_key(steam_id))
                    })
                    .count();

                format_prediction_line(
                    queue_id,
                    queue.size(),
                    players.len(),
                    available_histories,
                    aggregate_prediction(&observations),
                )
            },
        )
        .collect::<Vec<_>>()
        .join("\n");

    let match_label = if match_count == 1 { "match" } else { "matches" };
    format!(
        "<b>Predicted win rates for current queues</b>\n<i>Based on each player's latest {match_count} {match_label}</i>\n\n{queue_lines}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};

    fn steam_id(id: &str) -> SteamID {
        SteamID::new(id.to_string())
    }

    fn game(
        id: &str,
        own_team: &[&str],
        result: &str,
        minutes_ago: i64,
    ) -> services::leetify::LeetifyGame {
        let scores = match result {
            "win" => (13, 9),
            "loss" => (9, 13),
            _ => (12, 12),
        };

        services::leetify::LeetifyGame {
            id: Some(id.to_string()),
            own_team_steam64_ids: own_team.iter().map(|id| steam_id(id)).collect(),
            game_finished_at: Utc.with_ymd_and_hms(2026, 1, 1, 12, 0, 0).unwrap()
                - Duration::minutes(minutes_ago),
            map_name: "de_nuke".to_string(),
            match_result: result.to_string(),
            scores,
            skill_level: None,
            teammates_flashed: None,
            flashbangs_thrown: None,
            rounds_count: None,
        }
    }

    fn hydrated_match(
        id: &str,
        first_team: &[&str],
        first_score: u32,
        second_team: &[&str],
        second_score: u32,
        minutes_ago: i64,
    ) -> services::leetify::LeetifyMatch {
        services::leetify::LeetifyMatch {
            id: id.to_string(),
            game_finished_at: Utc.with_ymd_and_hms(2026, 1, 1, 12, 0, 0).unwrap()
                - Duration::minutes(minutes_ago),
            map_name: "de_nuke".to_string(),
            teams: vec![
                services::leetify::LeetifyMatchTeam {
                    steam64_ids: first_team.iter().map(|id| steam_id(id)).collect(),
                    score: first_score,
                },
                services::leetify::LeetifyMatchTeam {
                    steam64_ids: second_team.iter().map(|id| steam_id(id)).collect(),
                    score: second_score,
                },
            ],
        }
    }

    fn histories(
        entries: Vec<(&str, Vec<services::leetify::LeetifyGame>)>,
    ) -> HashMap<SteamID, Vec<services::leetify::LeetifyGame>> {
        entries
            .into_iter()
            .map(|(id, games)| (steam_id(id), games))
            .collect()
    }

    fn configured(ids: &[&str]) -> HashSet<SteamID> {
        ids.iter().map(|id| steam_id(id)).collect()
    }

    fn prediction_stats(
        queue_steam_ids: &[SteamID],
        histories: &HashMap<SteamID, Vec<services::leetify::LeetifyGame>>,
    ) -> Option<PredictionStats> {
        let observations = build_team_observations(queue_steam_ids, histories);
        aggregate_prediction(&observations)
    }

    fn prediction_stats_with_limit(
        queue_steam_ids: &[SteamID],
        histories: &HashMap<SteamID, Vec<services::leetify::LeetifyGame>>,
        match_count: usize,
    ) -> Option<PredictionStats> {
        let configured_steam_ids = queue_steam_ids.iter().cloned().collect();
        let observations = build_team_observations_with_limit(
            queue_steam_ids.len(),
            queue_steam_ids,
            &configured_steam_ids,
            histories,
            match_count,
        );
        aggregate_prediction(&observations)
    }

    fn assert_win_loss(stats: Option<PredictionStats>, wins: usize, losses: usize) {
        let stats = stats.expect("prediction should have decisive observations");
        assert_eq!((stats.wins, stats.losses), (wins, losses));
    }

    fn assert_observed_results(
        stats: Option<PredictionStats>,
        wins: usize,
        losses: usize,
        ties: usize,
    ) {
        let stats = stats.expect("prediction should have decisive observations");
        assert_eq!(
            (
                stats.observed_wins,
                stats.observed_losses,
                stats.observed_ties
            ),
            (wins, losses, ties)
        );
    }

    #[test]
    fn queue_start_is_now_for_instant_and_next_scheduled_occurrence_for_timed() {
        let tz = chrono_tz::Europe::Helsinki;
        let now = tz.with_ymd_and_hms(2026, 9, 12, 18, 0, 0).unwrap();
        let instant_id = QueueId::new(String::new());
        let instant_queue = Queue::new(
            NaiveTime::from_hms_opt(18, 30, 0).unwrap(),
            "/add".to_string(),
        );
        assert_eq!(queue_start_at(&instant_id, &instant_queue, now), now);

        let timed_id = QueueId::new("19:30".to_string());
        let timed_queue = Queue::new(
            NaiveTime::from_hms_opt(19, 30, 0).unwrap(),
            "/1930".to_string(),
        );
        assert_eq!(
            queue_start_at(&timed_id, &timed_queue, now),
            tz.with_ymd_and_hms(2026, 9, 12, 19, 30, 0).unwrap()
        );

        let after_schedule = tz.with_ymd_and_hms(2026, 9, 12, 20, 0, 0).unwrap();
        assert_eq!(
            queue_start_at(&timed_id, &timed_queue, after_schedule),
            tz.with_ymd_and_hms(2026, 9, 13, 19, 30, 0).unwrap()
        );
    }

    #[test]
    fn prediction_solo_match_has_composition_weight_three_in_three_player_queue() {
        let queue = vec![steam_id("A"), steam_id("B"), steam_id("C")];
        let histories = histories(vec![(
            "A",
            vec![game(
                "solo",
                &["A", "outside-1", "outside-2", "outside-3", "outside-4"],
                "win",
                1,
            )],
        )]);

        assert_win_loss(prediction_stats(&queue, &histories), 3, 0);
    }

    #[test]
    fn prediction_one_player_queue_distinguishes_historical_context() {
        let queue = vec![steam_id("A")];
        let configured_steam_ids = configured(&["A", "B", "C", "D", "E"]);
        let histories = histories(vec![(
            "A",
            vec![
                game(
                    "solo-context",
                    &["A", "outside-1", "outside-2", "outside-3", "outside-4"],
                    "win",
                    1,
                ),
                game(
                    "duo-context",
                    &["A", "B", "outside-1", "outside-2", "outside-3"],
                    "win",
                    2,
                ),
                game("stack-context", &["A", "B", "C", "D", "E"], "win", 3),
            ],
        )]);

        let observations = build_team_observations_with_configured(
            queue.len(),
            &queue,
            &configured_steam_ids,
            &histories,
        );
        let weights: HashMap<_, _> = observations
            .iter()
            .map(|observation| (observation.match_id.as_str(), observation.weight))
            .collect();

        assert_eq!(weights.get("solo-context"), Some(&5));
        assert_eq!(weights.get("duo-context"), Some(&4));
        assert_eq!(weights.get("stack-context"), Some(&1));

        let duo = observations
            .iter()
            .find(|observation| observation.match_id == "duo-context")
            .unwrap();
        assert_eq!(duo.current_overlap, 1);
        assert_eq!(duo.historical_configured_count, 2);
        assert_eq!(duo.configured_outside_count, 1);
        assert_eq!(duo.unconfigured_count, 3);
    }

    #[test]
    fn observation_weight_combines_overlap_and_context_score() {
        assert_eq!(observation_weight(1, 1, 1), 5);
        assert_eq!(observation_weight(1, 1, 2), 4);
        assert_eq!(observation_weight(1, 1, 5), 1);
        assert_eq!(observation_weight(2, 2, 2), 10);
        assert_eq!(observation_weight(2, 1, 1), 4);
        assert_eq!(observation_weight(3, 3, 3), 15);
        assert_eq!(observation_weight(3, 2, 3), 10);
        assert_eq!(observation_weight(3, 3, 5), 9);
        assert_eq!(observation_weight(3, 2, 2), 8);
        assert_eq!(observation_weight(5, 5, 5), 25);
        assert_eq!(observation_weight(5, 4, 5), 20);
        assert_eq!(observation_weight(5, 3, 3), 9);
    }

    #[test]
    fn prediction_uses_hydrated_roster_instead_of_player_scoped_history_stats() {
        let queue = vec![steam_id("A"), steam_id("B")];
        let histories = histories(vec![
            ("A", vec![game("shared", &["A"], "win", 1)]),
            ("B", vec![game("shared", &["B"], "win", 1)]),
        ]);
        let candidate_match_ids = select_candidate_match_ids(&queue, &histories, 30);
        let hydrated_matches = HashMap::from([(
            "shared".to_string(),
            hydrated_match(
                "shared",
                &["A", "B", "u1", "u2", "u3"],
                13,
                &["x1", "x2", "x3", "x4", "x5"],
                9,
                1,
            ),
        )]);

        let observations = build_team_observations_from_matches(
            queue.len(),
            &queue,
            &configured(&["A", "B"]),
            &candidate_match_ids,
            &hydrated_matches,
        );

        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].current_overlap, 2);
        assert_eq!(observations[0].historical_configured_count, 2);
        assert_eq!(observations[0].unconfigured_count, 3);
        assert_win_loss(aggregate_prediction(&observations), 10, 0);
    }

    #[test]
    fn hydrated_match_adds_queued_opponent_without_their_own_history() {
        let queue = vec![steam_id("A"), steam_id("B"), steam_id("C")];
        let histories = histories(vec![("A", vec![game("opponents", &["A"], "win", 1)])]);
        let candidate_match_ids = select_candidate_match_ids(&queue, &histories, 30);
        let hydrated_matches = HashMap::from([(
            "opponents".to_string(),
            hydrated_match(
                "opponents",
                &["A", "B", "u1", "u2", "u3"],
                13,
                &["C", "x1", "x2", "x3", "x4"],
                9,
                1,
            ),
        )]);

        let observations = build_team_observations_from_matches(
            queue.len(),
            &queue,
            &configured(&["A", "B", "C"]),
            &candidate_match_ids,
            &hydrated_matches,
        );

        assert_eq!(observations.len(), 2);
        assert_win_loss(aggregate_prediction(&observations), 8, 3);
    }

    #[test]
    fn candidate_selection_is_scoped_to_the_current_queue() {
        let histories = histories(vec![
            ("A", vec![game("a-match", &["A"], "win", 1)]),
            ("B", vec![game("b-match", &["B"], "loss", 1)]),
        ]);

        let candidate_match_ids = select_candidate_match_ids(&[steam_id("A")], &histories, 30);

        assert_eq!(candidate_match_ids, HashSet::from(["a-match".to_string()]));
    }

    #[test]
    fn prediction_rejects_incomplete_hydrated_rosters() {
        let queue = vec![steam_id("A")];
        let candidate_match_ids = HashSet::from(["incomplete".to_string()]);
        let hydrated_matches = HashMap::from([(
            "incomplete".to_string(),
            hydrated_match(
                "incomplete",
                &["A"],
                13,
                &["x1", "x2", "x3", "x4", "x5"],
                9,
                1,
            ),
        )]);

        let observations = build_team_observations_from_matches(
            queue.len(),
            &queue,
            &configured(&["A"]),
            &candidate_match_ids,
            &hydrated_matches,
        );

        assert!(observations.is_empty());
    }

    #[test]
    fn prediction_three_player_queue_uses_overlap_and_context_weights() {
        let queue = vec![steam_id("A"), steam_id("B"), steam_id("C")];
        let histories = histories(vec![(
            "A",
            vec![
                game(
                    "three",
                    &["A", "B", "C", "outside-1", "outside-2"],
                    "win",
                    1,
                ),
                game(
                    "two",
                    &["A", "B", "outside-1", "outside-2", "outside-3"],
                    "win",
                    2,
                ),
                game(
                    "one",
                    &["A", "outside-1", "outside-2", "outside-3", "outside-4"],
                    "win",
                    3,
                ),
            ],
        )]);

        let observations = build_team_observations(&queue, &histories);
        let weights: HashMap<_, _> = observations
            .iter()
            .map(|observation| (observation.match_id.as_str(), observation.weight))
            .collect();
        assert_eq!(weights.get("three"), Some(&15));
        assert_eq!(weights.get("two"), Some(&8));
        assert_eq!(weights.get("one"), Some(&3));
        assert_win_loss(aggregate_prediction(&observations), 26, 0);
    }

    #[test]
    fn prediction_two_player_queue_uses_overlap_and_context_weights() {
        let queue = vec![steam_id("A"), steam_id("B")];
        let histories = histories(vec![(
            "A",
            vec![
                game(
                    "two",
                    &["A", "B", "outside-1", "outside-2", "outside-3"],
                    "win",
                    1,
                ),
                game(
                    "one",
                    &["A", "outside-1", "outside-2", "outside-3", "outside-4"],
                    "win",
                    2,
                ),
            ],
        )]);

        let observations = build_team_observations(&queue, &histories);
        let weights: HashMap<_, _> = observations
            .iter()
            .map(|observation| (observation.match_id.as_str(), observation.weight))
            .collect();
        assert_eq!(weights.get("two"), Some(&10));
        assert_eq!(weights.get("one"), Some(&4));
        assert_win_loss(aggregate_prediction(&observations), 14, 0);
    }

    #[test]
    fn prediction_one_player_queue_gives_solo_history_weight_five() {
        let queue = vec![steam_id("A")];
        let histories = histories(vec![("A", vec![game("solo", &["A"], "win", 1)])]);

        let observations = build_team_observations(&queue, &histories);
        assert_eq!(observations[0].weight, 5);
        assert_win_loss(aggregate_prediction(&observations), 5, 0);
    }

    #[test]
    fn prediction_full_queue_uses_context_and_overlap_weights() {
        let queue = vec![
            steam_id("A"),
            steam_id("B"),
            steam_id("C"),
            steam_id("D"),
            steam_id("E"),
        ];
        let histories = histories(vec![(
            "A",
            vec![
                game("five", &["A", "B", "C", "D", "E"], "win", 1),
                game("four", &["A", "B", "C", "D", "F"], "win", 2),
                game(
                    "three",
                    &["A", "B", "C", "outside-1", "outside-2"],
                    "win",
                    3,
                ),
                game(
                    "two",
                    &["A", "B", "outside-1", "outside-2", "outside-3"],
                    "win",
                    4,
                ),
                game(
                    "one",
                    &["A", "outside-1", "outside-2", "outside-3", "outside-4"],
                    "win",
                    5,
                ),
            ],
        )]);

        let observations = build_team_observations_with_configured(
            queue.len(),
            &queue,
            &configured(&["A", "B", "C", "D", "E", "F"]),
            &histories,
        );
        let weights: HashMap<_, _> = observations
            .iter()
            .map(|observation| (observation.match_id.as_str(), observation.weight))
            .collect();
        assert_eq!(weights.get("five"), Some(&25));
        assert_eq!(weights.get("four"), Some(&20));
        assert_eq!(weights.get("three"), Some(&9));
        assert_eq!(weights.get("two"), Some(&4));
        assert_eq!(weights.get("one"), Some(&1));
        assert_win_loss(aggregate_prediction(&observations), 59, 0);
    }

    #[test]
    fn prediction_deduplicates_shared_same_side_match() {
        let queue = vec![steam_id("A"), steam_id("B"), steam_id("C")];
        let histories = histories(vec![
            (
                "A",
                vec![game("shared", &["A", "B", "C", "outside"], "win", 1)],
            ),
            (
                "B",
                vec![game("shared", &["A", "B", "C", "outside"], "win", 1)],
            ),
            (
                "C",
                vec![game("shared", &["A", "B", "C", "outside"], "win", 1)],
            ),
        ]);

        let observations = build_team_observations(&queue, &histories);
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].current_overlap, 3);
        assert_eq!(observations[0].weight, 15);
        assert_win_loss(aggregate_prediction(&observations), 15, 0);
    }

    #[test]
    fn prediction_two_player_overlap_has_composition_weight_four_in_three_player_queue() {
        let queue = vec![steam_id("A"), steam_id("B"), steam_id("C")];
        let histories = histories(vec![(
            "A",
            vec![game("shared", &["A", "B", "outside"], "win", 1)],
        )]);

        let observations = build_team_observations(&queue, &histories);
        assert_eq!(observations[0].current_overlap, 2);
        assert_eq!(observations[0].weight, 8);
        assert_win_loss(aggregate_prediction(&observations), 8, 0);
    }

    #[test]
    fn prediction_configured_outside_teammate_changes_context_without_overlap() {
        let queue = vec![steam_id("A"), steam_id("B"), steam_id("C")];
        let configured_steam_ids = configured(&["A", "B", "C", "D"]);
        let histories = histories(vec![(
            "A",
            vec![game(
                "configured-outside",
                &["A", "B", "D", "outside-1", "outside-2"],
                "win",
                1,
            )],
        )]);

        let observations = build_team_observations_with_configured(
            queue.len(),
            &queue,
            &configured_steam_ids,
            &histories,
        );
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].current_overlap, 2);
        assert_eq!(observations[0].historical_configured_count, 3);
        assert_eq!(observations[0].configured_outside_count, 1);
        assert_eq!(observations[0].unconfigured_count, 2);
        assert_eq!(observations[0].weight, 10);
    }

    #[test]
    fn prediction_counts_queued_opponents_as_separate_sides() {
        let queue = vec![steam_id("A"), steam_id("B"), steam_id("C")];
        let histories = histories(vec![
            (
                "A",
                vec![game("opponents", &["A", "B", "outside"], "win", 1)],
            ),
            (
                "B",
                vec![game("opponents", &["A", "B", "outside"], "win", 1)],
            ),
            ("C", vec![game("opponents", &["C", "enemy"], "loss", 1)]),
        ]);

        let stats = prediction_stats(&queue, &histories);
        assert_win_loss(stats, 8, 3);
        assert_observed_results(prediction_stats(&queue, &histories), 1, 1, 0);
    }

    #[test]
    fn prediction_equal_opposing_overlap_contributes_one_each() {
        let queue = vec![steam_id("A"), steam_id("B")];
        let histories = histories(vec![
            ("A", vec![game("opponents", &["A", "outside-a"], "win", 1)]),
            ("B", vec![game("opponents", &["B", "outside-b"], "loss", 1)]),
        ]);

        let stats = prediction_stats(&queue, &histories);
        assert_win_loss(stats, 4, 4);
        assert_observed_results(prediction_stats(&queue, &histories), 1, 1, 0);
    }

    #[test]
    fn prediction_opponent_does_not_increase_winning_side_overlap() {
        let queue = vec![steam_id("A"), steam_id("B"), steam_id("C")];
        let histories = histories(vec![
            (
                "A",
                vec![game("opponents", &["A", "B", "outside"], "win", 1)],
            ),
            ("C", vec![game("opponents", &["C", "enemy"], "loss", 1)]),
        ]);

        let observations = build_team_observations(&queue, &histories);
        let winning_side = observations
            .iter()
            .find(|observation| observation.result == PredictionResult::Win)
            .expect("winning side should be present");
        assert_eq!(winning_side.current_overlap, 2);
        assert_eq!(winning_side.historical_configured_count, 2);
        assert_eq!(winning_side.configured_outside_count, 0);
        assert_eq!(winning_side.unconfigured_count, 3);
        assert_eq!(winning_side.weight, 8);
        assert_win_loss(aggregate_prediction(&observations), 8, 3);
    }

    #[test]
    fn prediction_deduplicates_copies_of_one_team_observation() {
        let queue = vec![steam_id("A"), steam_id("B")];
        let histories = histories(vec![
            ("A", vec![game("duplicate", &["A", "B"], "win", 1)]),
            ("B", vec![game("duplicate", &["B", "A"], "win", 1)]),
        ]);

        let observations = build_team_observations(&queue, &histories);
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].weight, 10);
        assert_win_loss(aggregate_prediction(&observations), 10, 0);
    }

    #[test]
    fn prediction_uses_roster_overlap_when_only_one_player_selected_the_match() {
        let queue = vec![steam_id("A"), steam_id("B")];
        let mut a_games: Vec<_> = (1..30)
            .map(|minutes_ago| {
                game(
                    &format!("a-filler-{minutes_ago}"),
                    &["A"],
                    "tie",
                    minutes_ago,
                )
            })
            .collect();
        a_games.push(game("shared-old", &["A", "B"], "win", 31));

        let mut b_games: Vec<_> = (1..=30)
            .map(|minutes_ago| {
                game(
                    &format!("b-filler-{minutes_ago}"),
                    &["B"],
                    "tie",
                    minutes_ago,
                )
            })
            .collect();
        b_games.push(game("shared-old", &["A", "B"], "win", 31));

        let a_selected = select_recent_games(&a_games);
        let b_selected = select_recent_games(&b_games);
        assert!(a_selected
            .iter()
            .any(|game| game.id.as_deref() == Some("shared-old")));
        assert!(!b_selected
            .iter()
            .any(|game| game.id.as_deref() == Some("shared-old")));

        let histories = histories(vec![("A", a_games), ("B", b_games)]);
        let observations = build_team_observations(&queue, &histories);
        let shared = observations
            .iter()
            .find(|observation| observation.match_id == "shared-old")
            .expect("A's selected history should introduce the shared match");
        assert_eq!(shared.current_overlap, 2);
        assert_eq!(shared.weight, 10);
        assert_win_loss(aggregate_prediction(&observations), 10, 0);
    }

    #[test]
    fn prediction_uses_the_requested_latest_match_count_per_player() {
        let queue = vec![steam_id("A")];
        let histories = histories(vec![(
            "A",
            vec![
                game("new", &["A"], "loss", 1),
                game("old", &["A"], "win", 2),
            ],
        )]);

        let stats = prediction_stats_with_limit(&queue, &histories, 1).unwrap();
        assert_eq!((stats.observed_wins, stats.observed_losses), (0, 1));
        assert_eq!((stats.wins, stats.losses), (0, 5));
    }

    #[test]
    fn prediction_line_shows_recent_results_and_predicted_rate() {
        let line = format_prediction_line(
            &QueueId::new("20:15".to_string()),
            5,
            1,
            1,
            Some(PredictionStats {
                wins: 70,
                losses: 75,
                observed_wins: 14,
                observed_losses: 15,
                observed_ties: 1,
                unique_observations: 29,
                unique_matches: 29,
            }),
        );

        assert_eq!(
            line,
            "- <b>20:15</b> · <b>1/5 players</b>\n  <b>Recent results:</b> 14W / 15L / 1T (48%)\n  <b>Predicted win rate:</b> 48%"
        );
    }

    #[test]
    fn prediction_ignores_ties() {
        let queue = vec![steam_id("A")];
        let histories = histories(vec![("A", vec![game("tie", &["A"], "tie", 1)])]);

        assert!(prediction_stats(&queue, &histories).is_none());
    }

    #[test]
    fn prediction_uses_available_history_when_another_history_is_missing() {
        let queue = vec![steam_id("A"), steam_id("B")];
        let histories = histories(vec![("A", vec![game("available", &["A"], "win", 1)])]);

        assert_win_loss(prediction_stats(&queue, &histories), 4, 0);
    }

    #[test]
    fn prediction_counts_missing_history_player_in_roster_overlap() {
        let queue = vec![steam_id("A"), steam_id("B")];
        let histories = histories(vec![("A", vec![game("teammate", &["A", "B"], "win", 1)])]);

        let observations = build_team_observations(&queue, &histories);
        assert_eq!(observations[0].current_overlap, 2);
        assert_eq!(observations[0].weight, 10);
        assert_win_loss(aggregate_prediction(&observations), 10, 0);
    }

    #[test]
    fn prediction_is_unavailable_without_decisive_observations() {
        let queue = vec![steam_id("A")];
        let tie_history = histories(vec![("A", vec![game("tie", &["A"], "tie", 1)])]);
        assert!(prediction_stats(&queue, &tie_history).is_none());
        assert!(prediction_stats(&queue, &HashMap::new()).is_none());
    }

    #[test]
    fn prediction_keeps_opposite_sides_of_one_match_separate() {
        let a = steam_id("A");
        let b = steam_id("B");
        let queue = vec![a.clone(), b.clone()];
        let histories = histories(vec![
            ("A", vec![game("same-match", &["A"], "win", 1)]),
            ("B", vec![game("same-match", &["B"], "loss", 1)]),
        ]);

        let observations = build_team_observations(&queue, &histories);
        assert_eq!(observations.len(), 2);
        assert!(observations.iter().any(|observation| {
            observation.queued_side == vec![a.clone()]
                && observation.result == PredictionResult::Win
        }));
        assert!(observations.iter().any(|observation| {
            observation.queued_side == vec![b.clone()]
                && observation.result == PredictionResult::Loss
        }));
        assert_win_loss(aggregate_prediction(&observations), 4, 4);
    }

    #[test]
    fn prediction_rejects_conflicting_duplicate_observations() {
        let queue = vec![steam_id("A")];
        let mut conflicting = game("conflict", &["A"], "win", 1);
        conflicting.map_name = "de_mirage".to_string();
        let histories = histories(vec![(
            "A",
            vec![game("conflict", &["A"], "win", 1), conflicting],
        )]);

        let observations = build_team_observations(&queue, &histories);
        assert!(observations.is_empty());
        assert!(aggregate_prediction(&observations).is_none());
    }

    #[test]
    fn prediction_prefers_a_more_complete_compatible_duplicate_roster() {
        let queue = vec![steam_id("A"), steam_id("B")];
        let configured_steam_ids = configured(&["A", "B"]);
        let histories = histories(vec![(
            "A",
            vec![
                game("partial", &["A", "B"], "win", 1),
                game(
                    "partial",
                    &["A", "B", "outside-1", "outside-2", "outside-3"],
                    "win",
                    1,
                ),
            ],
        )]);

        let observations = build_team_observations_with_configured(
            queue.len(),
            &queue,
            &configured_steam_ids,
            &histories,
        );
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].historical_roster.len(), 5);
        assert_eq!(observations[0].historical_configured_count, 2);
        assert_eq!(observations[0].unconfigured_count, 3);
        assert_eq!(observations[0].weight, 10);
    }

    #[test]
    fn prediction_rejects_conflicting_duplicate_rosters() {
        let queue = vec![steam_id("A")];
        let configured_steam_ids = configured(&["A", "B", "C"]);
        let histories = histories(vec![(
            "A",
            vec![
                game("roster-conflict", &["A", "B", "outside-1"], "win", 1),
                game("roster-conflict", &["A", "C", "outside-1"], "win", 1),
            ],
        )]);

        let observations = build_team_observations_with_configured(
            queue.len(),
            &queue,
            &configured_steam_ids,
            &histories,
        );
        assert!(observations.is_empty());
    }
}
