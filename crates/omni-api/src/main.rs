use omni_api::{AppState, Config, setup};
use std::io::BufRead;

#[tokio::main]
async fn main() {
    if let Err(message) = run().await {
        eprintln!("omni-api: {message}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None => serve().await,
        Some("setup") => first_run_setup(&args[1..]).await,
        Some(other) => Err(format!("unknown command {other}")),
    }
}

/// Creates the first user, password and workspace; the password is the first
/// line of standard input, so it never shows up in the process list.
async fn first_run_setup(args: &[String]) -> Result<(), String> {
    let setup_args = setup::parse_args(args)?;
    let config = Config::from_env()?;
    let mut line = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut line)
        .map_err(|e| format!("cannot read the password from standard input: {e}"))?;
    let password = line.trim_end_matches(['\n', '\r']);
    setup::run(&config, &setup_args, password).await?;
    println!(
        "Created {} with the workspace {}.",
        setup_args.email, setup_args.workspace
    );
    Ok(())
}

async fn serve() -> Result<(), String> {
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
