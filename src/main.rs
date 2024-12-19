#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize environment variables
    dotenvy::dotenv()?;

    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter("happychartsv2=debug")
        .init();

    tracing::info!("Starting backtest and improvement process...");

    // Run the backtesting and prompt improvement
    let mut counter = 0;
    while {
        let res = happychartsv2::backtest::run_backtest_and_improve()
            .await
            .map_err(|e| {
                tracing::error!(error=?e, "Backtest and improvement failed");
                e
            })?;
        counter += 1;
        tracing::info!(score=?res, %counter, "Backtest and improvement completed successfully");
        res < 0.7 && counter < 10
    } {}

    tracing::info!("Backtest and improvement completed successfully.");
    Ok(())
}
