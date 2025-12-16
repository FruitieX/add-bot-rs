use crate::{
    command::Command,
    commands::{
        activity::get_activity_inputfile,
        queue::{add_remove, list, remove_all},
        sahko::get_sahko_inputfile,
        stats::{hall_of_fame, hall_of_shame, last_played, stat_leaderboard, stats},
        weather::{temperature, weather as weather_report},
    },
    settings::Settings,
    state_container::StateContainer,
    util::{mk_username, send_msg, send_photo},
};

use chrono_tz::Tz;
use teloxide::{prelude::*, Bot};

/// Handler for parsed incoming Telegram commands.
pub async fn handle_cmd(
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

    let markdown = matches!(cmd, Command::Help);

    let text = match cmd {
        Command::Help => Command::help(),
        Command::AddRemove { time, for_user } => {
            let username = for_user.unwrap_or_else(|| mk_username(&user));
            add_remove(username, state, chat_id, &tz, time, &sc).await
        }
        Command::RemoveAll => {
            let username = mk_username(&user);
            remove_all(username, state, chat_id, &sc).await
        }
        Command::List => list(state, chat_id, &tz),
        Command::Stats { for_user } => {
            let username = for_user.unwrap_or_else(|| mk_username(&user));
            stats(&settings, &username).await
        }
        Command::LastPlayed { for_user } => {
            let username = for_user.unwrap_or_else(|| mk_username(&user));
            last_played(&settings, &tz, username).await
        }
        Command::HallOfShame => hall_of_shame(&settings, &tz).await,
        Command::HallOfFame { rank_type } => hall_of_fame(&settings, rank_type).await,
        Command::Temperature => temperature().await,
        Command::Weather => weather_report().await,
        Command::Sahko => {
            let photo = match get_sahko_inputfile().await {
                Ok(photo) => photo,
                Err(e) => {
                    eprintln!("Failed to fetch price chart: {}", e);
                    return None;
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
                    return None;
                }
            };

            send_photo(&bot, &chat_id, photo).await;
            return Some(());
        }
        Command::StatLeaderboard { stat_type } => stat_leaderboard(&settings, stat_type).await,
    };

    send_msg(&bot, &chat_id, &text, markdown).await;

    Some(())
}
