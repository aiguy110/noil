# 09: Web Server and API

Implement basic HTTP API for querying logs and fibers.

## Location

`src/web/server.rs`, `src/web/api.rs`

## Server Setup

```rust
pub async fn run_server(
    storage: Arc<dyn Storage>,
    config: WebConfig,
) -> Result<()> {
    let app = Router::new()
        .route("/api/logs", get(list_logs))
        .route("/api/logs/:id", get(get_log))
        .route("/api/logs/:id/fibers", get(get_log_fibers))
        .route("/api/fibers", get(list_fibers))
        .route("/api/fibers/:id", get(get_fiber))
        .route("/api/fibers/:id/logs", get(get_fiber_logs))
        .route("/health", get(health_check))
        .with_state(AppState { storage });

    let listener = tokio::net::TcpListener::bind(&config.listen).await?;
    tracing::info!("Web server listening on {}", config.listen);
    axum::serve(listener, app).await?;
    Ok(())
}
```

## API Endpoints

### `GET /api/logs`

List logs with pagination and optional filters.

Query params:
- `start`: ISO8601 timestamp, filter logs >= this time
- `end`: ISO8601 timestamp, filter logs <= this time
- `source`: filter by source_id
- `limit`: max results (default 100, max 1000)
- `offset`: pagination offset

Response:
```json
{
  "logs": [
    {
      "id": "uuid",
      "timestamp": "2025-12-04T02:42:11Z",
      "source_id": "nginx_access",
      "raw_text": "log line content...",
      "ingestion_time": "2025-12-04T02:42:12Z"
    }
  ],
  "total": 1000,
  "limit": 100,
  "offset": 0
}
```

### `GET /api/logs/:id`

Get single log by UUID.

Response: single log object or 404.

### `GET /api/logs/:id/fibers`

Get all fibers containing this log.

Response:
```json
{
  "fibers": [
    {
      "id": "uuid",
      "fiber_type": "request_trace",
      "attributes": {...},
      "first_activity": "...",
      "last_activity": "...",
      "closed": true
    }
  ]
}
```

### `GET /api/fibers`

List fibers with pagination and optional filters.

Query params:
- `type`: filter by fiber_type
- `closed`: boolean, filter by closed status
- `limit`, `offset`: pagination

Response:
```json
{
  "fibers": [...],
  "total": 500,
  "limit": 100,
  "offset": 0
}
```

### `GET /api/fibers/:id`

Get single fiber by UUID.

### `GET /api/fibers/:id/logs`

Get all logs belonging to this fiber, in timestamp order.

Query params:
- `limit`, `offset`: pagination

Response:
```json
{
  "logs": [...],
  "total": 50,
  "limit": 100,
  "offset": 0
}
```

### `GET /health`

Health check endpoint.

Response:
```json
{
  "status": "ok",
  "version": "0.1.0"
}
```

## Error Responses

Use consistent error format:

```json
{
  "error": {
    "code": "NOT_FOUND",
    "message": "Fiber not found: uuid"
  }
}
```

HTTP status codes:
- 200: success
- 400: bad request (invalid params)
- 404: not found
- 500: internal error

## Handler Implementation

```rust
async fn get_fiber_logs(
    State(state): State<AppState>,
    Path(fiber_id): Path<Uuid>,
    Query(params): Query<PaginationParams>,
) -> Result<Json<LogsResponse>, ApiError> {
    let logs = state.storage
        .get_fiber_logs(fiber_id, params.limit(), params.offset())
        .await
        .map_err(|e| match e {
            StorageError::NotFound(_) => ApiError::NotFound(format!("Fiber {}", fiber_id)),
            e => ApiError::Internal(e.to_string()),
        })?;

    Ok(Json(LogsResponse {
        logs,
        total: logs.len(),  // TODO: separate count query
        limit: params.limit(),
        offset: params.offset(),
    }))
}
```

## Integration with Pipeline

The web server runs concurrently with the pipeline:

```rust
// In cli/run.rs
tokio::spawn(run_server(storage.clone(), config.web));
```

Storage is shared (Arc) between pipeline and web server.

## Future: Embedded Frontend

For MVP, API-only is sufficient. Later tickets can add:
- Static file serving from embedded assets
- WebSocket for live updates

## Acceptance Criteria

- Server starts on configured address
- All endpoints return correct data
- Pagination works
- Filtering works
- Errors return proper status codes and messages
- Concurrent requests handled correctly
