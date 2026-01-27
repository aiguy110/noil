# Working Set Testing Feature - Implementation Documentation

## Overview

This document tracks the implementation of the Working Set Testing Feature for Noil. This feature allows users to select log lines from the main viewer, add them to a working set, and test fiber type rules against those logs to verify correlation behavior.

## Feature Summary

**Purpose**: Make fiber rule development more user-friendly by allowing users to:
1. Right-click log lines/fibers in the main viewer to add them to a working set
2. View selected lines in a test panel on the Fiber Rules page
3. Test fiber rules against the working set with visual feedback
4. See IoU-ranked results comparing expected vs actual fiber membership

**User Preferences**:
- **Name**: "Working Set" for the collection of selected lines
- **Results display**: Modal overlay (focused, closeable for iteration)
- **Persistence**: Single global working set, persisted via localStorage (survives refresh)

---

## Implementation Status

### ✅ COMPLETED: Phase 1 - Frontend Context Menu & Working Set State

**Files Modified**:
- `frontend/js/app.js`
- `frontend/js/logviewer.js`
- `frontend/css/style.css`

**Implemented Features**:

1. **Working Set State Management** (`app.js`)
   - Added `workingSet` property to `NoilApp` class
   - Structure: `{ logIds: [], logs: {}, timestamp: string }`
   - Methods implemented:
     - `loadWorkingSetFromStorage()` - Loads from localStorage on app init
     - `saveWorkingSetToStorage()` - Persists to localStorage on changes
     - `addToWorkingSet(logId)` - Adds log to working set
     - `removeFromWorkingSet(logId)` - Removes log from working set
     - `clearWorkingSet()` - Clears all logs from working set
     - `getWorkingSet()` - Returns current working set
     - `isInWorkingSet(logId)` - Check if log is in working set

2. **Context Menu** (`app.js`)
   - Right-click handler on `.log-line` elements
   - Menu items:
     - Add to Working Set (shown when not in set)
     - Remove from Working Set (shown when in set)
     - Separator
     - Copy Log ID
     - Copy Timestamp
   - Smart positioning with boundary detection
   - Click-outside to dismiss
   - Z-index: 500 (same as hamburger dropdown)

3. **Visual Indicators** (`app.js`, `logviewer.js`, `style.css`)
   - Star icon (★) displayed on log lines in working set
   - Position: Top-right corner of log line
   - Color: `var(--accent-color)` with glow effect
   - Pulse animation for visibility
   - Class: `.in-working-set` added to log lines
   - Auto-updates when logs are added/removed

4. **CSS Styles** (`style.css`)
   ```css
   .context-menu { ... }
   .context-menu-item { ... }
   .working-set-indicator { ... }
   @keyframes starPulse { ... }
   ```

5. **Integration Points**
   - Context menu initialized in `app.init()`
   - Visual indicators updated after log viewer renders
   - Working set survives page refresh (localStorage)
   - Working set is global across all fiber types

---

### ✅ COMPLETED: Phase 2 - Working Set Panel UI

**Files Modified**:
- `frontend/index.html`
- `frontend/css/style.css`
- `frontend/js/fiber-processing.js`
- `frontend/js/app.js`

**Implemented Features**:

1. **3-Column Layout** (`index.html`, `style.css`)
   - Fiber Rules page restructured from 2-column to 3-column
   - Layout: Sidebar (220px) | Editor (40%) | Working Set Panel (35%)
   - Responsive: All columns are flex containers
   - Border separation between columns

2. **Working Set Panel HTML** (`index.html`)
   ```html
   <div class="working-set-panel">
     <div class="working-set-header">
       <h4>Working Set (<span id="working-set-count">0</span>)</h4>
       <div class="working-set-actions">
         <button id="working-set-clear">Clear</button>
         <button id="working-set-test">Test Rules</button>
       </div>
     </div>
     <div id="working-set-content" class="working-set-content">
       <!-- Logs displayed here -->
     </div>
   </div>
   ```

