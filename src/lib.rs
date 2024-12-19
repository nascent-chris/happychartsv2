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

#[tokio::test]
async fn test_basic() {
    dotenvy::dotenv().unwrap();
    tracing_subscriber::fmt::init();
    let response = analyze_data_gpt("Hello, world!").await.unwrap();
    tracing::info!(?response);
}
