pub mod backtest;
pub mod prompt_builder;

use std::env;

use anyhow::{Context as _, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Copy)]
pub enum Model {
    O1Preview,
    O1Mini,
}

impl Model {
    pub fn as_str(&self) -> &str {
        match self {
            Model::O1Preview => "o1-preview",
            Model::O1Mini => "o1-mini",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Long,
    Short,
    None,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy)]
pub struct CoinbaseCandle(
    f64, // time
    f64, // low
    f64, // high
    f64, // open
    f64, // close
    f64, // volume
);

pub fn candles_to_array(candles: Vec<CoinbaseCandle>) -> Vec<[f64; 6]> {
    // Coinbase returns candles most recent first, so reverse to chronological
    let mut candles = candles;
    candles.reverse();

    candles
        .into_iter()
        .map(|c| {
            let CoinbaseCandle(time, low, high, open, close, volume) = c;
            [time, open, high, low, close, volume]
        })
        .collect()
}

async fn get_candle_data(
    symbol: &str,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<Vec<CoinbaseCandle>> {
    let client = reqwest::Client::new();
    // let end = Utc::now();
    // let start = end - chrono::Duration::hours(24);

    let url = format!(
        "https://api.exchange.coinbase.com/products/{symbol}-USD/candles\
        ?start={}\
        &end={}\
        &granularity=3600",
        start.timestamp(),
        end.timestamp()
    );

    let response = client
        .get(&url)
        .header("User-Agent", "Mozilla/5.0")
        .send()
        .await?;

    let data: Vec<CoinbaseCandle> = response.json().await?;
    Ok(data)
}

pub async fn analyze_data_gpt(prompt: &str, model: Model) -> Result<String> {
    let api_key =
        env::var("OPENAI_API_KEY").context("OPENAI_API_KEY environment variable is not set")?;

    let body = json!({
        "model": model.as_str(),
        "messages": [
            {
                "role": "user",
                "content": prompt
            }
        ]
    });

    let client = reqwest::Client::new();
    tracing::debug!(
        ?model,
        prompt_len = prompt.len(),
        "Sending request to OpenAI API"
    );

    let resp = client
        .post("https://api.openai.com/v1/chat/completions")
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {}", api_key))
        .timeout(std::time::Duration::from_secs(300))
        .json(&body)
        .send()
        .await
        .context("Failed to send request to OpenAI API")?;

    // Check if the response is successful
    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        tracing::error!("OpenAI API returned error: {} - {}", status, text);
        anyhow::bail!("OpenAI API error: {} - {}", status, text);
    }

    let val: Value = resp
        .json()
        .await
        .context("Failed to parse OpenAI API response as JSON")?;

    tracing::debug!("Full OpenAI API response: {}", val);

    // Extract the "content" field from the first choice
    let content = val["choices"]
        .get(0)
        .and_then(|choice| choice["message"]["content"].as_str())
        .map(str::to_string)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Could not find 'content' field in the API response: {}",
                val
            )
        })?;

    Ok(content)
}

// Profit threshold multipliers
pub const LONG_THRESHOLD: f64 = 1.003; // +0.3%
pub const SHORT_THRESHOLD: f64 = 0.997; // -0.3%

