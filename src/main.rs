#[tokio::main]
async fn main() {
    // Initialize logging from env
    tracing_subscriber::fmt()
        // .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        // .with_target(false) // Optional: removes the target from log messages
        // .with_ansi(true) // Enables ANSI terminal colors
        // .with_level(true)
        .init();

    tracing::info!("Hello, world!");
}
