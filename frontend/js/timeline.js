/**
 * Timeline visualization for fibers
 */
class Timeline {
    constructor(containerEl, onFiberSelect, onFiberContextMenu) {
        this.container = containerEl;
        this.onFiberSelect = onFiberSelect;
        this.onFiberContextMenu = onFiberContextMenu || null;
        this.fibers = [];
        this.fiberTypeMetadata = [];
        this.isMetadataLoaded = false;
        this.selectedFiberId = null;
        this.selectedLogTimestamp = null;
        this.zoom = 1.0;
        this.minZoom = 0.1;
        this.maxZoom = 10.0;

        // Panning state
        this.isPanning = false;
        this.startX = 0;
        this.startY = 0;
        this.scrollLeft = 0;
        this.scrollTop = 0;

        // View mode: 'tangled' (lane-based) or 'straight' (current horizontal)
        this.viewMode = localStorage.getItem('noil_timeline_view_mode') || 'tangled';

        // Tangled view data structures
        this.lanes = [];           // [{sourceId, y, height, color}]
        this.fiberPaths = [];      // [{fiberId, fiberType, points: [{x, laneIndex}]}]
        this.membershipData = {};  // fiber_id -> [{timestamp, source_id}]

        this.loadFiberTypeMetadata();
        this.initPanning();
        this.initViewToggle();
        this.render();
    }

    initViewToggle() {
        const toggleContainer = document.querySelector('.timeline-view-toggle');
        if (!toggleContainer) return;

        const toggleInput = toggleContainer.querySelector('input[type="checkbox"]');
        if (!toggleInput) return;

        toggleInput.checked = this.viewMode === 'straight';

        toggleInput.addEventListener('change', () => {
            const newMode = toggleInput.checked ? 'straight' : 'tangled';
            if (newMode !== this.viewMode) {
                this.setViewMode(newMode);
            }
        });
    }

    setViewMode(mode) {
        this.viewMode = mode;
        localStorage.setItem('noil_timeline_view_mode', mode);
        this.render();
    }

    getViewMode() {
        return this.viewMode;
    }

    initPanning() {
        // Get the scrollable container (parent of timeline-canvas)
        const scrollContainer = this.container.parentElement;
        const axisEl = document.getElementById('timeline-axis');

        const onMouseDown = (e) => {
            // Only start panning if clicking on the background (not on a fiber)
            if (e.target.classList.contains('fiber-line') ||
                e.target.classList.contains('fiber-path') ||
                e.target.classList.contains('source-fiber-line')) {
                return;
            }

            this.isPanning = true;
            this.startX = e.clientX;
            this.startY = e.clientY;
            this.scrollLeft = scrollContainer.scrollLeft;
            this.scrollTop = scrollContainer.scrollTop;

            scrollContainer.style.cursor = 'grabbing';
            document.body.style.userSelect = 'none';
            e.preventDefault();
        };

        const onMouseMove = (e) => {
            if (!this.isPanning) return;

            e.preventDefault();
            const deltaX = e.clientX - this.startX;
            const deltaY = e.clientY - this.startY;

            scrollContainer.scrollLeft = this.scrollLeft - deltaX;
            scrollContainer.scrollTop = this.scrollTop - deltaY;
        };

        const onMouseUp = () => {
            if (this.isPanning) {
                this.isPanning = false;
                scrollContainer.style.cursor = 'grab';
                document.body.style.userSelect = '';
            }
        };

        // Attach mouse events
        scrollContainer.addEventListener('mousedown', onMouseDown);
        document.addEventListener('mousemove', onMouseMove);
        document.addEventListener('mouseup', onMouseUp);

        // Sync axis scroll with timeline scroll (timeline -> axis)
        scrollContainer.addEventListener('scroll', () => {
            if (axisEl && !this.isSyncingScroll) {
                this.isSyncingScroll = true;
                axisEl.scrollLeft = scrollContainer.scrollLeft;
                this.isSyncingScroll = false;
            }
        });

        // Sync timeline scroll with axis scroll (axis -> timeline)
        if (axisEl) {
            axisEl.addEventListener('scroll', () => {
                if (!this.isSyncingScroll) {
                    this.isSyncingScroll = true;
                    scrollContainer.scrollLeft = axisEl.scrollLeft;
                    this.isSyncingScroll = false;
                }
            });
        }

        // Mouse wheel zoom
        scrollContainer.addEventListener('wheel', (e) => {
            e.preventDefault();

            // Get mouse position relative to container
            const rect = scrollContainer.getBoundingClientRect();
            const mouseX = e.clientX - rect.left;
            const viewportWidth = scrollContainer.offsetWidth;

            // Calculate position in content under the cursor
            const oldPos = scrollContainer.scrollLeft + mouseX;

            // The fiber content starts after left padding (viewportWidth).
            // Only the fiber content area scales with zoom - padding stays constant.
            const fiberOffset = viewportWidth;
            const posInFiber = oldPos - fiberOffset;

            // Store old zoom before changing
            const oldZoom = this.zoom;

            // Zoom in or out based on wheel direction
            if (e.deltaY < 0) {
                this.zoomIn();
            } else {
                this.zoomOut();
            }

            // Adjust scroll position to keep the same time under the cursor
            requestAnimationFrame(() => {
                // Scale only the fiber content position, not the padding
                const newPosInFiber = posInFiber * (this.zoom / oldZoom);
                const newPos = fiberOffset + newPosInFiber;
                const newScrollLeft = newPos - mouseX;
                scrollContainer.scrollLeft = Math.max(0, newScrollLeft);
            });
        }, { passive: false });

        // Set initial cursor style
        scrollContainer.style.cursor = 'grab';
        this.isSyncingScroll = false;
    }

