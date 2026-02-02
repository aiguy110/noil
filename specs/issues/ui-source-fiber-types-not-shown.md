# Issue: UI Not Showing `source` Fiber Types

**Status**: Open
**Priority**: Medium
**Component**: Web UI / Frontend
**Date Reported**: 2026-02-02

## Problem Description

The Noil web UI is not displaying fiber types that represent individual sources (e.g., per-source fibers that never close). These fiber types are commonly used to provide a "jumping-off point" for navigation, grouping all logs from a single source into one fiber.

## Expected Behavior

The UI should display all fiber types, including:
- Correlation fibers (e.g., `request_trace`, `simple_log`)
- Source-specific fibers that group all logs from a single source

Example of a source fiber type from the parent config:

```yaml
fiber_types:
  nginx_all:
    description: "All nginx logs as a single fiber"
    temporal:
      max_gap: infinite
    attributes:
      - name: source_marker
        type: string
        key: true
        derived: "nginx"
    sources:
      nginx_access:
        patterns:
          - regex: '.+'
```

## Actual Behavior

Source fiber types are not visible in the UI. Only correlation fiber types (those that group logs across sources based on extracted attributes) appear.

## Steps to Reproduce

1. Start noil with a config containing both correlation and source fiber types
2. Navigate to the web UI
3. Observe that source fiber types (e.g., `nginx_all`) are missing from the fiber type selector/list

## Impact

- Users cannot easily navigate to "all logs from source X"
- Reduces discoverability of the full dataset
- Makes it harder to use the UI as a log viewer for individual sources

## Suggested Fix

Investigation needed to determine why source fiber types are filtered out:
- Check frontend filtering logic that populates fiber type lists
- Verify API responses include all fiber types
- Ensure UI components render all returned fiber types

The API query `GET /api/fibers?type=nginx_all` (or equivalent source fiber type) should work if the backend is functioning correctly. The issue is likely in:
- Frontend filtering/display logic
- UI component that lists available fiber types

## Related Files

- `frontend/js/app.js` - Main frontend logic
- `frontend/index.html` - UI structure
- `src/web/api.rs` - API handlers

## Test Case

After fix, verify:
1. All fiber types from config appear in UI fiber type selector
2. Can query and display source fibers (e.g., "nginx_all") in UI
3. Can navigate from a log to its source fiber