3. **Working Set Panel Functionality** (`fiber-processing.js`)
   - `initWorkingSetPanel()` - Initializes event listeners
   - `renderWorkingSetPanel()` - Renders current working set
   - `createWorkingSetLogItem(log, fibers)` - Creates compact log display
   - Features:
     - Dynamic count display in header
     - Empty state message with instructions
     - Compact log line display (12px font, 6px padding)
     - Timestamp + source badge
     - Truncated text (50 chars + ellipsis)
     - Full text in tooltip
     - Fiber type badges showing current memberships
     - Remove button (×) on hover
     - Clear button clears entire working set
     - Test button triggers test workflow

4. **CSS Styles** (`style.css`)
   - `.working-set-panel` - Panel container
   - `.working-set-header` - Header with count and buttons
   - `.working-set-content` - Scrollable content area
   - `.working-set-log-item` - Compact log display
   - `.working-set-fiber-badge` - Fiber type badges
   - `.working-set-log-remove` - Remove button (appears on hover)
   - `.working-set-empty` - Empty state message

5. **Integration**
   - Panel initialized when Fiber Rules page is first opened
   - Auto-updates when working set changes
   - Same working set visible regardless of selected fiber type
   - Test button tests currently selected fiber type against global working set
   - Fetches log details and fiber memberships on-demand (lazy loading)

---

### ✅ COMPLETED: Phase 4 - Test Modal & API Client (Frontend)

**Files Modified**:
- `frontend/index.html`
- `frontend/css/style.css`
- `frontend/js/fiber-processing.js`
- `frontend/js/api.js`

**Implemented Features**:

1. **Test Results Modal HTML** (`index.html`)
   ```html
   <div id="test-results-modal" class="modal">
     <div class="modal-content test-results-modal-content">
       <div class="modal-header">
         <h2>Test Results</h2>
         <button id="close-test-results-modal">×</button>
       </div>
       <div class="modal-body test-results-body">
         <div id="test-results-content">
           <!-- Results populated dynamically -->
         </div>
       </div>
     </div>
   </div>
   ```

2. **Test Workflow** (`fiber-processing.js`)
   - `testWorkingSet()` - Main test method
     - Validates fiber type is selected
     - Validates working set is not empty
     - Validates YAML syntax
     - Calls backend API
     - Shows results modal
     - Handles errors gracefully

3. **Test Results Display** (`fiber-processing.js`)
   - `showTestResults(result)` - Renders results in modal
   - Status header with icon and summary:
     - ✓ Perfect Match (IoU = 1.0) - Green
     - ⚠ Partial Match (0 < IoU < 1.0) - Yellow/Orange
     - ✗ No Match (IoU = 0) - Red
   - Expected Logs section:
     - Shows all logs in working set
     - Indicates which are in best match fiber (✓)
     - Indicates which are missing (✗)
   - Best Match Fiber section:
     - Shows all logs in best matching fiber
     - Indicates matches (✓)
     - Indicates extras (+ Extra)
   - All Fibers section:
     - Lists all generated fibers
     - Sorted by IoU (descending)
     - Highlights best match
     - Shows stats for each fiber

4. **API Client Method** (`api.js`)
   ```javascript
   async testWorkingSet(fiberTypeName, logIds, yamlContent) {
     return this.request(`/api/fiber-types/${encodeURIComponent(fiberTypeName)}/test-working-set`, {
       method: 'POST',
       body: JSON.stringify({
         log_ids: logIds,
         yaml_content: yamlContent,
         include_margin: true
       }),
     });
   }
   ```

5. **CSS Styles** (`style.css`)
   - `.test-results-modal-content` - Modal sizing (1200px wide, 85vh max height)
   - `.test-result-status-*` - Status header variants (success/warning/error)
   - `.test-result-section` - Section containers
   - `.test-log-item` - Log display in results
   - `.match-success` / `.match-missing` / `.match-extra` - Color coding
   - `.test-fiber-item` - Fiber list items
   - `.best-match` - Highlight for best matching fiber

6. **Error Handling**
   - Graceful fallback when backend not implemented
   - Shows user-friendly message: "Backend API not yet implemented"
   - Logs error details to console
   - Validates YAML before sending to backend
   - Validates working set is not empty

---

## ✅ COMPLETED: Phase 3 - Backend Test Endpoint

**Files Modified**:
- `src/web/api.rs` - Added test endpoint handler and types
- `src/web/server.rs` - Added route

**Implemented Features**:

