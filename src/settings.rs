use std::collections::HashMap;

use serde::Deserialize;

#[derive(Clone, Deserialize, Debug)]
pub struct TeloxideSettings {
    pub bot_api_token: String,
}

#[derive(Clone, Deserialize, Debug)]
pub struct PlayersSettings {
    pub steamid_mappings: HashMap<String, String>,
}

#[derive(Clone, Deserialize, Debug)]
pub struct Settings {
    pub teloxide: TeloxideSettings,
    pub players: PlayersSettings,
}

pub fn read_settings() -> Result<Settings, config::ConfigError> {
    config::Config::builder()
        .add_source(config::File::with_name("Settings"))
        .build()?
        .try_deserialize::<Settings>()
}
