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

        this.loadFiberTypeMetadata();
        this.initPanning();
        this.render();
    }

    initPanning() {
        // Get the scrollable container (parent of timeline-canvas)
        const scrollContainer = this.container.parentElement;
        const axisEl = document.getElementById('timeline-axis');

        const onMouseDown = (e) => {
            // Only start panning if clicking on the background (not on a fiber)
            if (e.target.classList.contains('fiber-line')) {
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
            this.renderLegend();
        } catch (error) {
            console.error('Failed to load fiber type metadata:', error);
            this.fiberTypeMetadata = [];
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
        this.renderLegend();
        this.render();

        // On first load, scroll to show fiber content (past the left padding)
        if (isFirstLoad) {
            const scrollContainer = this.container.parentElement;
            const viewportWidth = scrollContainer.offsetWidth || 800;
            scrollContainer.scrollLeft = viewportWidth;
        }
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
        if (this.fibers.length === 0) {
            this.container.innerHTML = `
                <div class="empty-state">
                    <div class="empty-state-icon">📊</div>
                    <div class="empty-state-text">No fibers to display</div>
                </div>
            `;
            return;
        }

        // Calculate time bounds
        let minTime = Infinity;
        let maxTime = -Infinity;

        this.fibers.forEach(fiber => {
            const start = new Date(fiber.first_activity).getTime();
            const end = new Date(fiber.last_activity).getTime();
            minTime = Math.min(minTime, start);
            maxTime = Math.max(maxTime, end);
        });

        // Add some padding to the time range
        const padding = (maxTime - minTime) * 0.05;
        minTime -= padding;
        maxTime += padding;

        const timeRange = maxTime - minTime;
        // Use parent container width (viewport) not canvas width
        const viewportWidth = this.container.parentElement.offsetWidth || 800;

        // Fiber content width scales with zoom (how compressed/expanded the fibers are)
        const fiberContentWidth = viewportWidth * this.zoom;
        const pixelsPerMs = fiberContentWidth / timeRange;

        // Canvas includes content plus padding on each side to allow panning
        // beyond the fiber content even when zoomed out
        const paddingWidth = viewportWidth;
        const effectiveWidth = fiberContentWidth + paddingWidth * 2;
        const fiberOffset = paddingWidth; // Left padding where content starts

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
        if (this.selectedLogTimestamp) {
            console.log('Timeline: selectedLogTimestamp =', this.selectedLogTimestamp);
            const timestamp = new Date(this.selectedLogTimestamp).getTime();
            console.log('Timeline: timestamp (ms) =', timestamp);
            console.log('Timeline: minTime =', minTime, 'maxTime =', maxTime);
            console.log('Timeline: timestamp in range?', timestamp >= minTime && timestamp <= maxTime);

            if (timestamp >= minTime && timestamp <= maxTime) {
                const indicatorLeft = fiberOffset + (timestamp - minTime) * pixelsPerMs;
                console.log('Timeline: Creating indicator at left =', indicatorLeft);

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
                console.log('Timeline: Indicator appended');
            }
        }

        // Set container height based on number of fibers
        this.container.style.height = `${this.fibers.length * 14 + 20}px`;
        // Set container width to match the effective width used for positioning
        this.container.style.minWidth = `${effectiveWidth}px`;

        // Calculate visible time range (what's visible in the viewport at current zoom)
        const visibleTimeRange = timeRange / this.zoom;

        // Update time axis
        this.renderTimeAxis(minTime, maxTime, effectiveWidth, pixelsPerMs, visibleTimeRange, fiberOffset, paddingWidth);
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
