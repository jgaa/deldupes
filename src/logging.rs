use anyhow::Result;
use tracing_subscriber::{fmt, EnvFilter};

pub fn init(verbose: u8) -> Result<()> {
    // Base filter:
    // - if RUST_LOG is set, use it
    // - else default to "info" (or "debug" with -v)
    let default_level = match verbose {
        0 => "info",
        1 => "debug",
        _ => "trace",
    };

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(default_level));

    fmt()
        .with_env_filter(filter)
        .with_target(false) // cleaner output: omit crate/module path
        .compact()
        .init();

    Ok(())
}