    async loadFiberTypeMetadata() {
        try {
            this.fiberTypeMetadata = await api.getAllFiberTypes();
            this.isMetadataLoaded = true;
            this.renderLegend();
            this.render();
        } catch (error) {
            console.error('Failed to load fiber type metadata:', error);
            this.fiberTypeMetadata = [];
            this.isMetadataLoaded = false;
        }
    }

    renderLegend() {
        const legendEl = document.getElementById('timeline-legend');
        if (!legendEl || this.fiberTypeMetadata.length === 0) return;

        legendEl.innerHTML = '';

        // Get unique fiber types from current fibers or all fiber types from metadata
        const fiberTypes = this.fibers.length > 0
            ? [...new Set(this.fibers.map(f => f.fiber_type))]
            : this.fiberTypeMetadata.map(ft => ft.name);

        fiberTypes.forEach(fiberType => {
            const color = colorManager.getFiberTypeColor(fiberType);

            const item = document.createElement('div');
            item.className = 'legend-item';

            const colorBox = document.createElement('div');
            colorBox.className = 'legend-color';
            colorBox.style.backgroundColor = color;

            const label = document.createElement('span');
            label.textContent = fiberType;

            item.appendChild(colorBox);
            item.appendChild(label);
            legendEl.appendChild(item);
        });
    }

    setFibers(fibers) {
        const isFirstLoad = this.fibers.length === 0 && fibers.length > 0;
        this.fibers = this.sortFibers(fibers);
        this.membershipData = {}; // Clear membership data when using setFibers
        this.renderLegend();
        this.render();

        // On first load, scroll to show fiber content (past the left padding)
        if (isFirstLoad) {
            const scrollContainer = this.container.parentElement;
            const viewportWidth = scrollContainer.offsetWidth || 800;
            scrollContainer.scrollLeft = viewportWidth;
        }
    }

    /**
     * Set fibers along with membership data for tangled view
     * @param {Array} fibers - Array of fiber objects
     * @param {Object} membershipData - Map of fiber_id -> [{timestamp, source_id}]
     */
    setFibersWithMembership(fibers, membershipData) {
        const isFirstLoad = this.fibers.length === 0 && fibers.length > 0;
        this.fibers = this.sortFibers(fibers);
        this.membershipData = membershipData || {};
        this.renderLegend();
        this.render();

        // On first load, scroll to show fiber content (past the left padding)
        if (isFirstLoad) {
            const scrollContainer = this.container.parentElement;
            const viewportWidth = scrollContainer.offsetWidth || 800;
            scrollContainer.scrollLeft = viewportWidth;
        }
    }

    /**
     * Get the visible time range based on current scroll position and zoom
     * @returns {{start: Date, end: Date}} The visible time range
     */
    getVisibleTimeRange() {
        if (this.fibers.length === 0) {
            return { start: new Date(), end: new Date() };
        }

        // Calculate time bounds from fibers
        let minTime = Infinity;
        let maxTime = -Infinity;

        this.fibers.forEach(fiber => {
            const start = new Date(fiber.first_activity).getTime();
            const end = new Date(fiber.last_activity).getTime();
            minTime = Math.min(minTime, start);
            maxTime = Math.max(maxTime, end);
        });

        const padding = (maxTime - minTime) * 0.05;
        minTime -= padding;
        maxTime += padding;

        const timeRange = maxTime - minTime;
        const scrollContainer = this.container.parentElement;
        const viewportWidth = scrollContainer.offsetWidth || 800;
        const fiberContentWidth = viewportWidth * this.zoom;
        const pixelsPerMs = fiberContentWidth / timeRange;
        const paddingWidth = viewportWidth;

        // Calculate visible time range based on scroll position
        const scrollLeft = scrollContainer.scrollLeft;
        const visibleStartPx = scrollLeft - paddingWidth;
        const visibleEndPx = visibleStartPx + viewportWidth;

        const visibleStartTime = minTime + (visibleStartPx / pixelsPerMs);
        const visibleEndTime = minTime + (visibleEndPx / pixelsPerMs);

        return {
            start: new Date(visibleStartTime),
            end: new Date(visibleEndTime)
        };
    }

