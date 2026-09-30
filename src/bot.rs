use crate::{
    command::Command,
    commands::{
        activity::get_activity_inputfile,
        queue::{add_remove, list, predictions, remove_all},
        results::get_results_inputfile,
        sahko::get_sahko_inputfile,
        stats::{
            hall_of_fame, hall_of_shame, last_played, recent_form, stat_leaderboard, stats,
            team_flash_leaderboard,
        },
        weather::{temperature, weather as weather_report},
    },
    settings::Settings,
    state_container::StateContainer,
    util::{mk_username, send_msg, send_photo},
};

use chrono_tz::Tz;
use std::time::Duration;
use teloxide::{payloads::SendChatActionSetters, prelude::*, types::ChatAction, Bot};

/// Handler for parsed incoming Telegram commands.
pub async fn handle_cmd(
    settings: Settings,
    sc: StateContainer,
    tz: Tz,
    bot: Bot,
    msg: Message,
    cmd: Command,
) -> Option<()> {
    let chat_id = msg.chat.id;
    let thread_id = msg.thread_id;
    let typing = async {
        let mut interval = tokio::time::interval(Duration::from_secs(4));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            // The first tick is immediate; subsequent ticks refresh Telegram's
            // five-second typing status while the command is still running.
            interval.tick().await;
            let mut request = bot.send_chat_action(chat_id, ChatAction::Typing);
            if let Some(thread_id) = thread_id {
                request = request.message_thread_id(thread_id);
            }
            if let Err(error) = request.await {
                log::warn!("Failed to send typing status for {chat_id}: {error}");
            }
        }
    };

    // Dropping the typing future on completion also covers early returns and
    // cancellation, without leaving a background task running.
    tokio::select! {
        biased;
        result = handle_cmd_inner(settings, sc, tz, bot.clone(), msg, cmd) => result,
        _ = typing => unreachable!("typing refresh loop does not terminate"),
    }
}

async fn handle_cmd_inner(
    settings: Settings,
    sc: StateContainer,
    tz: Tz,
    bot: Bot,
    msg: Message,
    cmd: Command,
) -> Option<()> {
    let state = sc.read().await;
    let chat_id = msg.chat.id;
    let user = msg.from?;

    let text = match cmd {
        Command::Help => Command::help(),
        Command::HelpAll => crate::command::HELP_TEXT.to_string(),
        Command::Version => crate::command::version(),
        Command::AddRemove { time, for_user } => {
            let username = for_user.unwrap_or_else(|| mk_username(&user));
            add_remove(&settings, username, state, chat_id, &tz, time, &sc).await
        }
        Command::RemoveAll => {
            let username = mk_username(&user);
            remove_all(&settings, username, state, chat_id, &sc, &tz).await
        }
        Command::List => list(state, chat_id, &tz),
        Command::Predictions { match_count } => {
            predictions(&settings, state, chat_id, &tz, match_count).await
        }
        Command::Stats {
            for_user,
            form_options,
        } => {
            let username = for_user.unwrap_or_else(|| mk_username(&user));
            let requester = user
                .username
                .as_ref()
                .map(|name| crate::types::Username::new(name.clone()));
            let form_options = sc
                .resolve_recent_form_options(requester.as_ref(), &username, form_options)
                .await;
            stats(&settings, &username, form_options).await
        }
        Command::RecentForm {
            for_user,
            form_options,
        } => {
            let username = for_user.unwrap_or_else(|| mk_username(&user));
            let requester = user
                .username
                .as_ref()
                .map(|name| crate::types::Username::new(name.clone()));
            let form_options = sc
                .resolve_recent_form_options(requester.as_ref(), &username, form_options)
                .await;
            recent_form(&settings, &username, form_options).await
        }
        Command::LastPlayed { for_user } => {
            let username = for_user.unwrap_or_else(|| mk_username(&user));
            last_played(&settings, &tz, username).await
        }
        Command::HallOfShame => hall_of_shame(&settings, &tz).await,
        Command::HallOfFame { rank_type } => hall_of_fame(&settings, rank_type).await,
        Command::Temperature => temperature(tz).await,
        Command::Weather => weather_report(tz).await,
        Command::Sahko => {
            let photo = match get_sahko_inputfile(&settings, &state, chat_id, &tz).await {
                Ok(photo) => photo,
                Err(e) => {
                    eprintln!("Failed to fetch price chart: {}", e);
                    send_msg(
                        &bot,
                        &chat_id,
                        &crate::services::failure::message(&e, "Electricity chart", "/el"),
                    )
                    .await;
                    return Some(());
                }
            };

            send_photo(&bot, &chat_id, photo).await;
            return Some(());
        }
        Command::Activity { for_user } => {
            let photo = match get_activity_inputfile(&settings, for_user.as_ref()).await {
                Ok(photo) => photo,
                Err(e) => {
                    eprintln!("Failed to fetch activity chart: {}", e);
                    send_msg(
                        &bot,
                        &chat_id,
                        &crate::services::failure::message(&e, "Activity chart", "/activity"),
                    )
                    .await;
                    return Some(());
                }
            };

            send_photo(&bot, &chat_id, photo).await;
            return Some(());
        }
        Command::Results { for_user } => {
            let photo = match get_results_inputfile(&settings, for_user.as_ref()).await {
                Ok(photo) => photo,
                Err(e) => {
                    eprintln!("Failed to fetch results chart: {}", e);
                    send_msg(
                        &bot,
                        &chat_id,
                        &crate::services::failure::message(&e, "Results chart", "/results"),
                    )
                    .await;
                    return Some(());
                }
            };

            send_photo(&bot, &chat_id, photo).await;
            return Some(());
        }
        Command::StatLeaderboard { stat_type } => stat_leaderboard(&settings, stat_type).await,
        Command::TeamFlash => team_flash_leaderboard(&settings).await,
    };

    send_msg(&bot, &chat_id, &text).await;

    Some(())
}
