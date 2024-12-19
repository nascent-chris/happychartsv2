use std::env;

use anyhow::{Context as _, Result};
use serde_json::{json, Value};

pub async fn analyze_data_gpt(prompt: &str) -> Result<String> {
    let api_key = env::var("OPENAI_API_KEY")?;

    let body = json!({
        "model": "o1-mini",
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

pub fn label_candles(data: &[[f64; 6]]) -> Vec<&'static str> {
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
                (true, true) => "long", // tie-break: choose "long"
                (true, false) => "long",
                (false, true) => "short",
                (false, false) => "none",
            }
        })
        .collect::<Vec<_>>();

    // The last candle has no future candle, so "none"
    labels.push("none");

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
        assert_eq!(labels, vec!["short", "short", "none"]);
    }

    #[tokio::test]
    async fn test_basic() {
        dotenvy::dotenv().unwrap();
        tracing_subscriber::fmt::init();
        let response = analyze_data_gpt("Hello, world!").await.unwrap();
        tracing::info!(?response);
    }
}