    sortFibers(fibers) {
        // Build a map of fiber type name to is_source_fiber
        const sourceMap = new Map(
            this.fiberTypeMetadata.map(ft => [ft.name, ft.is_source_fiber])
        );

        // Separate source fibers from traced fibers
        const sourceFibers = fibers.filter(f => sourceMap.get(f.fiber_type) === true);
        const tracedFibers = fibers.filter(f => sourceMap.get(f.fiber_type) !== true);

        // Sort each group by fiber_type name
        sourceFibers.sort((a, b) => a.fiber_type.localeCompare(b.fiber_type));
        tracedFibers.sort((a, b) => a.fiber_type.localeCompare(b.fiber_type));

        // Return source fibers first, then traced
        return [...sourceFibers, ...tracedFibers];
    }

    selectFiber(fiberId) {
        this.selectedFiberId = fiberId;
        this.render();
        if (this.onFiberSelect) {
            this.onFiberSelect(fiberId);
        }
    }

    setSelectedLogTimestamp(timestamp) {
        this.selectedLogTimestamp = timestamp;
        this.render();
    }

    clearSelectedLogTimestamp() {
        this.selectedLogTimestamp = null;
        this.render();
    }

    zoomIn() {
        this.zoom = Math.min(this.maxZoom, this.zoom * 1.1);
        this.render();
    }

    zoomOut() {
        this.zoom = Math.max(this.minZoom, this.zoom / 1.1);
        this.render();
    }

    resetView() {
        // Reset zoom to 1.0
        this.zoom = 1.0;

        // Reset scroll position to show fiber content (accounting for left padding)
        const scrollContainer = this.container.parentElement;
        const viewportWidth = scrollContainer.offsetWidth || 800;
        // Scroll to start of fiber content (past the left padding)
        scrollContainer.scrollLeft = viewportWidth;
        scrollContainer.scrollTop = 0;

        // Re-render with reset view
        this.render();
    }

    render() {
        if (!this.isMetadataLoaded) {
            return;
        }

        if (this.fibers.length === 0) {
            this.container.innerHTML = `
                <div class="empty-state">
                    <div class="empty-state-icon">📊</div>
                    <div class="empty-state-text">No fibers to display</div>
                </div>
            `;
            return;
        }

        if (this.viewMode === 'tangled') {
            this.renderTangledView();
        } else {
            this.renderStraightView();
        }
    }

    /**
     * Calculate common time/dimension values used by both render modes
     */
    calculateRenderParams() {
        let minTime = Infinity;
        let maxTime = -Infinity;

        this.fibers.forEach(fiber => {
            const start = new Date(fiber.first_activity).getTime();
            const end = new Date(fiber.last_activity).getTime();
            minTime = Math.min(minTime, start);
            maxTime = Math.max(maxTime, end);
        });

        const padding = (maxTime - minTime) * 0.05;
        minTime -= padding;
        maxTime += padding;

        const timeRange = maxTime - minTime;
        const viewportWidth = this.container.parentElement.offsetWidth || 800;
        const fiberContentWidth = viewportWidth * this.zoom;
        const pixelsPerMs = fiberContentWidth / timeRange;
        const paddingWidth = viewportWidth;
        const effectiveWidth = fiberContentWidth + paddingWidth * 2;
        const fiberOffset = paddingWidth;

        return {
            minTime,
            maxTime,
            timeRange,
            viewportWidth,
            fiberContentWidth,
            pixelsPerMs,
            paddingWidth,
            effectiveWidth,
            fiberOffset
        };
    }

