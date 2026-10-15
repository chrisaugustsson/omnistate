use omni_api::{AppState, Config};

#[tokio::main]
async fn main() {
    if let Err(message) = run().await {
        eprintln!("omni-api: {message}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    let config = Config::from_env()?;
    omni_api::migrate(&config, 60).await?;
    let state = AppState::new(&config)?;
    state.ensure_bucket(&config, 60).await?;

    let listener = tokio::net::TcpListener::bind(&config.bind)
        .await
        .map_err(|e| format!("cannot listen on {}: {e}", config.bind))?;
    println!("omni-api listening on {}", config.bind);
    axum::serve(listener, omni_api::router(state))
        .with_graceful_shutdown(shutdown())
        .await
        .map_err(|e| e.to_string())
}

/// Resolves on SIGTERM (what `docker stop` sends) or Ctrl-C.
async fn shutdown() {
    use tokio::signal::unix::{SignalKind, signal};
    let mut terminate = signal(SignalKind::terminate()).expect("SIGTERM handler");
    tokio::select! {
        _ = terminate.recv() => {}
        _ = tokio::signal::ctrl_c() => {}
    }
}
