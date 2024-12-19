#[tokio::main]
async fn main() {
    // Initialize logging
    tracing_subscriber::fmt().init();

    tracing::info!("Starting backtest and improvement process...");

    // Run the backtesting and prompt improvement
    if let Err(e) = happychartsv2::backtest::run_backtest_and_improve().await {
        tracing::error!(error=?e, "Backtest and improvement failed");
    } else {
        tracing::info!("Backtest and improvement completed successfully.");
    }
}
