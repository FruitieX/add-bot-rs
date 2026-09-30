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
    util::{mk_username, send_msg, send_photo, ReplyContext},
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
                log::warn!(
                    "Failed to send typing status for {chat_id}: {}",
                    crate::util::telegram_error_summary(&error)
                );
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
    let destination = ReplyContext::from_message(&msg);
    let chat_id = msg.chat.id;
    let user = msg.from?;

    let text = match cmd {
        Command::Help => Command::help(),
        Command::HelpAll => crate::command::HELP_TEXT.to_string(),
        Command::Version => crate::command::version(),
        Command::AddRemove { time, for_user } => {
            let username = for_user.unwrap_or_else(|| mk_username(&user));
            match add_remove(username, destination, tz, time, &sc).await {
                Ok(change) => {
                    if let Err(error) =
                        crate::commands::queue::send_queue_change(&settings, &sc, &bot, change, tz)
                            .await
                    {
                        log::error!("Queue delivery/update failed: {error}");
                    }
                    return Some(());
                }
                Err(error) => {
                    log::error!("Queue change failed: {error}");
                    crate::state_container::save_failure_message(&error).to_owned()
                }
            }
        }
        Command::RemoveAll => {
            let username = mk_username(&user);
            match remove_all(username, destination, &sc, tz).await {
                Ok(Some(change)) => {
                    if let Err(error) =
                        crate::commands::queue::send_queue_change(&settings, &sc, &bot, change, tz)
                            .await
                    {
                        log::error!("Queue delivery/update failed: {error}");
                    }
                    return Some(());
                }
                Ok(None) => "You’re not in any queues.".into(),
                Err(error) => {
                    log::error!("Queue change failed: {error}");
                    crate::state_container::save_failure_message(&error).to_owned()
                }
            }
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
            let form_options = match form_options {
                Ok(options) => options,
                Err(error) => {
                    log::error!("Preference save failed: {error}");
                    if let Err(error) = send_msg(
                        &bot,
                        destination,
                        crate::state_container::save_failure_message(&error),
                    )
                    .await
                    {
                        log::warn!(
                            "Preference error reply failed: {}",
                            crate::util::telegram_error_summary(&error)
                        );
                    }
                    return Some(());
                }
            };
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
            let form_options = match form_options {
                Ok(options) => options,
                Err(error) => {
                    log::error!("Preference save failed: {error}");
                    if let Err(error) = send_msg(
                        &bot,
                        destination,
                        crate::state_container::save_failure_message(&error),
                    )
                    .await
                    {
                        log::warn!(
                            "Preference error reply failed: {}",
                            crate::util::telegram_error_summary(&error)
                        );
                    }
                    return Some(());
                }
            };
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
                    if let Err(error) = send_msg(
                        &bot,
                        destination,
                        &crate::services::failure::message(&e, "Electricity chart", "/el"),
                    )
                    .await
                    {
                        log::warn!(
                            "Chart error reply failed: {}",
                            crate::util::telegram_error_summary(&error)
                        );
                    }
                    return Some(());
                }
            };

            if let Err(error) = send_photo(&bot, destination, photo).await {
                log::warn!(
                    "Chart delivery failed: {}",
                    crate::util::telegram_error_summary(&error)
                );
            }
            return Some(());
        }
        Command::Activity { for_user, style } => {
            let photo = match get_activity_inputfile(&settings, for_user.as_ref(), tz, style).await
            {
                Ok(photo) => photo,
                Err(e) => {
                    eprintln!("Failed to fetch activity chart: {}", e);
                    if let Err(error) = send_msg(
                        &bot,
                        destination,
                        &crate::services::failure::message(&e, "Activity chart", "/activity"),
                    )
                    .await
                    {
                        log::warn!(
                            "Chart error reply failed: {}",
                            crate::util::telegram_error_summary(&error)
                        );
                    }
                    return Some(());
                }
            };

            if let Err(error) = send_photo(&bot, destination, photo).await {
                log::warn!(
                    "Chart delivery failed: {}",
                    crate::util::telegram_error_summary(&error)
                );
            }
            return Some(());
        }
        Command::Results { for_user } => {
            let photo = match get_results_inputfile(&settings, for_user.as_ref(), tz).await {
                Ok(photo) => photo,
                Err(e) => {
                    eprintln!("Failed to fetch results chart: {}", e);
                    if let Err(error) = send_msg(
                        &bot,
                        destination,
                        &crate::services::failure::message(&e, "Results chart", "/results"),
                    )
                    .await
                    {
                        log::warn!(
                            "Chart error reply failed: {}",
                            crate::util::telegram_error_summary(&error)
                        );
                    }
                    return Some(());
                }
            };

            if let Err(error) = send_photo(&bot, destination, photo).await {
                log::warn!(
                    "Chart delivery failed: {}",
                    crate::util::telegram_error_summary(&error)
                );
            }
            return Some(());
        }
        Command::StatLeaderboard { stat_type } => stat_leaderboard(&settings, stat_type).await,
        Command::TeamFlash => team_flash_leaderboard(&settings).await,
    };

    if let Err(error) = send_msg(&bot, destination, &text).await {
        log::warn!(
            "Command reply failed: {}",
            crate::util::telegram_error_summary(&error)
        );
    }

    Some(())
}
