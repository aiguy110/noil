/**
 * Timeline visualization for fibers
 */
class Timeline {
    constructor(containerEl, onFiberSelect) {
        this.container = containerEl;
        this.onFiberSelect = onFiberSelect;
        this.fibers = [];
        this.fiberTypeMetadata = [];
        this.selectedFiberId = null;
        this.selectedLogTimestamp = null;
        this.zoom = 1.0;
        this.minZoom = 0.1;
        this.maxZoom = 10.0;

        this.loadFiberTypeMetadata();
        this.render();
    }

    async loadFiberTypeMetadata() {
        try {
            this.fiberTypeMetadata = await api.getAllFiberTypes();
        } catch (error) {
            console.error('Failed to load fiber type metadata:', error);
            this.fiberTypeMetadata = [];
        }
    }

    setFibers(fibers) {
        this.fibers = this.sortFibers(fibers);
        this.render();
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
        this.zoom = Math.min(this.maxZoom, this.zoom * 1.5);
        this.render();
    }

    zoomOut() {
        this.zoom = Math.max(this.minZoom, this.zoom / 1.5);
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
        const containerWidth = this.container.offsetWidth || 800;
        const pixelsPerMs = (containerWidth * this.zoom) / timeRange;

        // Clear container
        this.container.innerHTML = '';

        // Create fiber lines
        this.fibers.forEach((fiber, index) => {
            const start = new Date(fiber.first_activity).getTime();
            const end = new Date(fiber.last_activity).getTime();

            const left = (start - minTime) * pixelsPerMs;
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
                const indicatorLeft = (timestamp - minTime) * pixelsPerMs;
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
        this.container.style.minWidth = `${containerWidth * this.zoom}px`;

        // Update time axis
        this.renderTimeAxis(minTime, maxTime, containerWidth, pixelsPerMs);
    }

    renderTimeAxis(minTime, maxTime, containerWidth, pixelsPerMs) {
        const axisEl = document.getElementById('timeline-axis');
        if (!axisEl) return;

        axisEl.innerHTML = '';

        // Calculate appropriate time intervals
        const timeRange = maxTime - minTime;
        const numLabels = Math.floor(containerWidth / 100); // One label every 100px
        const interval = timeRange / numLabels;

        for (let i = 0; i <= numLabels; i++) {
            const time = minTime + (interval * i);
            const position = (time - minTime) * pixelsPerMs;

            const label = document.createElement('span');
            label.className = 'time-label';
            label.textContent = this.formatTime(new Date(time));
            label.style.left = `${position + 20}px`; // +20 for padding

            axisEl.appendChild(label);
        }
    }

    formatTime(date) {
        const hours = String(date.getHours()).padStart(2, '0');
        const minutes = String(date.getMinutes()).padStart(2, '0');
        const seconds = String(date.getSeconds()).padStart(2, '0');
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
