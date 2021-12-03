use crate::{
    command::Command,
    state::StateContainer,
    util::{fmt_naive_time, mk_username, send_msg},
};
use std::time::Duration;
use teloxide::Bot;
use teloxide::{prelude::*, utils::command::BotCommand};

/// Task that polls and takes action for any queues that have timed out.
pub async fn poll_for_timeouts(state: StateContainer, bot: Bot) {
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await
    }
}

/// Handler for parsed incoming Telegram commands.
pub async fn handle_cmd(state: StateContainer, bot: Bot, msg: Message, cmd: Command) -> Option<()> {
    let chat_id = msg.chat.id;
    let user = msg.from()?;
    let user_id = user.id;

    match cmd {
        Command::Help => send_msg(&bot, chat_id, &Command::descriptions()).await,
        Command::Add(queue_id) => {
            let username = mk_username(user);
            let text = format!(
                "Hello {} ({}) in {}! queue_id = {:?}",
                username,
                user_id,
                chat_id,
                queue_id.as_ref().map(fmt_naive_time)
            );
            send_msg(&bot, chat_id, &text).await
        }
    }

    Some(())
}