    renderStraightView() {
        const params = this.calculateRenderParams();
        const { minTime, maxTime, timeRange, pixelsPerMs, paddingWidth, effectiveWidth, fiberOffset } = params;

        // Clear container
        this.container.innerHTML = '';

        // Create fiber lines
        this.fibers.forEach((fiber, index) => {
            const start = new Date(fiber.first_activity).getTime();
            const end = new Date(fiber.last_activity).getTime();

            const left = fiberOffset + (start - minTime) * pixelsPerMs;
            const width = Math.max(4, (end - start) * pixelsPerMs); // Minimum 4px width

            const line = document.createElement('div');
            line.className = 'fiber-line';
            if (fiber.id === this.selectedFiberId) {
                line.classList.add('selected');
            }

            const color = colorManager.getFiberTypeColor(fiber.fiber_type);
            line.style.backgroundColor = color;
            line.style.position = 'absolute';
            line.style.left = `${left}px`;
            line.style.width = `${width}px`;
            line.style.top = `${index * 14}px`;

            // Add tooltip
            const duration = end - start;
            const durationStr = this.formatDuration(duration);
            const isSourceFiber = this.fiberTypeMetadata.find(ft => ft.name === fiber.fiber_type)?.is_source_fiber;
            const fiberTypeLabel = isSourceFiber ? 'source' : 'traced';
            line.title = `${fiber.fiber_type}\nType: ${fiberTypeLabel}\nID: ${fiber.id}\nDuration: ${durationStr}\n${fiber.closed ? 'Closed' : 'Open'}`;

            line.addEventListener('click', () => {
                this.selectFiber(fiber.id);
            });

            // Add right-click context menu
            if (this.onFiberContextMenu) {
                line.addEventListener('contextmenu', (e) => {
                    e.preventDefault();
                    this.onFiberContextMenu(e, fiber.id);
                });
            }

            this.container.appendChild(line);
        });

        // Render timestamp indicator if log is selected
        this.renderTimestampIndicator(minTime, maxTime, pixelsPerMs, fiberOffset);

        // Set container height based on number of fibers
        this.container.style.height = `${this.fibers.length * 14 + 20}px`;
        // Set container width to match the effective width used for positioning
        this.container.style.minWidth = `${effectiveWidth}px`;

        // Calculate visible time range (what's visible in the viewport at current zoom)
        const visibleTimeRange = timeRange / this.zoom;

        // Update time axis
        this.renderTimeAxis(minTime, maxTime, effectiveWidth, pixelsPerMs, visibleTimeRange, fiberOffset, paddingWidth);
    }

