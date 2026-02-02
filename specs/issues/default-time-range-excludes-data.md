# Issue: Default Time Range on `/api/logs` Excludes Historical Data

**Status**: Open
**Priority**: Low
**Component**: Web API / Storage
**Date Reported**: 2026-02-02

## Problem Description

The `/api/logs` endpoint returns zero results when called without explicit time range parameters, even when logs exist in the database. This creates a confusing user experience where the database contains data but appears empty.

## Expected Behavior

When `/api/logs` is called without time parameters, one of the following should happen:

**Option 1** (Recommended): Return all logs (with pagination)
```bash
curl "http://localhost:7104/api/logs?limit=100"
# Should return up to 100 most recent logs regardless of age
```

**Option 2**: Document the default time range clearly
- API documentation should explicitly state the default range (e.g., "last 24 hours")
- Consider adding a response header indicating the applied time range
- Return a helpful message or warning when no logs match the default range

## Actual Behavior

```bash
# Without parameters - returns empty
curl "http://localhost:7104/api/logs"
# {"logs": [], "total": 0, "limit": 100, "offset": 0}

# With explicit time range covering sample data - works correctly
curl "http://localhost:7104/api/logs?start=2025-01-11T00:00:00Z&end=2025-01-11T23:59:59Z"
# {"logs": [...41 logs...], "total": 41, ...}
```

The default time range appears to be "recent logs only" (possibly last 24 hours from current time), but this is not documented and causes confusion when working with:
- Historical sample data
- Reprocessed logs
- Archived data
- Development/testing scenarios

## Impact

**Usability Issues**:
- New users think the system has no data when logs actually exist
- Unclear why `/api/fibers` shows fibers but `/api/logs` is empty
- Forces all API consumers to always specify time ranges
- Particularly problematic for sample configs and demos

**Workarounds**:
- Always specify `start` and `end` parameters
- Use very wide time ranges: `?start=2020-01-01T00:00:00Z&end=2030-01-01T00:00:00Z`

## Root Cause Investigation Needed

Check the implementation in `src/web/api.rs`:

1. What is the actual default time range logic?
2. Where are the default `start` and `end` values set?
3. Is the default based on wall clock time or logical clock time?

Expected location: `list_logs()` handler in `src/web/api.rs`

## Suggested Fixes

### Quick Fix (Minimal Change)
Add clear documentation to API spec and error message when no results:

```json
{
  "logs": [],
  "total": 0,
  "limit": 100,
  "offset": 0,
  "info": "No logs found in default time range (last 24 hours). Specify start/end parameters to query historical logs."
}
```

### Recommended Fix
Remove default time filtering entirely:
- Let users query all logs by default (with pagination)
- Only apply time filtering when explicitly requested
- This matches typical database/API behavior

### Alternative Fix
Add a configuration option:

```yaml
web:
  api:
    default_log_time_range: all  # or: "24h", "7d", etc.
```

## Related Files

- `src/web/api.rs` - List logs endpoint implementation
- `src/storage/duckdb.rs` - Query execution with time filters
- `specs/API.md` - API documentation (needs update)

## Test Cases

After fix:
1. Verify `/api/logs` without parameters returns logs (or documents default clearly)
2. Verify historical sample data is accessible
3. Verify explicit time ranges still work correctly
4. Update API documentation with actual default behavior
5. Add integration test for default time range behavior

## API Documentation Update Needed

Current docs (specs/API.md) state:

```
| `start` | ISO8601 timestamp | No | Filter logs >= this timestamp (default: 24 hours ago) |
| `end` | ISO8601 timestamp | No | Filter logs <= this timestamp (default: now) |
```

This should be verified and updated to match actual implementation, or implementation should be changed to match documentation.
