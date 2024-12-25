use color_eyre::Result;
use teloxide::types::InputFile;

use crate::services::porssisahko::get_price_chart;

pub async fn get_sahko_inputfile() -> Result<InputFile> {
    let price_chart_bytes = get_price_chart().await?;
    let inputfile = InputFile::memory(price_chart_bytes);
    Ok(inputfile)
}
