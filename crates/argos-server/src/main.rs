use argos_server::{
    config::Config,
    server::{AppState, router},
    store::Store,
};
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "argos_server=info,tower_http=info".into()),
        )
        .init();
    let config = Config::load().map_err(std::io::Error::other)?;
    let store = Store::open(&config.database).await?;
    let listener = tokio::net::TcpListener::bind(config.bind).await?;
    tracing::info!(bind=%config.bind,demo=config.demo,"ARGOS web service starting");
    let state = AppState::new(config, store)
        .await
        .map_err(std::io::Error::other)?;
    let worker = state.worker.clone();
    let shutdown_worker = worker.clone();
    axum::serve(listener, router(state))
        .with_graceful_shutdown(async move {
            shutdown_signal().await;
            shutdown_worker.stop();
        })
        .await?;
    worker.stop();
    Ok(())
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut term) => {
                tokio::select! {_ = tokio::signal::ctrl_c()=>{},_ = term.recv()=>{}}
            }
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