pub fn label_candles(data: &[[f64; 6]]) -> Vec<Action> {
    use Action::*;
    // For convenience, define indexes into the candle array
    const HIGH: usize = 2;
    const LOW: usize = 3;
    const CLOSE: usize = 4;

    let mut labels = data
        .windows(2)
        .map(|w| {
            let current = &w[0];
            let next = &w[1];

            let c_close = current[CLOSE];
            let next_high = next[HIGH];
            let next_low = next[LOW];

            let long_cond = next_high >= c_close * LONG_THRESHOLD;
            let short_cond = next_low <= c_close * SHORT_THRESHOLD;

            match (long_cond, short_cond) {
                (true, true) => Short, // tie-break: choose "short"
                (true, false) => Long,
                (false, true) => Short,
                (false, false) => None,
            }
        })
        .collect::<Vec<_>>();

    // The last candle has no future candle, so "none"
    labels.push(None);

    labels
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Action;

    #[test]
    fn test_label_candles_basic_short_scenario() {
        // Original scenario from the provided test
        let data = [
            [0.0, 0.0, 3603.0, 3599.99, 3594.88, 100.0],
            [0.0, 0.0, 3600.0, 3565.45, 3599.99, 100.0],
            [0.0, 0.0, 3570.86, 3558.89, 3565.52, 100.0],
        ];

        let labels = label_candles(&data);
        assert_eq!(labels.len(), 3);
        // Check logic:
        // 1st candle: short
        // 2nd candle: short
        // 3rd candle: none (no future candle)
        assert_eq!(labels, vec![Action::Short, Action::Short, Action::None]);
    }

    #[test]
    fn test_label_candles_single_candle() {
        // Only one candle => no future candle, label should be [none]
        let data = [
            [0.0, 0.0, 100.0, 99.0, 100.0, 500.0], // arbitrary values
        ];

        let labels = label_candles(&data);
        assert_eq!(labels, vec![Action::None]);
    }

    #[test]
    fn test_label_candles_no_conditions_met() {
        // No conditions for long or short met
        // close=100, next high=100.2 (<100*1.003=100.3?), no
        // next low=99.8 (>100*0.997=99.7?), no (low must be <=99.7 to trigger short)
        let data = [
            [0.0, 0.0, 100.2, 99.8, 100.0, 500.0],
            [0.0, 0.0, 100.2, 99.8, 100.1, 500.0],
            [0.0, 0.0, 100.2, 99.8, 100.2, 500.0],
        ];

        let labels = label_candles(&data);
        assert_eq!(labels.len(), 3);
        // Each window:
        // 1st->2nd: close=100.0, needs high>=100.3 or low<=99.7; got high=100.2, low=99.8 => none
        // 2nd->3rd: close=100.1, needs high>=100.401 or low<=99.799; got high=100.2, low=99.8 => none
        // last candle => none
        assert_eq!(labels, vec![Action::None, Action::None, Action::None]);
    }

    #[test]
    fn test_label_candles_long_condition() {
        // Trigger a long condition:
        // close=100.0, next candle high=101.0 (1% increase >0.3%), no short condition triggered
        let data = [
            [0.0, 0.0, 101.0, 99.5, 100.0, 1000.0],
            [0.0, 0.0, 101.0, 100.0, 100.5, 1000.0],
        ];
        // For the first candle's label:
        // c_close=100.0, LONG_THRESHOLD=100.3, next_high=101.0 >100.3 => long condition met
        // short condition would require next_low<=99.7. If low=100.0 (from 2nd candle), no short triggered.
        let labels = label_candles(&data);
        assert_eq!(labels.len(), 2);
        assert_eq!(labels[0], Action::Long);
        assert_eq!(labels[1], Action::None);
    }

    #[test]
    fn test_label_candles_short_condition() {
        // Trigger a short condition:
        // c_close=100.0, need next_low<=99.7
        let data = [
            [0.0, 0.0, 100.5, 99.8, 100.0, 1000.0],
            [0.0, 0.0, 100.2, 99.6, 100.0, 1000.0], // low=99.6 triggers short
        ];

        let labels = label_candles(&data);
        assert_eq!(labels, vec![Action::Short, Action::None]);
    }

    #[test]
    fn test_label_candles_both_conditions() {
        // Both long and short conditions are met simultaneously:
        // c_close=100.0
        // long requires next_high>=100.3
        // short requires next_low<=99.7
        // If next candle: high=100.5, low=99.5
        // Both triggered => tie-break chooses Short
        let data = [
            [0.0, 0.0, 100.5, 99.5, 100.0, 1000.0],
            [0.0, 0.0, 101.0, 99.5, 100.3, 1000.0],
        ];

        let labels = label_candles(&data);
        assert_eq!(labels.len(), 2);
        // 1st->2nd candle: both long and short triggered => short chosen
        assert_eq!(labels[0], Action::Short);
        assert_eq!(labels[1], Action::None);
    }

    #[test]
    fn test_label_candles_multiple_varied() {
        let data = [
            [0.0, 0.0, 100.2, 99.8, 100.0, 500.0], // Candle0
            [0.0, 0.0, 100.2, 99.8, 100.0, 500.0], // Candle1
            [0.0, 0.0, 102.0, 99.9, 101.0, 500.0], // Candle2
            [0.0, 0.0, 101.1, 99.0, 100.5, 500.0], // Candle3
            [0.0, 0.0, 102.0, 99.0, 100.0, 500.0], // Candle4
        ];

        let labels = label_candles(&data);
        assert_eq!(labels.len(), 5);
        assert_eq!(
            labels,
            vec![
                Action::None,
                Action::Long,
                Action::Short,
                Action::Short,
                Action::None
            ]
        );
    }
}