1. **Request/Response Types** (`src/web/api.rs`)
   - `TestWorkingSetRequest` - Request payload with log_ids, yaml_content, include_margin
   - `TestWorkingSetResponse` - Response with expected_logs, time_window, fibers_generated, best_match_index
   - `FiberMatchResult` - Individual fiber result with IoU, matching/missing/extra logs
   - `TimeWindowDto` - Time window start/end

2. **Handler Function** (`src/web/api.rs`)
   - `test_working_set()` - Main handler that:
     - Validates request (non-empty log_ids)
     - Queries logs by IDs from storage
     - Calculates time window: [min_timestamp - max_gap, max_timestamp + max_gap]
     - Parses fiber type YAML and validates fiber type name matches path parameter
     - Creates temporary Config with only the test fiber type
     - Creates temporary FiberProcessor
     - Queries all logs in time window (up to 10,000 logs)
     - Processes logs through temporary processor
     - Collects fiber memberships
     - Computes IoU for each generated fiber
     - Sorts results by IoU descending
     - Returns response with best match indicated

3. **IoU Calculation** (`src/web/api.rs`)
   - `calculate_iou()` - Helper function that computes Intersection over Union
   - Formula: |intersection| / |union|
   - Returns 0.0 if union is empty

4. **Route Registration** (`src/web/server.rs`)
   - POST `/api/fiber-types/:name/test-working-set`
   - Added import for `test_working_set` handler

5. **Error Handling**
   - 400 Bad Request: Empty log_ids, invalid YAML, fiber type name mismatch
   - 404 Not Found: Log IDs don't exist
   - 500 Internal Server Error: Processing failed

**Backend API Specification**:

### Endpoint
```
POST /api/fiber-types/:name/test-working-set
```

### Request Body
```json
{
  "log_ids": ["uuid1", "uuid2", "uuid3"],
  "yaml_content": "fiber_type_name:\n  description: ...",
  "include_margin": true
}
```

### Response Body
```json
{
  "expected_logs": [
    {
      "id": "uuid1",
      "timestamp": "2025-01-26T10:23:45.123Z",
      "source_id": "nginx",
      "raw_text": "GET /api/users"
    },
    {
      "id": "uuid2",
      "timestamp": "2025-01-26T10:23:46.001Z",
      "source_id": "app",
      "raw_text": "Processing user request"
    }
  ],
  "time_window": {
    "start": "2025-01-26T10:20:00Z",
    "end": "2025-01-26T10:30:00Z"
  },
  "fibers_generated": [
    {
      "fiber_id": "abc-123-def",
      "iou": 0.67,
      "matching_logs": ["uuid1", "uuid2"],
      "missing_logs": ["uuid3"],
      "extra_log_ids": ["uuid4", "uuid5"],
      "logs": [
        /* full log objects for all logs in this fiber */
      ]
    }
  ],
  "best_match_index": 0
}
```

### Implementation Steps

1. **Add Request/Response Types** (`src/web/api.rs`)
   ```rust
   #[derive(Deserialize)]
   pub struct TestWorkingSetRequest {
       pub log_ids: Vec<Uuid>,
       pub yaml_content: String,
       pub include_margin: Option<bool>,
   }

   #[derive(Serialize)]
   pub struct TestWorkingSetResponse {
       pub expected_logs: Vec<LogDto>,
       pub time_window: TimeWindow,
       pub fibers_generated: Vec<FiberMatchResult>,
       pub best_match_index: Option<usize>,
   }

   #[derive(Serialize)]
   pub struct FiberMatchResult {
       pub fiber_id: Uuid,
       pub iou: f64,
       pub matching_logs: Vec<Uuid>,
       pub missing_logs: Vec<Uuid>,
       pub extra_log_ids: Vec<Uuid>,
       pub logs: Vec<LogDto>,
   }

   #[derive(Serialize)]
   pub struct TimeWindow {
       pub start: String,
       pub end: String,
   }
   ```

