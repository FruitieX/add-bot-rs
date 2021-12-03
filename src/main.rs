use anyhow::Result;
use command::Command;
use state::StateContainer;
use teloxide::Bot;

mod bot;
mod command;
mod state;
mod util;

#[tokio::main]
async fn main() -> Result<()> {
    // Try restoring state from file, or default to empty state.
    let state = StateContainer::try_read_from_file().await?;

    // Initialize the Telegram bot API.
    teloxide::enable_logging!();
    let bot = Bot::from_env();

    // Spawn a new task that polls for queues that have timed out.
    tokio::spawn(bot::poll_for_timeouts(state.clone(), bot.clone()));

    // Start polling for Telegram messages.
    teloxide::commands_repl(bot.clone(), "CSGO add bot", move |cx, command: Command| {
        let bot = bot.clone();
        let state = state.clone();
        async move {
            bot::handle_cmd(state, bot, cx.update, command).await;

            // We need to help the compiler out with this type 😵
            Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
        }
    })
    .await;

    Ok(())
}
