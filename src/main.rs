use crate::state_container::StateContainer;
use anyhow::Result;
use teloxide::Bot;

mod bot;
mod command;
mod state;
mod state_container;
mod types;
mod util;

#[tokio::main]
async fn main() -> Result<()> {
    // Try restoring state from file, or default to empty state.
    let sc = StateContainer::try_read_from_file().await?;

    // Initialize the Telegram bot API.
    teloxide::enable_logging!();
    let bot = Bot::from_env();

    // Spawn a new task that polls for queues that have timed out.
    tokio::spawn(bot::poll_for_timeouts(sc.clone(), bot.clone()));

    // Start polling for Telegram messages.
    teloxide::repl(bot.clone(), move |cx| {
        let bot = bot.clone();
        let sc = sc.clone();
        async move {
            let msg = cx.update;
            let msg_text = msg.text();

            // Only attempt parsing message if there's any message text.
            if let Some(msg_text) = msg_text {
                let cmd = command::parse_cmd(msg_text)?;

                if let Some(cmd) = cmd {
                    bot::handle_cmd(sc, bot, msg, cmd).await;
                }
            }

            // We need to help the compiler out with this type 😵
            Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
        }
    })
    .await;

    Ok(())
}