    renderTangledView() {
        const params = this.calculateRenderParams();
        const { minTime, maxTime, timeRange, pixelsPerMs, paddingWidth, effectiveWidth, fiberOffset } = params;

        // Clear container
        this.container.innerHTML = '';

        // Separate source fibers from traced fibers
        const sourceMap = new Map(
            this.fiberTypeMetadata.map(ft => [ft.name, ft.is_source_fiber])
        );
        const sourceFibers = this.fibers.filter(f => sourceMap.get(f.fiber_type) === true);
        const tracedFibers = this.fibers.filter(f => sourceMap.get(f.fiber_type) !== true);

        // Compute lanes from source fibers
        const sourceLaneTypes = this.fiberTypeMetadata
            .filter(ft => ft.is_source_fiber)
            .map(ft => ft.name);
        this.lanes = this.computeLanesFromTypes(sourceLaneTypes);

        // Build source ID to lane index map
        const sourceToLane = new Map(this.lanes.map((lane, idx) => [lane.sourceId, idx]));

        // Compute track assignments and dynamic lane sizing
        const trackLayout = this.computeLaneTracks(tracedFibers, sourceToLane);
        const laneTrackCounts = trackLayout.laneTrackCounts;

        const trackSpacing = 12;
        const lanePaddingTop = 14;
        const lanePaddingBottom = 10;
        const minLaneHeight = 40;

        const laneLayout = new Map();
        let currentTop = 0;
        this.lanes.forEach(lane => {
            const trackCount = Math.max(1, laneTrackCounts.get(lane.sourceId) || 1);
            const height = Math.max(
                minLaneHeight,
                lanePaddingTop + lanePaddingBottom + trackCount * trackSpacing
            );
            laneLayout.set(lane.sourceId, { top: currentTop, height });
            lane.top = currentTop;
            lane.height = height;
            currentTop += height;
        });

        const totalHeight = Math.max(currentTop + 20, 100);

        // Create lane backgrounds
        this.lanes.forEach(lane => {
            const laneEl = document.createElement('div');
            laneEl.className = 'timeline-lane';
            laneEl.style.top = `${lane.top}px`;
            laneEl.style.height = `${lane.height}px`;
            laneEl.style.backgroundColor = this.colorWithAlpha(lane.color, 0.1);

            // Add lane label
            const labelEl = document.createElement('div');
            labelEl.className = 'timeline-lane-label';
            labelEl.textContent = lane.sourceId;
            labelEl.style.backgroundColor = this.colorWithAlpha(lane.color, 0.8);
            laneEl.appendChild(labelEl);

            this.container.appendChild(laneEl);
        });

        // Create SVG layers for traced fiber paths
        const svgBack = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
        svgBack.setAttribute('class', 'timeline-svg timeline-svg-back');
        svgBack.style.width = `${effectiveWidth}px`;
        svgBack.style.height = `${totalHeight}px`;
        this.container.appendChild(svgBack);

        const svgFront = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
        svgFront.setAttribute('class', 'timeline-svg timeline-svg-front');
        svgFront.style.width = `${effectiveWidth}px`;
        svgFront.style.height = `${totalHeight}px`;

        // Render traced fiber paths
        tracedFibers.forEach(fiber => {
            const membershipPoints = this.membershipData[fiber.id] || [];
            if (membershipPoints.length === 0) {
                // Fall back to simple line if no membership data
                this.renderTracedFiberSimple(
                    svgBack,
                    fiber,
                    minTime,
                    pixelsPerMs,
                    fiberOffset,
                    totalHeight,
                    svgFront
                );
            } else {
                this.renderTracedFiberPath(
                    svgBack,
                    fiber,
                    membershipPoints,
                    minTime,
                    pixelsPerMs,
                    fiberOffset,
                    laneLayout,
                    trackLayout.fiberSegments,
                    { trackSpacing, lanePaddingTop },
                    svgFront
                );
            }
        });

        // Render source fiber lines on top of their lanes
        sourceFibers.forEach(fiber => {
            const layout = laneLayout.get(fiber.fiber_type);
            if (!layout) return;

            const start = new Date(fiber.first_activity).getTime();
            const end = new Date(fiber.last_activity).getTime();

            const left = fiberOffset + (start - minTime) * pixelsPerMs;
            const width = Math.max(4, (end - start) * pixelsPerMs);

            const line = document.createElement('div');
            line.className = 'source-fiber-line';
            if (fiber.id === this.selectedFiberId) {
                line.classList.add('selected');
            }

            const color = colorManager.getFiberTypeColor(fiber.fiber_type);
            line.style.backgroundColor = color;
            line.style.left = `${left}px`;
            line.style.width = `${width}px`;
            line.style.top = `${layout.top + 8}px`; // Position near top of lane

            // Add tooltip
            const duration = end - start;
            const durationStr = this.formatDuration(duration);
            line.title = `${fiber.fiber_type}\nType: source\nID: ${fiber.id}\nDuration: ${durationStr}\n${fiber.closed ? 'Closed' : 'Open'}`;

            line.addEventListener('click', () => {
                this.selectFiber(fiber.id);
            });

            if (this.onFiberContextMenu) {
                line.addEventListener('contextmenu', (e) => {
                    e.preventDefault();
                    this.onFiberContextMenu(e, fiber.id);
                });
            }

            this.container.appendChild(line);
        });

        this.container.appendChild(svgFront);

        // Render timestamp indicator if log is selected
        this.renderTimestampIndicator(minTime, maxTime, pixelsPerMs, fiberOffset);

        // Set container dimensions
        this.container.style.height = `${totalHeight}px`;
        this.container.style.minWidth = `${effectiveWidth}px`;

        // Update time axis
        const visibleTimeRange = timeRange / this.zoom;
        this.renderTimeAxis(minTime, maxTime, effectiveWidth, pixelsPerMs, visibleTimeRange, fiberOffset, paddingWidth);
    }

    /**
     * Compute lanes from source fibers - one lane per unique source fiber type
     */
    computeLanesFromTypes(sourceTypes) {
        const uniqueTypes = [...new Set(sourceTypes)].sort();
        return uniqueTypes.map(fiberType => ({
            sourceId: fiberType,
            color: colorManager.getFiberTypeColor(fiberType)
        }));
    }

