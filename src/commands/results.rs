use color_eyre::Result;
use teloxide::types::InputFile;

use crate::{services::results::get_results_chart, settings::Settings, types::Username};

pub async fn get_results_inputfile(
    settings: &Settings,
    for_user: Option<&Username>,
) -> Result<InputFile> {
    let chart_bytes = get_results_chart(settings, for_user).await?;
    Ok(InputFile::memory(chart_bytes))
}
