use std::collections::HashMap;

use serde::Deserialize;

use crate::types::{SteamID, Username};

#[derive(Clone, Deserialize, Debug)]
#[serde(default)]
pub struct ElectricitySettings {
    /// Estimated electrical load of the gaming PC, excluding household base load.
    pub gaming_pc_power_watts: f64,
    /// Caruna Espoo Yleissiirto variable charge, including VAT, effective 2026-01-01.
    pub caruna_espoo_distribution_cents_per_kwh: f64,
}

impl ElectricitySettings {
    pub const DEFAULT_GAMING_PC_POWER_WATTS: f64 = 500.0;
    pub const DEFAULT_CARUNA_ESPOO_DISTRIBUTION_CENTS_PER_KWH: f64 = 2.77;
}

impl Default for ElectricitySettings {
    fn default() -> Self {
        Self {
            gaming_pc_power_watts: Self::DEFAULT_GAMING_PC_POWER_WATTS,
            caruna_espoo_distribution_cents_per_kwh:
                Self::DEFAULT_CARUNA_ESPOO_DISTRIBUTION_CENTS_PER_KWH,
        }
    }
}

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

#[derive(Clone, Deserialize, Debug, Default)]
#[serde(default)]
pub struct LeetifySettings {
    /// Optional API key from https://leetify.com/app/developer
    pub api_key: Option<String>,
}

#[derive(Clone, Deserialize, Debug)]

pub struct Settings {
    pub teloxide: TeloxideSettings,

    pub players: PlayersSettings,

    pub weather: Option<WeatherSettings>,
    #[serde(default)]
    pub leetify: Option<LeetifySettings>,

    #[serde(default)]
    pub electricity: ElectricitySettings,
}

pub fn read_settings() -> Result<Settings, config::ConfigError> {
    config::Config::builder()
        .add_source(config::File::with_name("Settings"))
        .build()?
        .try_deserialize::<Settings>()
}
