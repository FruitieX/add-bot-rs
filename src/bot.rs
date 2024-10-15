use crate::{
    command::Command,
    commands::{
        queue::{add_remove, list, remove_all},
        sahko::get_sahko_inputfile,
        stats::{hall_of_fame, hall_of_shame, last_played, stats},
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
        Command::Sahko => String::new(),
    };

    // TODO: Do something smarter
    match text.as_str() {
        "" => send_photo(&bot, &chat_id, get_sahko_inputfile().await).await,
        _ => send_msg(&bot, &chat_id, &text, markdown).await,
    };

    Some(())
}
