use color_eyre::Result;
use teloxide::types::InputFile;

use crate::{services::activity::get_activity_chart, settings::Settings, types::Username};

pub async fn get_activity_inputfile(
    settings: &Settings,
    for_user: Option<&Username>,
) -> Result<InputFile> {
    let chart_bytes = get_activity_chart(settings, for_user).await?;
    let inputfile = InputFile::memory(chart_bytes);
    Ok(inputfile)
}
