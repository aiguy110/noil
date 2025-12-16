# 13: Web Server Graceful Shutdown

Implement graceful shutdown for the web server instead of aborting.

## Priority

LOW - Minimal practical impact for current use case.

## Current State

At `src/cli/run.rs:283-284`:
```rust
// Note: web server doesn't gracefully shutdown yet, so we just abort it
web_handle.abort();
```

The web server (`src/web/server.rs`) uses `axum::serve()` which supports graceful shutdown but isn't configured for it.

## Implementation

Axum's `serve()` returns a future that can be combined with a shutdown signal using `with_graceful_shutdown()`.

### Update server.rs

```rust
// src/web/server.rs
use tokio::sync::watch;

pub async fn run_server(
    storage: Arc<dyn Storage>,
    config: WebConfig,
    mut shutdown_rx: watch::Receiver<bool>,
) -> Result<(), Box<dyn std::error::Error>> {
    let app_state = AppState { storage };

    let app = Router::new()
        .route("/health", get(health_check))
        // ... routes ...
        .with_state(app_state);

    let listener = tokio::net::TcpListener::bind(&config.listen).await?;
    tracing::info!("Web server listening on {}", config.listen);

    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = shutdown_rx.wait_for(|&v| v).await;
            tracing::info!("Web server shutting down gracefully");
        })
        .await?;

    Ok(())
}
```

### Update run.rs

```rust
// src/cli/run.rs

// Existing shutdown channel (already created at line 200)
let (shutdown_tx, shutdown_rx) = watch::channel(false);

// Pass receiver to web server
let web_shutdown_rx = shutdown_rx.clone();
let web_handle = tokio::spawn(async move {
    run_server(web_storage, web_config, web_shutdown_rx)
        .await
        .map_err(|e| RunError::WebServer(e.to_string()))
});

// In shutdown section, replace abort() with:
match tokio::time::timeout(Duration::from_secs(5), web_handle).await {
    Ok(Ok(Ok(()))) => info!("Web server stopped gracefully"),
    Ok(Ok(Err(e))) => error!(error = %e, "Web server error"),
    Ok(Err(e)) => error!(error = %e, "Web server join error"),
    Err(_) => {
        warn!("Web server shutdown timed out, aborting");
        // web_handle already consumed, task will be dropped
    }
}
```

## Files to Modify

1. `src/web/server.rs`: Add shutdown receiver parameter
2. `src/cli/run.rs`: Pass shutdown channel, wait for graceful shutdown

## Acceptance Criteria

- [ ] Web server receives shutdown signal
- [ ] In-flight requests complete (up to timeout)
- [ ] Clean shutdown logged
- [ ] Timeout fallback prevents hanging
- [ ] No changes to public API behavior