    computeLaneTracks(tracedFibers, sourceToLane) {
        const laneSegments = new Map();

        const pushSegment = (laneId, fiberId, start, end) => {
            const safeStart = Number.isFinite(start) ? start : 0;
            const safeEnd = Number.isFinite(end) ? end : safeStart;
            const normalizedEnd = Math.max(safeStart, safeEnd);
            if (!laneSegments.has(laneId)) {
                laneSegments.set(laneId, []);
            }
            laneSegments.get(laneId).push({ fiberId, start: safeStart, end: normalizedEnd });
        };

        tracedFibers.forEach(fiber => {
            const membershipPoints = this.membershipData[fiber.id] || [];
            if (membershipPoints.length === 0) return;

            const sortedPoints = membershipPoints
                .map(point => ({
                    source_id: point.source_id,
                    timestamp: new Date(point.timestamp).getTime()
                }))
                .filter(point => sourceToLane.has(point.source_id))
                .sort((a, b) => a.timestamp - b.timestamp || String(a.source_id).localeCompare(String(b.source_id)));

            if (sortedPoints.length === 0) return;

            const fiberEnd = new Date(fiber.last_activity).getTime();
            let currentLane = sortedPoints[0].source_id;
            let segmentStart = sortedPoints[0].timestamp;

            for (let i = 1; i < sortedPoints.length; i++) {
                const point = sortedPoints[i];
                if (point.source_id === currentLane) {
                    continue;
                }

                pushSegment(currentLane, fiber.id, segmentStart, point.timestamp);
                currentLane = point.source_id;
                segmentStart = point.timestamp;
            }

            const finalEnd = Number.isFinite(fiberEnd) ? fiberEnd : segmentStart;
            pushSegment(currentLane, fiber.id, segmentStart, finalEnd);
        });

        const laneTrackCounts = new Map();
        const fiberSegments = new Map();

        laneSegments.forEach((segments, laneId) => {
            const sortedSegments = [...segments].sort((a, b) => {
                if (a.start !== b.start) return a.start - b.start;
                const aDuration = a.end - a.start;
                const bDuration = b.end - b.start;
                if (aDuration !== bDuration) return bDuration - aDuration;
                return String(a.fiberId).localeCompare(String(b.fiberId));
            });

            const trackEnds = [];
            sortedSegments.forEach(segment => {
                let assignedTrack = null;
                for (let i = 0; i < trackEnds.length; i++) {
                    if (segment.start > trackEnds[i]) {
                        assignedTrack = i;
                        break;
                    }
                }

                if (assignedTrack === null) {
                    assignedTrack = trackEnds.length;
                    trackEnds.push(segment.end);
                } else {
                    trackEnds[assignedTrack] = Math.max(trackEnds[assignedTrack], segment.end);
                }

                const assignedSegment = {
                    laneId,
                    fiberId: segment.fiberId,
                    start: segment.start,
                    end: segment.end,
                    track: assignedTrack
                };

                if (!fiberSegments.has(segment.fiberId)) {
                    fiberSegments.set(segment.fiberId, []);
                }
                fiberSegments.get(segment.fiberId).push(assignedSegment);
            });

            laneTrackCounts.set(laneId, trackEnds.length);
        });

        return { laneTrackCounts, fiberSegments };
    }

    getTrackForPoint(fiberSegments, fiberId, laneId, timestamp) {
        const segments = fiberSegments.get(fiberId) || [];
        for (const segment of segments) {
            if (segment.laneId !== laneId) continue;
            if (timestamp >= segment.start && timestamp <= segment.end) {
                return segment.track;
            }
        }
        return 0;
    }

    /**
     * Render a traced fiber as a simple horizontal line (fallback when no membership data)
     */
    renderTracedFiberSimple(svg, fiber, minTime, pixelsPerMs, fiberOffset, totalHeight, frontSvg) {
        const start = new Date(fiber.first_activity).getTime();
        const end = new Date(fiber.last_activity).getTime();

        const x1 = fiberOffset + (start - minTime) * pixelsPerMs;
        const x2 = fiberOffset + (end - minTime) * pixelsPerMs;
        const y = totalHeight / 2; // Middle of all lanes

        const path = document.createElementNS('http://www.w3.org/2000/svg', 'line');
        path.setAttribute('x1', x1);
        path.setAttribute('y1', y);
        path.setAttribute('x2', x2);
        path.setAttribute('y2', y);
        path.setAttribute('stroke', colorManager.getFiberTypeColor(fiber.fiber_type));
        path.setAttribute('class', 'fiber-path');
        if (fiber.id === this.selectedFiberId) {
            path.classList.add('selected');
        }

        path.addEventListener('click', () => {
            this.selectFiber(fiber.id);
        });

        if (this.onFiberContextMenu) {
            path.addEventListener('contextmenu', (e) => {
                e.preventDefault();
                this.onFiberContextMenu(e, fiber.id);
            });
        }

        this.attachFrontClone(path, frontSvg, fiber.id === this.selectedFiberId);

        // Add tooltip via title element
        const title = document.createElementNS('http://www.w3.org/2000/svg', 'title');
        const duration = end - start;
        const durationStr = this.formatDuration(duration);
        title.textContent = `${fiber.fiber_type}\nType: traced\nID: ${fiber.id}\nDuration: ${durationStr}\n${fiber.closed ? 'Closed' : 'Open'}`;
        path.appendChild(title);

        svg.appendChild(path);
    }

