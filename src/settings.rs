use std::collections::HashMap;

use serde::Deserialize;

use crate::types::{SteamID, Username};

#[derive(Clone, Deserialize, Debug)]
pub struct TeloxideSettings {
    pub bot_api_token: String,
}

#[derive(Clone, Deserialize, Debug)]

pub struct PlayersSettings {
    pub steamid_mappings: HashMap<Username, SteamID>,
}

#[derive(Clone, Deserialize, Debug)]

pub struct WeatherSettings {
    pub latitude: f64,

    pub longitude: f64,

    pub display_name: String,
}

#[derive(Clone, Deserialize, Debug)]

pub struct Settings {
    pub teloxide: TeloxideSettings,

    pub players: PlayersSettings,

    pub weather: Option<WeatherSettings>,
}

pub fn read_settings() -> Result<Settings, config::ConfigError> {
    config::Config::builder()
        .add_source(config::File::with_name("Settings"))
        .build()?
        .try_deserialize::<Settings>()
}
