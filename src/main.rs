use crate::state_container::StateContainer;
use chrono_tz::Tz;
use clap::Parser;
use color_eyre::Result;
use teloxide::{prelude::Requester, types::Message, utils::client_from_env, Bot};

mod bot;
mod command;
mod commands;
mod delivery;
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
    color_eyre::install()?;
    let settings = settings::read_settings()?;

    let args = Args::parse();
    let tz: Tz = args.tz.parse()?;
    let sc = StateContainer::try_read_from_file(tz).await?;

    // Initialize the Telegram bot API.
    pretty_env_logger::init();
    let bot = Bot::with_client(&settings.teloxide.bot_api_token, client_from_env());
    let bot_username = bot.get_me().await?.username.clone().unwrap_or_default();

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
        let bot_username = bot_username.clone();

        async move {
            let msg_text = message.text();

            // Only attempt parsing message if there's any message text.
            if let Some(msg_text) = msg_text {
                match command::parse_cmd_for_bot(msg_text, &bot_username) {
                    Ok(Some(cmd)) => {
                        bot::handle_cmd(settings, sc, tz, bot, message, cmd).await;
                    }
                    Ok(None) => {}
                    Err(error) => {
                        log::warn!("Command arguments rejected: {error}");
                        if let Err(error) = util::send_msg(
                            &bot,
                            util::ReplyContext::from_message(&message),
                            &command::argument_error_response(msg_text, error.as_ref()),
                        )
                        .await
                        {
                            log::warn!(
                                "Argument error reply failed: {}",
                                crate::util::telegram_error_summary(&error)
                            );
                        }
                    }
                }
            }

            Ok(())
        }
    })
    .await;

    Ok(())
}
