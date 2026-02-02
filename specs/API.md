# Noil HTTP API Documentation

The Noil web API provides REST endpoints for querying logs, fibers, and their relationships.

## Configuration

The API server is configured in `config.yml`:

```yaml
web:
  listen: 127.0.0.1:7104  # Address to bind the web server
```

The server starts automatically when you run `noil` and runs concurrently with the log processing pipeline.

## Base URL

All API endpoints are prefixed with `/api/` except for the health check endpoint.

Example base URL: `http://127.0.0.1:7104`

## Authentication

The current version does not include authentication. This is suitable for local development and trusted network environments.

## Common Patterns

### Pagination

List endpoints support pagination via query parameters:

- `limit`: Maximum number of results to return (default: 100, max: 1000)
- `offset`: Number of results to skip (default: 0)

All paginated responses include:

```json
{
  "total": 250,
  "limit": 100,
  "offset": 0,
  "logs": [...],  // or "fibers": [...]
}
```

### Error Responses

All errors follow a consistent format:

```json
{
  "error": {
    "code": "ERROR_CODE",
    "message": "Human-readable error message"
  }
}
```

HTTP status codes:
- `200 OK`: Request succeeded
- `400 Bad Request`: Invalid request parameters
- `404 Not Found`: Resource not found
- `500 Internal Server Error`: Server error

Error codes:
- `NOT_FOUND`: Resource does not exist
- `BAD_REQUEST`: Invalid query parameters
- `INTERNAL_ERROR`: Internal server error

## Endpoints

### Health Check

Check if the API server is running.

**Request:**
```
GET /health
```

**Response:**
```json
{
  "status": "ok",
  "version": "0.1.0"
}
```

**Example:**
```bash
curl http://localhost:7104/health
```

---

### List Logs

Retrieve logs with optional filtering and pagination.

**Request:**
```
GET /api/logs?start={timestamp}&end={timestamp}&source={source_id}&limit={n}&offset={n}
```

**Query Parameters:**

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `start` | ISO8601 timestamp | No | Filter logs >= this timestamp (default: 24 hours ago) |
| `end` | ISO8601 timestamp | No | Filter logs <= this timestamp (default: now) |
| `source` | string | No | Filter by source_id (currently not implemented in storage) |
| `limit` | integer | No | Max results (default: 100, max: 1000) |
| `offset` | integer | No | Pagination offset (default: 0) |

**Response:**
```json
{
  "logs": [
    {
      "id": "550e8400-e29b-41d4-a716-446655440000",
      "timestamp": "2025-12-16T10:30:00Z",
      "source_id": "nginx_access",
      "raw_text": "[16/Dec/2025:10:30:00 +0000] GET /api/logs HTTP/1.1",
      "ingestion_time": "2025-12-16T10:30:01.234Z"
    },
    {
      "id": "550e8400-e29b-41d4-a716-446655440001",
      "timestamp": "2025-12-16T10:30:05Z",
      "source_id": "application_log",
      "raw_text": "2025-12-16T10:30:05.123Z INFO Request processed successfully",
      "ingestion_time": "2025-12-16T10:30:05.456Z"
    }
  ],
  "total": 2,
  "limit": 100,
  "offset": 0
}
```

**Examples:**

```bash
# Get logs from the last hour
curl "http://localhost:7104/api/logs?start=$(date -u -d '1 hour ago' +%Y-%m-%dT%H:%M:%SZ)"

# Get logs with pagination
curl "http://localhost:7104/api/logs?limit=50&offset=100"

# Get logs in a specific time range
curl "http://localhost:7104/api/logs?start=2025-12-16T10:00:00Z&end=2025-12-16T11:00:00Z"
```

---

### Get Single Log

Retrieve a specific log by its UUID.

**Request:**
```
GET /api/logs/{log_id}
```

**Path Parameters:**

| Parameter | Type | Description |
|-----------|------|-------------|
| `log_id` | UUID | The log's unique identifier |

**Response:**
```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "timestamp": "2025-12-16T10:30:00Z",
  "source_id": "nginx_access",
  "raw_text": "[16/Dec/2025:10:30:00 +0000] GET /api/logs HTTP/1.1",
  "ingestion_time": "2025-12-16T10:30:01.234Z"
}
```

**Error Response (404):**
```json
{
  "error": {
    "code": "NOT_FOUND",
    "message": "Log not found: 550e8400-e29b-41d4-a716-446655440000"
  }
}
```

**Example:**
```bash
curl http://localhost:7104/api/logs/550e8400-e29b-41d4-a716-446655440000
```

---

### Get Log's Fibers

Retrieve all fibers that contain a specific log.