    /**
     * Render a traced fiber as a path that weaves between lanes based on membership data
     */
    renderTracedFiberPath(svg, fiber, membershipPoints, minTime, pixelsPerMs, fiberOffset, laneLayout, fiberSegments, trackConfig, frontSvg) {
        if (membershipPoints.length === 0) return;

        const color = colorManager.getFiberTypeColor(fiber.fiber_type);
        const { trackSpacing, lanePaddingTop } = trackConfig;

        const sortedPoints = [...membershipPoints]
            .map(point => ({
                source_id: point.source_id,
                timestamp: new Date(point.timestamp).getTime()
            }))
            .sort((a, b) => a.timestamp - b.timestamp);

        // Build path points
        const pathPoints = [];
        sortedPoints.forEach(point => {
            const layout = laneLayout.get(point.source_id);
            if (!layout) return;

            const x = fiberOffset + (point.timestamp - minTime) * pixelsPerMs;
            const track = this.getTrackForPoint(fiberSegments, fiber.id, point.source_id, point.timestamp);
            const y = layout.top + lanePaddingTop + trackSpacing * track + trackSpacing / 2;
            pathPoints.push({ x, y });
        });

        if (pathPoints.length === 0) return;

        // Create SVG path with smooth curves
        let d = `M ${pathPoints[0].x} ${pathPoints[0].y}`;
        for (let i = 1; i < pathPoints.length; i++) {
            const prev = pathPoints[i - 1];
            const curr = pathPoints[i];
            // Use quadratic bezier for smooth transitions
            const midX = (prev.x + curr.x) / 2;
            d += ` Q ${midX} ${prev.y} ${midX} ${(prev.y + curr.y) / 2}`;
            d += ` Q ${midX} ${curr.y} ${curr.x} ${curr.y}`;
        }

        const path = document.createElementNS('http://www.w3.org/2000/svg', 'path');
        path.setAttribute('d', d);
        path.setAttribute('stroke', color);
        path.setAttribute('class', 'fiber-path');
        if (fiber.id === this.selectedFiberId) {
            path.classList.add('selected');
        }

        path.addEventListener('click', () => {
            this.selectFiber(fiber.id);
        });

        if (this.onFiberContextMenu) {
            path.addEventListener('contextmenu', (e) => {
                e.preventDefault();
                this.onFiberContextMenu(e, fiber.id);
            });
        }

        this.attachFrontClone(path, frontSvg, fiber.id === this.selectedFiberId);

        // Add tooltip
        const title = document.createElementNS('http://www.w3.org/2000/svg', 'title');
        const start = new Date(fiber.first_activity).getTime();
        const end = new Date(fiber.last_activity).getTime();
        const duration = end - start;
        const durationStr = this.formatDuration(duration);
        title.textContent = `${fiber.fiber_type}\nType: traced\nID: ${fiber.id}\nDuration: ${durationStr}\n${fiber.closed ? 'Closed' : 'Open'}`;
        path.appendChild(title);

        svg.appendChild(path);
    }

    attachFrontClone(path, frontSvg, keep) {
        if (!frontSvg) return;

        const createClone = () => {
            if (path.__frontClone) return;
            const clone = path.cloneNode(true);
            clone.classList.add('fiber-path-front');
            clone.classList.remove('selected');
            if (keep) {
                clone.classList.add('selected');
            }
            path.__frontClone = clone;
            frontSvg.appendChild(clone);
        };

        const removeClone = () => {
            if (!path.__frontClone) return;
            path.__frontClone.remove();
            path.__frontClone = null;
        };

        if (keep) {
            createClone();
        }

        path.addEventListener('mouseenter', () => {
            createClone();
        });

        path.addEventListener('mouseleave', () => {
            if (!keep) {
                removeClone();
            }
        });
    }

    /**
     * Render the timestamp indicator (vertical line) for the selected log
     */
    renderTimestampIndicator(minTime, maxTime, pixelsPerMs, fiberOffset) {
        if (!this.selectedLogTimestamp) return;

        const timestamp = new Date(this.selectedLogTimestamp).getTime();
        if (timestamp >= minTime && timestamp <= maxTime) {
            const indicatorLeft = fiberOffset + (timestamp - minTime) * pixelsPerMs;

            const indicator = document.createElement('div');
            indicator.className = 'timeline-log-indicator';
            indicator.style.position = 'absolute';
            indicator.style.left = `${indicatorLeft}px`;
            indicator.style.top = '0';
            indicator.style.bottom = '0';
            indicator.style.width = '2px';
            indicator.style.backgroundColor = 'var(--accent-color)';
            indicator.style.zIndex = '30';
            indicator.style.pointerEvents = 'none';

            this.container.appendChild(indicator);
        }
    }

