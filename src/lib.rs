pub mod backtest;
pub mod prompt_builder;

use std::env;

use anyhow::{Context as _, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Copy)]
pub enum Model {
    O1,
    O1Mini,
}

impl Model {
    pub fn as_str(&self) -> &str {
        match self {
            Model::O1 => "o1",
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
    let api_key = env::var("OPENAI_API_KEY")?;

    let body = json!({
        "model": model.as_str(),
        "messages": [
            {
                "role": "user",
                "content": [
                    {
                        "type": "text",
                        "text": prompt
                    },
                    // {
                    //     "type": "image_url",
                    //     "image_url": {
                    //         "url": base64_url
                    //     }
                    // }
                ]
            }
        ]
    });

    let client = reqwest::Client::new();
    tracing::debug!("Sending request to OpenAI API");
    let response = client
        .post("https://api.openai.com/v1/chat/completions")
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {api_key}"))
        .timeout(std::time::Duration::from_secs(60 * 5))
        .json(&body)
        .send()
        .await?;
    tracing::debug!("Received response from OpenAI API");

    let mut value = response.json::<Value>().await?;

    let content = value
        .get_mut("choices")
        .context("no choices in response")?
        .take()
        .as_array_mut()
        .map(std::mem::take)
        .context("no choice in response")?
        .into_iter()
        .next()
        .context("no choice in response")?
        .get_mut("message")
        .context("no message in response")?
        .take()
        .get_mut("content")
        .context("no content in response")?
        .take()
        .as_str()
        .context("content is not a string")?
        .to_string();

    Ok(content)
}

pub fn label_candles(data: &[[f64; 6]]) -> Vec<Action> {
    use Action::*;
    // For convenience, define indexes into the candle array
    const HIGH: usize = 2;
    const LOW: usize = 3;
    const CLOSE: usize = 4;

    // Profit threshold multipliers
    const LONG_THRESHOLD: f64 = 1.003; // +0.3%
    const SHORT_THRESHOLD: f64 = 0.997; // -0.3%

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

    #[test]
    fn test_label_candles() {
        // A small test with dummy data
        let data = [
            [0.0, 0.0, 3603.0, 3599.99, 3594.88, 100.0],
            [0.0, 0.0, 3600.0, 3565.45, 3599.99, 100.0],
            [0.0, 0.0, 3570.86, 3558.89, 3565.52, 100.0],
        ];

        let labels = label_candles(&data);

        assert_eq!(labels.len(), 3);
        // Just a sanity check with the given logic:
        // For the first candle:
        //   Close=3594.88, next High=3600 (<3594.88*1.003=3605.76? no), next Low=3565.45 (<3594.88*0.997=3583.09 yes) → "short"
        // For the second candle:
        //   Close=3599.99, next High=3570.86 (<3609.0 no), next Low=3558.89 (<3599.99*0.997=3591.0 yes) → "short"
        // Last candle = "none"
        assert_eq!(labels, vec![Action::Short, Action::Short, Action::None]);
    }

    #[tokio::test]
    async fn test_basic() {
        dotenvy::dotenv().unwrap();
        tracing_subscriber::fmt::init();
        let response = analyze_data_gpt("Hello, world!", Model::O1Mini)
            .await
            .unwrap();
        tracing::info!(?response);
    }
}