2. **Add Handler Function** (`src/web/api.rs`)
   ```rust
   pub async fn test_working_set(
       State(state): State<Arc<AppState>>,
       Path(fiber_type_name): Path<String>,
       Json(request): Json<TestWorkingSetRequest>,
   ) -> Result<Json<TestWorkingSetResponse>, ApiError> {
       // 1. Query logs by IDs
       // 2. Calculate time window: [min(timestamps) - max_gap, max(timestamps) + max_gap]
       // 3. Parse fiber type config from yaml_content
       // 4. Create temporary FiberProcessor with ONLY this fiber type
       // 5. Query all logs in time window
       // 6. Process logs through temporary processor
       // 7. Compute IoU for each generated fiber against expected log_ids
       // 8. Return results sorted by IoU (descending)
   }
   ```

3. **Add IoU Helper Function**
   ```rust
   fn calculate_iou(expected: &HashSet<Uuid>, actual: &HashSet<Uuid>) -> f64 {
       let intersection = expected.intersection(actual).count();
       let union = expected.union(actual).count();
       if union == 0 { 0.0 } else { intersection as f64 / union as f64 }
   }
   ```

4. **Add Route** (`src/web/server.rs`)
   ```rust
   .route(
       "/api/fiber-types/:name/test-working-set",
       post(api::test_working_set)
   )
   ```

5. **Error Handling**
   - 400 Bad Request: Invalid YAML, fiber type name mismatch
   - 404 Not Found: One or more log_ids don't exist
   - 500 Internal Server Error: Processing failed

---

## Testing Checklist

