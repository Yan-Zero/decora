//! Decora executable: reads config.yaml from the current directory by default, or accepts a single config path.
use decora::{Config, Gateway};
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Keep reqwest/hyper debug tracing disabled to prevent accidental log-level changes from exposing request details.
    tracing_subscriber::fmt()
        .with_env_filter("decora=info")
        .with_target(false)
        .init();
    let mut args = std::env::args_os().skip(1);
    let path = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("config.yaml"));
    if args.next().is_some() {
        return Err("usage: decora [config.yaml]".into());
    }
    let gateway = Gateway::new(Config::load(&path)?)?;
    let listener = tokio::net::TcpListener::bind(gateway.listen()).await?;
    tracing::info!("Decora started; /healthz does not imply upstreams are reachable");
    axum::serve(listener, gateway.router())
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
