# Issue: Collector HTTP Endpoints Missing + Inconsistent Error Format

**Status**: Open
**Priority**: High
**Component**: Collector HTTP Server / Error Handling
**Date Reported**: 2026-02-02
**Can Be Fixed In Single Session**: Yes

## Problem 1: Collector HTTP Endpoints Return 404

### Problem Description

The collector's HTTP server is running and bound to the correct port, but all HTTP endpoints return 404 Not Found, including `/health`, `/status`, and the batch API endpoint.

### Expected Behavior

According to the collector config and documentation:

1. **Health Check**: `GET /health` should return server health status
2. **Status UI**: `GET /status` should provide read-only status page (when `status_ui.enabled: true`)
3. **Batch API**: `GET /api/batches?sequence=N` should serve batches to parent instances

### Actual Behavior

```bash
# Collector is running on port 7105
curl "http://localhost:7105/health"
# Returns: HTTP 404 (empty response)

curl "http://localhost:7105/status"
# Returns: HTTP 404 (empty response)

curl "http://localhost:7105/api/batches?sequence=0"
# Returns: HTTP 404 (empty response)
```

The collector starts successfully and logs indicate the HTTP server is running:

```
INFO noil::collector::server: Starting collector HTTP server addr=127.0.0.1:7105
```

### Root Cause

The collector HTTP server is missing route definitions. Investigation needed in:

**File**: `src/collector/server.rs` (or `src/collector/mod.rs`)

Likely causes:
1. Routes not registered with the Axum router
2. Server starts but has empty/incomplete route table
3. Endpoint handlers not implemented

### Impact

**Critical Issues**:
- Parent instances cannot pull batches from collectors (breaks distributed mode)
- No way to check collector health or status
- Cannot debug or monitor collector state
- Distributed deployment completely non-functional

**Workarounds**:
- None - collector mode unusable for distributed deployments
- Local mode (single instance) works fine

---

## Problem 2: Inconsistent Error Format for Invalid UUID

### Problem Description

Most API endpoints return JSON-formatted errors, but the UUID validation error returns plain text. This breaks API clients expecting consistent JSON responses.

### Expected Behavior

All errors should return JSON with consistent format:

```json
{
  "error": {
    "code": "INVALID_UUID",
    "message": "UUID parsing failed: invalid character..."
  }
}
```

With appropriate HTTP status code (400 Bad Request).

### Actual Behavior

```bash
# Parent API - Invalid UUID in path parameter
curl "http://localhost:7104/api/logs/invalid-uuid-format"
# Returns: Plain text (not JSON)
# "Invalid URL: UUID parsing failed: invalid character: expected an optional prefix of `urn:uuid:` followed by [0-9a-fA-F-], found `i` at 1"

# For comparison, other errors are properly formatted:
curl "http://localhost:7104/api/logs/00000000-0000-0000-0000-000000000000"
# Returns: {"error":{"code":"NOT_FOUND","message":"Log not found: 00000000-0000-0000-0000-000000000000"}}

curl "http://localhost:7104/api/fibers"
# Returns: {"error":{"code":"BAD_REQUEST","message":"fiber type parameter required"}}
```

### Root Cause

UUID parsing happens at the Axum path parameter extraction layer, before reaching the handler function. The error is likely coming from Axum's default `Path<Uuid>` rejection handler.

**File**: `src/web/api.rs`

Path parameter extraction:
```rust
async fn get_log(Path(log_id): Path<Uuid>) -> Result<Json<LogRecord>, ApiError>
```

When `log_id` fails to parse, Axum returns its default error format instead of our custom `ApiError` format.

### Impact

**API Client Issues**:
- JSON parsing fails on error responses
- Cannot reliably detect error conditions
- Inconsistent error handling code needed
- Breaks clients expecting JSON responses

---

## Suggested Fixes

### Fix 1: Implement Collector HTTP Endpoints

**File**: `src/collector/server.rs`

Add route definitions to the Axum router:

```rust
use axum::{
    routing::{get, post},
    Router,
    Json,
};

pub async fn start_server(addr: SocketAddr, state: CollectorState) -> Result<(), Error> {
    let app = Router::new()
        .route("/health", get(health_check))
        .route("/status", get(status_page))
        .route("/api/batches", get(get_batches))
        .route("/api/batches/:sequence/ack", post(acknowledge_batch))
        .with_state(state);

    // ... bind and serve
}

async fn health_check() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

async fn get_batches(
    Query(params): Query<BatchQuery>,
    State(state): State<CollectorState>,
) -> Result<Json<BatchResponse>, CollectorError> {
    // Implementation
}
```

Refer to parent's `/health` implementation in `src/web/server.rs` for structure.

### Fix 2: Add Custom UUID Rejection Handler

**File**: `src/web/api.rs`

Add custom extractor with proper error handling:

```rust
use axum::{
    extract::{Path, rejection::PathRejection},
    response::IntoResponse,
};

// Custom UUID path extractor
pub struct UuidPath(pub Uuid);

#[async_trait]
impl<S> FromRequestParts<S> for UuidPath
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        match Path::<Uuid>::from_request_parts(parts, state).await {
            Ok(Path(uuid)) => Ok(UuidPath(uuid)),
            Err(_) => Err(ApiError::BadRequest("Invalid UUID format".to_string())),
        }
    }
}

// Update handlers to use UuidPath instead of Path<Uuid>
async fn get_log(UuidPath(log_id): UuidPath) -> Result<Json<LogRecord>, ApiError> {
    // ...
}
```

Alternatively, implement a custom rejection handler for the entire app.

---

## Testing

### Test Collector Endpoints

```bash
# After fix - all should return valid responses
curl http://localhost:7105/health | jq .
# Expected: {"status": "ok", "version": "0.1.0"}

curl http://localhost:7105/status
# Expected: HTML status page or JSON status data

curl "http://localhost:7105/api/batches?sequence=0" | jq .
# Expected: {"batches": [...], "has_more": true/false}
```

### Test UUID Error Format

```bash
# After fix - should return JSON
curl http://localhost:7104/api/logs/invalid-uuid | jq .
# Expected: {"error": {"code": "BAD_REQUEST", "message": "Invalid UUID format"}}

# Verify HTTP status code
curl -w "\nHTTP %{http_code}\n" http://localhost:7104/api/logs/invalid-uuid
# Expected: HTTP 400
```

---

## Related Files

### Collector Endpoints
- `src/collector/server.rs` - Collector HTTP server (needs route definitions)
- `src/collector/mod.rs` - Collector module exports
- `src/web/server.rs` - Parent HTTP server (reference implementation)
- `specs/COLLECTOR_MODE.md` - Collector API specification

### Error Handling
- `src/web/api.rs` - API error types and handlers
- `src/web/server.rs` - Router setup and middleware

---

## Priority Justification

**High Priority** because:
1. Collector mode is completely non-functional without HTTP endpoints
2. Distributed deployment is a documented feature that doesn't work
3. Fixes are straightforward and can be completed in one session
4. Error format inconsistency affects all API consumers
5. Both issues have clear, well-defined solutions

## Implementation Checklist

- [ ] Add collector HTTP routes (`/health`, `/status`, `/api/batches`)
- [ ] Implement batch serving endpoint
- [ ] Implement acknowledgment endpoint
- [ ] Add custom UUID path extractor with JSON error handling
- [ ] Update all UUID path parameters to use custom extractor
- [ ] Test all collector endpoints return valid responses
- [ ] Test invalid UUID returns JSON error with HTTP 400
- [ ] Update API documentation if needed
- [ ] Add integration tests for collector HTTP API
