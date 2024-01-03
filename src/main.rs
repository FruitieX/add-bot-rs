use crate::state_container::StateContainer;
use anyhow::Result;
use chrono_tz::Tz;
use clap::Parser;
use teloxide::{types::Message, utils::client_from_env, Bot};

mod bot;
mod command;
mod commands;
mod services;
mod settings;
mod state;
mod state_container;
mod types;
mod util;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(short, long, default_value = "UTC")]
    tz: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let settings = settings::read_settings()?;

    // Try restoring state from file, or default to empty state.
    let sc = StateContainer::try_read_from_file().await?;

    let args = Args::parse();
    let tz: Tz = args.tz.parse().unwrap();

    // Initialize the Telegram bot API.
    pretty_env_logger::init();
    let bot = Bot::with_client(&settings.teloxide.bot_api_token, client_from_env());

    // Spawn a new task that polls for queues that have timed out.
    tokio::spawn(commands::queue::poll_for_timeouts(
        sc.clone(),
        tz,
        bot.clone(),
    ));

    // Start polling for Telegram messages.
    teloxide::repl(bot.clone(), move |message: Message, bot: Bot| {
        let settings = settings.clone();
        let sc = sc.clone();

        async move {
            let msg_text = message.text();

            // Only attempt parsing message if there's any message text.
            if let Some(msg_text) = msg_text {
                let cmd = command::parse_cmd(msg_text);

                if let Ok(Some(cmd)) = cmd {
                    bot::handle_cmd(settings, sc, tz, bot, message, cmd).await;
                }
            }

            Ok(())
        }
    })
    .await;

    Ok(())
}
