pub mod backtest;
pub mod prompt_builder;

use std::{env, fs};

use anyhow::{Context as _, Result};
use chrono::{DateTime, Duration, Utc};
use prompt_builder::build_data_section;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

// Profit threshold multipliers
pub const LONG_THRESHOLD: f64 = 1.05;
pub const SHORT_THRESHOLD: f64 = 0.95;

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

const CANDLE_HOURS: usize = 24; // 24-hour window
const PROMPT_FILE: &str = "prompt.txt";

pub async fn run_live_analysis() -> Result<(Action, String)> {
    // We'll fetch data for the last N hours
    let end = Utc::now();
    let start = end - Duration::hours(CANDLE_HOURS as i64);

    // Fetch live data directly from the API (no caching)
    let eth_candles = candles_to_array(get_candle_data("ETH", start, end).await?);
    let btc_candles = candles_to_array(get_candle_data("BTC", start, end).await?);
    let sol_candles = candles_to_array(get_candle_data("SOL", start, end).await?);

    if eth_candles.len() < CANDLE_HOURS
        || btc_candles.len() < CANDLE_HOURS
        || sol_candles.len() < CANDLE_HOURS
    {
        anyhow::bail!("Not enough recent data to perform live analysis");
    }

    let base_prompt = fs::read_to_string(PROMPT_FILE).context("Failed to read base prompt file")?;

    let eth_window = &eth_candles[eth_candles.len() - CANDLE_HOURS..];
    let btc_window = &btc_candles[btc_candles.len() - CANDLE_HOURS..];
    let sol_window = &sol_candles[sol_candles.len() - CANDLE_HOURS..];

    let data_section = build_data_section(eth_window, btc_window, sol_window);
    let full_prompt = format!("{}\n\n{}", base_prompt, data_section);

    let response = analyze_data_gpt(&full_prompt, Model::O1Mini).await?;
    let clean_response = response.replace("```json", "").replace("```", "");

    let val: Value = serde_json::from_str(&clean_response)
        .with_context(|| format!("Response not valid JSON: {}", clean_response))?;
    let action_str = val
        .get("action")
        .and_then(|a| a.as_str())
        .context("Missing 'action' field in response")?;
    let rationale = val
        .get("rationale")
        .and_then(|r| r.as_str())
        .unwrap_or("")
        .to_string();

    let pred = match action_str {
        "long" => Action::Long,
        "short" => Action::Short,
        "none" => Action::None,
        _ => Action::None,
    };

    Ok((pred, rationale))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Action;

    #[test]
    fn test_label_candles_basic_short_scenario() {
        // Test with original values, calculated relative to base price
        let base = 3594.88; // Close price of first candle
        let data = [
            [0.0, 0.0, 3603.0, 3599.99, base, 100.0],
            [
                0.0,
                0.0,
                3600.0,
                base * SHORT_THRESHOLD - 1.0,
                3599.99,
                100.0,
            ],
            [0.0, 0.0, 3570.86, 3558.89, 3565.52, 100.0],
        ];

        let labels = label_candles(&data);
        assert_eq!(labels.len(), 3);
        assert_eq!(labels, vec![Action::Short, Action::Short, Action::None]);
    }

    #[test]
    fn test_label_candles_single_candle() {
        let data = [[0.0, 0.0, 100.0, 99.0, 100.0, 500.0]];
        let labels = label_candles(&data);
        assert_eq!(labels, vec![Action::None]);
    }

    #[test]
    fn test_label_candles_no_conditions_met() {
        let base = 100.0;
        // Set high just below LONG_THRESHOLD and low just above SHORT_THRESHOLD
        let data = [
            [0.0, 0.0, base * 1.002, base * 0.998, base, 500.0],
            [0.0, 0.0, base * 1.002, base * 0.998, base * 1.001, 500.0],
            [0.0, 0.0, base * 1.002, base * 0.998, base * 1.002, 500.0],
        ];

        let labels = label_candles(&data);
        assert_eq!(labels.len(), 3);
        assert_eq!(labels, vec![Action::None, Action::None, Action::None]);
    }

    #[test]
    fn test_label_candles_long_condition() {
        let base = 100.0;
        // High above LONG_THRESHOLD, low above SHORT_THRESHOLD (no short trigger)
        let data = [
            [0.0, 0.0, base * 1.01, base * 0.998, base, 1000.0],
            [0.0, 0.0, base * 1.01, base, base * 1.005, 1000.0],
        ];

        let labels = label_candles(&data);
        assert_eq!(labels.len(), 2);
        assert_eq!(labels[0], Action::Long);
        assert_eq!(labels[1], Action::None);
    }

    #[test]
    fn test_label_candles_short_condition() {
        let base = 100.0;
        // Low below SHORT_THRESHOLD, high below LONG_THRESHOLD (no long trigger)
        let data = [
            [0.0, 0.0, base * 1.002, base * 0.998, base, 1000.0],
            [
                0.0,
                0.0,
                base * 1.002,
                base * SHORT_THRESHOLD - 0.001,
                base,
                1000.0,
            ],
        ];

        let labels = label_candles(&data);
        assert_eq!(labels, vec![Action::Short, Action::None]);
    }

    #[test]
    fn test_label_candles_both_conditions() {
        let base = 100.0;
        // Both conditions triggered - should choose Short as tie-breaker
        let data = [
            [0.0, 0.0, base * 1.002, base * 0.998, base, 1000.0],
            [
                0.0,
                0.0,
                base * LONG_THRESHOLD + 0.001,
                base * SHORT_THRESHOLD - 0.001,
                base * 1.003,
                1000.0,
            ],
        ];

        let labels = label_candles(&data);
        assert_eq!(labels.len(), 2);
        assert_eq!(labels[0], Action::Short);
        assert_eq!(labels[1], Action::None);
    }

    #[test]
    fn test_label_candles_multiple_varied() {
        let base = 100.0;
        let data = [
            [0.0, 0.0, base * 1.002, base * 0.998, base, 500.0], // Candle0: No trigger (high=100.2, low=99.8)
            [0.0, 0.0, base * 1.002, base * 0.998, base, 500.0], // Candle1: Long trigger via next candle's high
            [0.0, 0.0, base * 1.02, base * 0.999, base * 1.01, 500.0], // Candle2: Short trigger via next candle's low
            [0.0, 0.0, base * 1.011, base * 0.99, base * 1.005, 500.0], // Candle3: Short trigger (both conditions met)
            [0.0, 0.0, base * 1.02, base * 0.99, base, 500.0], // Candle4: Last candle (None)
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