**Request:**
```
GET /api/logs/{log_id}/fibers
```

**Path Parameters:**

| Parameter | Type | Description |
|-----------|------|-------------|
| `log_id` | UUID | The log's unique identifier |

**Response:**
```json
{
  "fibers": [
    {
      "id": "660e8400-e29b-41d4-a716-446655440000",
      "fiber_type": "request_trace",
      "attributes": {
        "mac": "aa:bb:cc:11:22:33",
        "program1_thread": "42",
        "ip": "192.168.1.100",
        "src_port": 50123,
        "connection": "192.168.1.100:50123->8080"
      },
      "first_activity": "2025-12-16T10:30:00Z",
      "last_activity": "2025-12-16T10:30:15Z",
      "closed": true
    },
    {
      "id": "660e8400-e29b-41d4-a716-446655440001",
      "fiber_type": "nginx_all",
      "attributes": {
        "source_marker": "nginx"
      },
      "first_activity": "2025-12-16T00:00:00Z",
      "last_activity": "2025-12-16T10:30:00Z",
      "closed": false
    }
  ],
  "total": 2,
  "limit": 9223372036854775807,
  "offset": 0
}
```

**Example:**
```bash
curl http://localhost:7104/api/logs/550e8400-e29b-41d4-a716-446655440000/fibers
```

---

### List Fibers

Retrieve fibers with optional filtering and pagination.

**Request:**
```
GET /api/fibers?type={fiber_type}&closed={boolean}&limit={n}&offset={n}
```

**Query Parameters:**

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `type` | string | **Yes** | Filter by fiber type name |
| `closed` | boolean | No | Filter by closed status (true/false) |
| `limit` | integer | No | Max results (default: 100, max: 1000) |
| `offset` | integer | No | Pagination offset (default: 0) |

**Response:**
```json
{
  "fibers": [
    {
      "id": "660e8400-e29b-41d4-a716-446655440000",
      "fiber_type": "request_trace",
      "attributes": {
        "mac": "aa:bb:cc:11:22:33",
        "program1_thread": "42",
        "ip": "192.168.1.100"
      },
      "first_activity": "2025-12-16T10:30:00Z",
      "last_activity": "2025-12-16T10:30:15Z",
      "closed": true
    }
  ],
  "total": 1,
  "limit": 100,
  "offset": 0
}
```

**Error Response (400):**
```json
{
  "error": {
    "code": "BAD_REQUEST",
    "message": "fiber type parameter required"
  }
}
```

**Examples:**

```bash
# Get all request_trace fibers
curl "http://localhost:7104/api/fibers?type=request_trace"

# Get only closed fibers
curl "http://localhost:7104/api/fibers?type=request_trace&closed=true"

# Get only open fibers with pagination
curl "http://localhost:7104/api/fibers?type=request_trace&closed=false&limit=50"
```

---

### Get Single Fiber

Retrieve a specific fiber by its UUID.

**Request:**
```
GET /api/fibers/{fiber_id}
```

**Path Parameters:**

| Parameter | Type | Description |
|-----------|------|-------------|
| `fiber_id` | UUID | The fiber's unique identifier |

**Response:**
```json
{
  "id": "660e8400-e29b-41d4-a716-446655440000",
  "fiber_type": "request_trace",
  "attributes": {
    "mac": "aa:bb:cc:11:22:33",
    "program1_thread": "42",
    "program2_thread": "17",
    "ip": "192.168.1.100",
    "src_port": 50123,
    "dst_port": 8080,
    "connection": "192.168.1.100:50123->8080"
  },
  "first_activity": "2025-12-16T10:30:00Z",
  "last_activity": "2025-12-16T10:30:15Z",
  "closed": true
}
```

**Error Response (404):**
```json
{
  "error": {
    "code": "NOT_FOUND",
    "message": "Fiber not found: 660e8400-e29b-41d4-a716-446655440000"
  }
}
```

**Example:**
```bash
curl http://localhost:7104/api/fibers/660e8400-e29b-41d4-a716-446655440000
```

---

### Get Fiber's Logs

Retrieve all logs that belong to a specific fiber, in chronological order.

**Request:**
```
GET /api/fibers/{fiber_id}/logs?limit={n}&offset={n}
```

**Path Parameters:**

| Parameter | Type | Description |
|-----------|------|-------------|
| `fiber_id` | UUID | The fiber's unique identifier |

**Query Parameters:**

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `limit` | integer | No | Max results (default: 100, max: 1000) |
| `offset` | integer | No | Pagination offset (default: 0) |

