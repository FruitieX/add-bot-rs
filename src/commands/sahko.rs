use teloxide::types::InputFile;

use crate::services::porssisahko::get_price_chart;

pub async fn get_sahko_inputfile() -> InputFile {
    // TODO: Error handling?
    let price_chart_bytes = get_price_chart().await.unwrap();

    InputFile::memory(price_chart_bytes)
}
