use tracing_subscriber::{layer::SubscriberExt as _, util::SubscriberInitExt as _};

use myopic::extractor::html;

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| format!("{}=debug", env!("CARGO_CRATE_NAME")).into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();
    let html_content = include_str!("../temp/crates-io-scrapper.html");
    html::parse(html_content);
}