**Response:**
```json
{
  "logs": [
    {
      "id": "550e8400-e29b-41d4-a716-446655440000",
      "timestamp": "2025-12-16T10:30:00Z",
      "source_id": "program1",
      "raw_text": "thread-42 Received connection from 192.168.1.100",
      "ingestion_time": "2025-12-16T10:30:00.123Z"
    },
    {
      "id": "550e8400-e29b-41d4-a716-446655440001",
      "timestamp": "2025-12-16T10:30:02Z",
      "source_id": "program1",
      "raw_text": "thread-42 Processing request from MAC aa:bb:cc:11:22:33",
      "ingestion_time": "2025-12-16T10:30:02.456Z"
    },
    {
      "id": "550e8400-e29b-41d4-a716-446655440002",
      "timestamp": "2025-12-16T10:30:05Z",
      "source_id": "program2",
      "raw_text": "thread-17 Handling request for MAC aa:bb:cc:11:22:33",
      "ingestion_time": "2025-12-16T10:30:05.789Z"
    },
    {
      "id": "550e8400-e29b-41d4-a716-446655440003",
      "timestamp": "2025-12-16T10:30:15Z",
      "source_id": "program2",
      "raw_text": "thread-17 Request complete",
      "ingestion_time": "2025-12-16T10:30:15.012Z"
    }
  ],
  "total": 4,
  "limit": 100,
  "offset": 0
}
```

**Example:**
```bash
# Get all logs for a fiber
curl http://localhost:7104/api/fibers/660e8400-e29b-41d4-a716-446655440000/logs

# Get logs with pagination
curl "http://localhost:7104/api/fibers/660e8400-e29b-41d4-a716-446655440000/logs?limit=10&offset=0"
```

---

## Common Use Cases

### Tracing a Request Through the System

1. Start with a known log (e.g., from a user report):
   ```bash
   curl http://localhost:7104/api/logs/550e8400-e29b-41d4-a716-446655440000
   ```

2. Find all fibers containing this log:
   ```bash
   curl http://localhost:7104/api/logs/550e8400-e29b-41d4-a716-446655440000/fibers
   ```

3. Get all logs in the relevant fiber to see the complete trace:
   ```bash
   curl http://localhost:7104/api/fibers/660e8400-e29b-41d4-a716-446655440000/logs
   ```

### Monitoring Recent Activity

Get logs from the last 5 minutes:
```bash
curl "http://localhost:7104/api/logs?start=$(date -u -d '5 minutes ago' +%Y-%m-%dT%H:%M:%SZ)"
```

### Finding Open Sessions

Get all open fibers of a specific type:
```bash
curl "http://localhost:7104/api/fibers?type=request_trace&closed=false"
```

### Analyzing a Specific Fiber

Get fiber details and all its logs:
```bash
FIBER_ID="660e8400-e29b-41d4-a716-446655440000"

# Get fiber metadata
curl http://localhost:7104/api/fibers/$FIBER_ID | jq .

# Get all logs in chronological order
curl http://localhost:7104/api/fibers/$FIBER_ID/logs | jq '.logs[] | {timestamp, source_id, raw_text}'
```

## Response Fields Reference

### Log Object

| Field | Type | Description |
|-------|------|-------------|
| `id` | UUID | Unique log identifier |
| `timestamp` | ISO8601 | Log timestamp (extracted from log content) |
| `source_id` | string | Source name from config (e.g., "nginx_access") |
| `raw_text` | string | Original log line content |
| `ingestion_time` | ISO8601 | When the log was ingested by Noil |

### Fiber Object

| Field | Type | Description |
|-------|------|-------------|
| `id` | UUID | Unique fiber identifier |
| `fiber_type` | string | Fiber type name from config (e.g., "request_trace") |
| `attributes` | object | Key-value pairs of fiber attributes |
| `first_activity` | ISO8601 | Timestamp of first log in fiber |
| `last_activity` | ISO8601 | Timestamp of most recent log in fiber |
| `closed` | boolean | Whether fiber is closed (no more logs can join) |

## Notes

- All timestamps are in ISO 8601 format with UTC timezone
- UUIDs are in standard 8-4-4-4-12 format
- The `attributes` field in fiber objects is a JSON object with dynamic keys based on fiber type configuration
- Derived attributes (computed via interpolation) are included in the `attributes` object
- Keys used for fiber matching are also included in `attributes` even after fiber closure
- Pagination `total` reflects the count of returned results, not necessarily all matching records (this may be improved in future versions)

## Future Enhancements

Planned additions to the API:

- WebSocket endpoint for live log streaming
- Query logs by source_id filter
- List all fibers without requiring type parameter
- Full-text search in log content
- Time-series aggregations
- Export endpoints (CSV, JSON Lines)
- Authentication and authorization
- Rate limiting
- CORS configuration