    /**
     * Convert hex color to rgba
     */
    colorWithAlpha(color, alpha) {
        if (!color) return color;

        const trimmed = color.trim();
        if (trimmed.startsWith('#')) {
            if (trimmed.length !== 7) return trimmed;
            const r = parseInt(trimmed.slice(1, 3), 16);
            const g = parseInt(trimmed.slice(3, 5), 16);
            const b = parseInt(trimmed.slice(5, 7), 16);
            return `rgba(${r}, ${g}, ${b}, ${alpha})`;
        }

        const hslMatch = trimmed.match(/hsla?\(([^)]+)\)/i);
        if (hslMatch) {
            const parts = hslMatch[1].split(',').map(part => part.trim());
            if (parts.length >= 3) {
                const h = parts[0];
                const s = parts[1];
                const l = parts[2];
                return `hsla(${h}, ${s}, ${l}, ${alpha})`;
            }
        }

        const rgbMatch = trimmed.match(/rgba?\(([^)]+)\)/i);
        if (rgbMatch) {
            const parts = rgbMatch[1].split(',').map(part => part.trim());
            if (parts.length >= 3) {
                const r = parts[0];
                const g = parts[1];
                const b = parts[2];
                return `rgba(${r}, ${g}, ${b}, ${alpha})`;
            }
        }

        return trimmed;
    }

    renderTimeAxis(minTime, maxTime, effectiveWidth, pixelsPerMs, visibleTimeRange, fiberOffset, paddingWidth) {
        const axisEl = document.getElementById('timeline-axis');
        if (!axisEl) return;

        axisEl.innerHTML = '';

        // Make the axis content the same width as the timeline canvas
        const totalWidth = effectiveWidth;

        // Calculate extended time range that includes padding areas
        // paddingWidth pixels on each side represents extrapolated time before/after fibers
        const paddingTime = paddingWidth / pixelsPerMs;
        const extendedMinTime = minTime - paddingTime;
        const extendedMaxTime = maxTime + paddingTime;
        const extendedTimeRange = extendedMaxTime - extendedMinTime;

        // Calculate appropriate time intervals based on zoom
        const baseSpacing = 100; // pixels between labels
        const numLabels = Math.max(1, Math.floor(totalWidth / baseSpacing));
        const interval = extendedTimeRange / numLabels;

        // Create a wrapper for labels and hatch marks with explicit width
        const contentWrapper = document.createElement('div');
        contentWrapper.style.position = 'relative';
        contentWrapper.style.width = `${totalWidth}px`;
        contentWrapper.style.minWidth = `${totalWidth}px`;
        contentWrapper.style.height = '100%';

        for (let i = 0; i <= numLabels; i++) {
            const time = extendedMinTime + (interval * i);
            // Position from left edge of canvas (extendedMinTime maps to position 0)
            const position = (time - extendedMinTime) * pixelsPerMs;

            // Create hatch mark
            const hatch = document.createElement('div');
            hatch.className = 'time-hatch';
            hatch.style.position = 'absolute';
            hatch.style.left = `${position + 20}px`; // +20 for padding
            hatch.style.top = '0';
            hatch.style.width = '1px';
            hatch.style.height = '6px';
            hatch.style.backgroundColor = '#666';

            // Create label
            const label = document.createElement('span');
            label.className = 'time-label';
            label.textContent = this.formatTime(new Date(time), visibleTimeRange);
            label.style.left = `${position + 20}px`; // +20 for padding
            label.style.top = '8px'; // Position below hatch mark

            contentWrapper.appendChild(hatch);
            contentWrapper.appendChild(label);
        }

        axisEl.appendChild(contentWrapper);
    }

    formatTime(date, visibleTimeRange) {
        const hours = String(date.getHours()).padStart(2, '0');
        const minutes = String(date.getMinutes()).padStart(2, '0');
        const seconds = String(date.getSeconds()).padStart(2, '0');

        // If visible range is less than 10 seconds, show milliseconds
        if (visibleTimeRange < 10000) {
            const milliseconds = String(date.getMilliseconds()).padStart(3, '0');
            return `${hours}:${minutes}:${seconds}.${milliseconds}`;
        }

        // If visible range spans more than one day, show date
        if (visibleTimeRange > 86400000) {
            const year = date.getFullYear();
            const month = String(date.getMonth() + 1).padStart(2, '0');
            const day = String(date.getDate()).padStart(2, '0');
            return `${year}-${month}-${day} ${hours}:${minutes}:${seconds}`;
        }

        // Default: just show time
        return `${hours}:${minutes}:${seconds}`;
    }

    formatDuration(ms) {
        if (ms < 1000) {
            return `${ms}ms`;
        } else if (ms < 60000) {
            return `${(ms / 1000).toFixed(1)}s`;
        } else if (ms < 3600000) {
            return `${(ms / 60000).toFixed(1)}m`;
        } else {
            return `${(ms / 3600000).toFixed(1)}h`;
        }
    }
}
