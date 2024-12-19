use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use futures::stream::{FuturesUnordered, StreamExt};
use futures::TryFutureExt;
use serde_json::Value;
use std::fmt::Write as FmtWrite;
use std::fs;
use std::path::Path; // Importing fmt::Write so writeln! works on String

use crate::prompt_builder::build_data_section;
use crate::{
    analyze_data_gpt, candles_to_array, get_candle_data, label_candles, Action, CoinbaseCandle,
    Model,
};

const CANDLE_HOURS: usize = 24; // 24-hour window
const CACHE_DIR: &str = "cache";
const PROMPT_FILE: &str = "prompt.txt";

pub async fn run_backtest_and_improve() -> Result<()> {
    // Ensure cache directory exists
    fs::create_dir_all(CACHE_DIR)?;

    // We'll fetch data for the last N hours, let's say we fetch 48 hours for testing.
    let end = Utc::now();
    let start = end - Duration::hours(48); // 48 hours of data

    // Fetch or load cached data
    let eth_candles = candles_to_array(load_or_fetch("ETH", start, end).await?);
    let btc_candles = candles_to_array(load_or_fetch("BTC", start, end).await?);
    let sol_candles = candles_to_array(load_or_fetch("SOL", start, end).await?);

    // Label ETH data for ground truth
    let labels = label_candles(&eth_candles);

    if eth_candles.len() < CANDLE_HOURS {
        anyhow::bail!("Not enough ETH candles to perform backtesting");
    }

    // Load the current prompt from a file
    let base_prompt = fs::read_to_string(PROMPT_FILE).context("Failed to read base prompt file")?;

    let tasks = (CANDLE_HOURS..eth_candles.len()).filter_map(|i| {
        if btc_candles.len() < i || sol_candles.len() < i {
            // Not enough data for this index
            return None;
        }

        let eth_window = &eth_candles[i - CANDLE_HOURS..i];
        let btc_window = &btc_candles[i - CANDLE_HOURS..i];
        let sol_window = &sol_candles[i - CANDLE_HOURS..i];

        let data_section = build_data_section(eth_window, btc_window, sol_window);
        let full_prompt = format!("{}\n\n{}", base_prompt, data_section);
        let label = labels[i - 1];

        // Each future returns Result<(Action, String, Action)>
        let fut = query_model_and_compare(full_prompt, label).map_ok(move |res| (i, res));

        Some(fut)
    });

    // Convert our tasks into a stream, then apply buffer_unordered(5) to limit concurrency
    let results = futures::stream::iter(tasks).buffer_unordered(20);

    futures::pin_mut!(results);

    let mut correct_count = 0usize;
    let mut total = 0usize;
    let mut failures = Vec::new();

    while let Some(res) = results.next().await {
        let (i, (pred, rationale, label)) = res?;
        total += 1;
        if pred == label {
            correct_count += 1;
        } else {
            failures.push((i, pred, label, rationale));
        }
    }

    let accuracy = if total > 0 {
        correct_count as f64 / total as f64
    } else {
        0.0
    };

    tracing::info!("Backtesting complete. Accuracy: {:.2}%", accuracy * 100.0);

    if !failures.is_empty() {
        tracing::debug!(?failures);
        let improvement_prompt = build_improvement_prompt(&base_prompt, &failures);
        let improved_prompt = analyze_data_gpt(&improvement_prompt, Model::O1Preview).await?;
        fs::write(PROMPT_FILE, improved_prompt)?;
        tracing::info!("Prompt improved and saved to {}", PROMPT_FILE);
    }

    Ok(())
}

async fn load_or_fetch(
    symbol: &str,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<Vec<CoinbaseCandle>> {
    let cache_file = format!("{}/{}_data.json", CACHE_DIR, symbol);
    if Path::new(&cache_file).exists() {
        let data = fs::read_to_string(&cache_file)?;
        let candles: Vec<CoinbaseCandle> =
            serde_json::from_str(&data).context("Failed to deserialize cached candle data")?;
        Ok(candles)
    } else {
        let candles = get_candle_data(symbol, start, end).await?;
        // Serialize and store them in the cache file for next time
        let json = serde_json::to_string(&candles)?;
        fs::write(&cache_file, json)?;
        Ok(candles)
    }
}

async fn query_model_and_compare(
    prompt: String,
    label: Action,
) -> Result<(Action, String, Action)> {
    let response = analyze_data_gpt(&prompt, Model::O1Mini).await?;

    // Clean up the response to remove code fences if present
    let clean_response = response.replace("```json", "").replace("```", "");

    let val: Value = serde_json::from_str(&clean_response)
        .with_context(|| format!("Response not valid JSON: {}", clean_response))?;
    let action_str = val.get("action").and_then(|a| a.as_str()).unwrap_or("none");
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

    Ok((pred, rationale, label))
}

fn build_improvement_prompt(
    base_prompt: &str,
    failures: &[(usize, Action, Action, String)],
) -> String {
    let mut prompt = String::new();
    prompt.push_str("You are an assistant that improves trading prompts.\n");
    prompt.push_str("We have a base prompt (below) that instructs the model to produce an action (long, short, or none) and a brief rationale based on provided ETH, BTC, and SOL market data.\n");
    prompt.push_str("We performed backtesting and found some instances where the model's predicted action did not match the correct action.\n");
    prompt.push_str("Below are some examples of these failures:\n");
    for (i, pred, label, rationale) in failures.iter().take(10) {
        let _ = writeln!(
            prompt,
            "Window {}: Model predicted {:?}, but the correct action was {:?}. Model's rationale: {}",
            i, pred, label, rationale
        );
    }
    prompt.push_str("\nWe need to improve the prompt so that:\n");
    prompt.push_str("- The model is more likely to produce correct 'action' decisions.\n");
    prompt.push_str("- The rationale remains concise and well-aligned with the chosen action.\n");
    prompt.push_str(
        "- The model should not provide disclaimers or mention hypothetical scenarios.\n",
    );
    prompt.push_str("- The model should consistently rely on patterns, correlations, and recent price changes from the data.\n");
    prompt.push_str("- The data is appended directly after the prompt.\n");
    prompt.push_str("\nOriginal Prompt:\n");
    prompt.push_str(base_prompt);
    prompt.push_str("\n\nPlease suggest an improved version of the prompt text (without adding any external formatting or code fences), incorporating the above improvements.\n");

    prompt
}