### ✅ Phase 1 Tests (Completed)
- [x] Right-click on log line shows context menu
- [x] Menu positioned correctly (doesn't go off-screen)
- [x] "Add to Working Set" adds log
- [x] "Remove from Working Set" removes log
- [x] Star icon appears on selected lines
- [x] Click outside closes menu
- [x] Copy Log ID works
- [x] Copy Timestamp works
- [x] Working set persists across page refresh
- [x] Star indicators update correctly

### ✅ Phase 2 Tests (Completed)
- [x] 3-column layout displays correctly on Fiber Rules page
- [x] Working set count updates correctly
- [x] Empty state shows when no logs selected
- [x] Compact log lines display with timestamp, source, text
- [x] Fiber type badges display correctly
- [x] Clear button removes all logs
- [x] Remove button (×) removes individual logs
- [x] Same working set visible across all fiber types
- [x] Panel updates when logs added/removed from main view

### ✅ Phase 3 Tests (Backend - Ready for Testing)
- [ ] Backend endpoint returns 200 OK with valid request
- [ ] Time window calculated correctly with max_gap margin
- [ ] Temporary fiber processor created successfully
- [ ] Logs in time window queried correctly
- [ ] Fiber processing produces expected results
- [ ] IoU calculated correctly
- [ ] Results sorted by IoU descending
- [ ] Error handling for invalid YAML
- [ ] Error handling for missing logs
- [ ] Error handling for fiber type name mismatch
- [ ] Error handling for processing failures

### ⏳ Phase 4 Tests (Integration - Ready for Testing)
- [ ] Test button disabled when working set empty
- [ ] Test button disabled when no fiber type selected
- [ ] Loading indicator shows during processing
- [ ] Results modal opens with correct data
- [ ] Perfect match shows green success status
- [ ] Partial match shows warning with details
- [ ] No match shows error state
- [ ] Expected logs panel shows match indicators
- [ ] Best match fiber panel shows extra logs
- [ ] All fibers list sorted by IoU
- [ ] Close button closes modal
- [ ] ESC key closes modal
- [ ] Click outside closes modal

---

## Edge Cases & Error Handling

### Implemented (Frontend)
- ✅ Empty working set - Test button shows error message
- ✅ No fiber type selected - Test button shows error message
- ✅ Invalid YAML - Validation before sending to backend
- ✅ localStorage full - Graceful fallback with warning
- ✅ Corrupted localStorage - Clear and start fresh
- ✅ Deleted logs - Remove from working set when API returns 404
- ✅ Backend not implemented - Friendly message shown

### Implemented (Backend)
- ✅ All logs outside time window - Returns empty fiber results
- ✅ Max_gap = infinite - Uses 1 hour default for time window calculation
- ✅ No fibers generated - Returns empty array
- ✅ Fiber type name mismatch in YAML - Returns 400 Bad Request
- ⏳ Very old working set (>7 days) - Could show warning in frontend

---

## Technical Details

### Working Set Data Structure

**In Memory** (app.js):
```javascript
{
  logIds: ["uuid1", "uuid2", "uuid3"],  // Array of UUIDs
  logs: {                                // Map of UUID -> LogDto (lazy loaded)
    "uuid1": { id: "uuid1", timestamp: "...", ... },
    "uuid2": { id: "uuid2", timestamp: "...", ... }
  },
  timestamp: "2025-01-26T12:34:56.789Z"  // Last modified
}
```

**In localStorage** (persisted):
```json
{
  "logIds": ["uuid1", "uuid2", "uuid3"],
  "timestamp": "2025-01-26T12:34:56.789Z"
}
```

Note: Full log objects are NOT persisted to avoid stale data. They are lazy-loaded from API when needed.

### IoU (Intersection over Union) Calculation

```
IoU = |intersection| / |union|
    = matching_logs / (expected_logs + fiber_logs - matching_logs)
```

**Examples**:
- Perfect match: IoU = 1.0 (all expected logs in fiber, no extras)
- Partial match: 0 < IoU < 1.0 (some overlap)
- No match: IoU = 0 (no overlap)

### Time Window Calculation

```
start = min(log_timestamps) - max_gap
end = max(log_timestamps) + max_gap
```

This ensures all logs that could potentially be part of the same fiber are included in the test.

---

## Future Enhancements (Not in MVP)

1. **Multiple Working Sets**
   - Allow users to create and save multiple named working sets
   - Switch between working sets
   - Persist to database instead of localStorage

2. **Working Set Import/Export**
   - Export working set as JSON
   - Import working set from file
   - Share working sets between team members

3. **Advanced Filtering**
   - Filter working set by source, time range, or attributes
   - Search within working set

4. **Comparison Mode**
   - Compare results of multiple fiber types against same working set
   - Side-by-side comparison view

5. **Historical Tests**
   - Save test results for later comparison
   - Track improvements over time
   - Regression detection

6. **Interactive Refinement**
   - Click on logs in results to see which pattern matched
   - Edit YAML directly from results modal
   - Suggest regex improvements based on test results

---

## Files Modified Summary

### Frontend (Completed)
- ✅ `frontend/js/app.js` - Working set state, context menu, indicators
- ✅ `frontend/js/logviewer.js` - Visual indicator updates
- ✅ `frontend/js/fiber-processing.js` - Working set panel, test logic, results modal
- ✅ `frontend/js/api.js` - Test endpoint method
- ✅ `frontend/css/style.css` - All styles for context menu, panel, modal
- ✅ `frontend/index.html` - Panel and modal markup

### Backend (Completed)
- ✅ `src/web/api.rs` - Test endpoint handler and types
- ✅ `src/web/server.rs` - Route registration

---

## Timeline

- **Phase 1 (Frontend Context Menu)**: ✅ Completed
- **Phase 2 (Frontend Working Set Panel)**: ✅ Completed
- **Phase 3 (Backend Test Endpoint)**: ✅ Completed
- **Phase 4 (Integration Testing)**: ⏳ Ready for Testing
- **Phase 5 (Polish)**: ⏳ Pending

---

## Notes for Backend Implementation

### Key Considerations

1. **Temporary Fiber Processor**
   - Create isolated processor with only the test fiber type
   - Don't interfere with main fiber processing
   - Use same logic as hot-reload for consistency

2. **Time Window**
   - Parse `max_gap` from fiber type config
   - Handle `infinite` max_gap (use reasonable default like 1 hour)
   - Include margin to catch logs that might be close to window boundaries

3. **Performance**
   - Limit time window to reasonable range (e.g., max 24 hours)
   - Limit number of logs in working set (e.g., max 100)
   - Use efficient queries with indexes

4. **Accuracy**
   - Use same fiber processing logic as production
   - Don't simplify or shortcut the logic for testing
   - Ensure IoU calculation is correct

5. **Error Messages**
   - Provide helpful error messages for common issues
   - Include line numbers for YAML parse errors
   - Suggest fixes when possible

---

## Contact & Support

For questions or issues with this implementation:
1. Check this document first
2. Review the code comments in modified files
3. Test with small working sets first (2-3 logs)
4. Check browser console for detailed error messages

## End of Document
