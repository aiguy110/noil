/**
 * Main application controller
 */
class NoilApp {
    constructor() {
        this.timeline = null;
        this.logViewer = null;
        this.filters = {
            showClosed: true,
            fiberTypes: [],
            timeRange: '24h',
        };
        this.allFibers = [];
        this.filteredFibers = [];

        // Navigation state
        this.navHistory = [];           // [{fiberId, logId}]
        this.navHistoryPosition = -1;   // Current position in history
        this.selectedLogId = null;      // Currently selected log line
        this.logFibersCache = {};       // Map: logId -> fiber list

        this.init();
    }

    async init() {
        // Initialize components
        this.initDrawer();
        this.initTabs();
        this.initTimeline();
        this.initLogViewer();
        this.initColorConfig();
        this.initFilters();
        this.initNavigationTab();

        // Load initial data
        await this.loadData();

        // Set up periodic refresh
        setInterval(() => this.refresh(), 30000); // Refresh every 30 seconds
    }

    initDrawer() {
        const drawer = document.getElementById('drawer');
        const toggleBtn = document.getElementById('drawer-toggle');

        toggleBtn.addEventListener('click', () => {
            drawer.classList.toggle('collapsed');
        });
    }

    initTabs() {
        const tabButtons = document.querySelectorAll('.tab-btn');
        const tabContents = document.querySelectorAll('.tab-content');

        tabButtons.forEach(btn => {
            btn.addEventListener('click', () => {
                const tabName = btn.dataset.tab;

                // Update active tab button
                tabButtons.forEach(b => b.classList.remove('active'));
                btn.classList.add('active');

                // Update active tab content
                tabContents.forEach(content => {
                    if (content.id === `tab-${tabName}`) {
                        content.classList.add('active');
                    } else {
                        content.classList.remove('active');
                    }
                });
            });
        });
    }

    initTimeline() {
        const container = document.getElementById('timeline-canvas');
        this.timeline = new Timeline(container, (fiberId) => {
            this.onFiberSelected(fiberId);
        });

        // Zoom controls
        document.getElementById('zoom-in').addEventListener('click', () => {
            this.timeline.zoomIn();
        });

        document.getElementById('zoom-out').addEventListener('click', () => {
            this.timeline.zoomOut();
        });

        document.getElementById('refresh').addEventListener('click', () => {
            this.refresh();
        });

        // Initialize timeline resize functionality
        this.initTimelineResize();
    }

    initTimelineResize() {
        const resizeHandle = document.getElementById('timeline-resize-handle');
        const timelineSection = document.getElementById('timeline-section');
        let isResizing = false;
        let startY = 0;
        let startHeight = 0;

        resizeHandle.addEventListener('mousedown', (e) => {
            isResizing = true;
            startY = e.clientY;
            startHeight = timelineSection.offsetHeight;
            resizeHandle.classList.add('dragging');

            // Prevent text selection during drag
            e.preventDefault();
        });

        document.addEventListener('mousemove', (e) => {
            if (!isResizing) return;

            const deltaY = e.clientY - startY;
            const newHeight = startHeight + deltaY;

            // Set min and max heights
            const minHeight = 100;
            const maxHeight = window.innerHeight - 300; // Leave room for log viewer

            if (newHeight >= minHeight && newHeight <= maxHeight) {
                timelineSection.style.height = `${newHeight}px`;
                // Update CSS variable for consistency
                document.documentElement.style.setProperty('--timeline-height', `${newHeight}px`);
            }
        });

        document.addEventListener('mouseup', () => {
            if (isResizing) {
                isResizing = false;
                resizeHandle.classList.remove('dragging');
            }
        });
    }

    initLogViewer() {
        const container = document.getElementById('log-content');
        const titleEl = document.getElementById('log-viewer-title');
        const infoEl = document.getElementById('fiber-info');

        this.logViewer = new LogViewer(container, titleEl, infoEl);

        // Load more button
        document.getElementById('load-more-logs').addEventListener('click', () => {
            this.logViewer.loadMore();
        });
    }

    async initColorConfig() {
        // Populate fiber type colors
        const fiberTypeContainer = document.getElementById('fiber-type-colors');
        const sourceContainer = document.getElementById('source-colors');

        // Get all fiber types and sources
        const fiberTypes = await api.getAllFiberTypes();
        const sources = await api.getAllSources();

        // Create color pickers for fiber types
        fiberTypes.forEach(type => {
            const item = this.createColorItem(
                type,
                colorManager.getFiberTypeColor(type),
                (color) => {
                    colorManager.setFiberTypeColor(type, color);
                    this.timeline.render();
                }
            );
            fiberTypeContainer.appendChild(item);
        });

        // Create color pickers for sources
        sources.forEach(source => {
            const item = this.createColorItem(
                source,
                colorManager.getSourceColor(source),
                (color) => {
                    colorManager.setSourceColor(source, color);
                    this.logViewer.render();
                }
            );
            sourceContainer.appendChild(item);
        });

        // Reset colors button
        document.getElementById('reset-colors').addEventListener('click', () => {
            colorManager.resetToDefaults();
            this.initColorConfig();
            this.timeline.render();
            this.logViewer.render();
        });
    }

    createColorItem(label, color, onChange) {
        const item = document.createElement('div');
        item.className = 'color-item';

        const labelEl = document.createElement('label');
        labelEl.textContent = label;

        const input = document.createElement('input');
        input.type = 'color';
        input.value = colorManager.hslToHex(color);

        input.addEventListener('change', (e) => {
            onChange(e.target.value);
        });

        item.appendChild(labelEl);
        item.appendChild(input);

        return item;
    }

    async initFilters() {
        // Populate filter options
        const fiberTypeSelect = document.getElementById('filter-fiber-type');

        const fiberTypes = await api.getAllFiberTypes();
        fiberTypes.forEach(type => {
            const option = document.createElement('option');
            option.value = type;
            option.textContent = type;
            fiberTypeSelect.appendChild(option);
        });

        // Apply filters button
        document.getElementById('apply-filters').addEventListener('click', () => {
            this.applyFilters();
        });
    }

    initNavigationTab() {
        // Back button
        document.getElementById('nav-back').addEventListener('click', () => {
            this.navigateBack();
        });

        // Forward button
        document.getElementById('nav-forward').addEventListener('click', () => {
            this.navigateForward();
        });

        // Clear selection button
        document.getElementById('nav-clear-selection').addEventListener('click', () => {
            this.clearLogSelection();
        });

        // Set up log selection callback
        this.logViewer.onLogSelect = (logId) => {
            this.selectLogLine(logId);
        };
    }

    pushNavHistory(fiberId, logId = null) {
        // If we're not at the end of history, truncate forward entries
        if (this.navHistoryPosition < this.navHistory.length - 1) {
            this.navHistory = this.navHistory.slice(0, this.navHistoryPosition + 1);
        }

        // Push new entry
        this.navHistory.push({ fiberId, logId });
        this.navHistoryPosition = this.navHistory.length - 1;

        this.updateNavigationUI();
    }

    navigateBack() {
        if (this.navHistoryPosition > 0) {
            this.navHistoryPosition--;
            const entry = this.navHistory[this.navHistoryPosition];

            // Update timeline selection without triggering callback
            this.timeline.selectedFiberId = entry.fiberId;
            this.timeline.render();

            // Load fiber logs
            this.logViewer.loadFiber(entry.fiberId);

            // Restore log selection if any
            if (entry.logId) {
                this.selectedLogId = entry.logId;
                this.logViewer.highlightLog(entry.logId);
                this.filterTimelineByLog(entry.logId);
            } else {
                this.clearLogSelection();
            }

            this.updateNavigationUI();
        }
    }

    navigateForward() {
        if (this.navHistoryPosition < this.navHistory.length - 1) {
            this.navHistoryPosition++;
            const entry = this.navHistory[this.navHistoryPosition];

            // Update timeline selection without triggering callback
            this.timeline.selectedFiberId = entry.fiberId;
            this.timeline.render();

            // Load fiber logs
            this.logViewer.loadFiber(entry.fiberId);

            // Restore log selection if any
            if (entry.logId) {
                this.selectedLogId = entry.logId;
                this.logViewer.highlightLog(entry.logId);
                this.filterTimelineByLog(entry.logId);
            } else {
                this.clearLogSelection();
            }

            this.updateNavigationUI();
        }
    }

    async selectLogLine(logId) {
        this.selectedLogId = logId;

        // Fetch fibers containing this log
        try {
            const data = await api.getLogFibers(logId);
            this.logFibersCache[logId] = data.fibers;

            // Filter timeline to only show these fibers
            this.filterTimelineByLog(logId);

            // Update navigation tab UI
            this.updateOtherFibersList(data.fibers);

            // Highlight the log
            this.logViewer.highlightLog(logId);

            // Show clear selection button
            document.getElementById('nav-clear-selection').style.display = 'block';
            document.getElementById('nav-other-fibers-section').style.display = 'block';

        } catch (error) {
            console.error('Failed to fetch fibers for log:', error);
        }
    }

    filterTimelineByLog(logId) {
        const fiberIds = this.logFibersCache[logId]?.map(f => f.id) || [];
        const filteredFibers = this.allFibers.filter(f => fiberIds.includes(f.id));
        this.timeline.setFibers(filteredFibers);
    }

    clearLogSelection() {
        this.selectedLogId = null;
        this.logViewer.highlightLog(null);

        // Restore full filtered timeline
        this.timeline.setFibers(this.filteredFibers);

        // Hide UI elements
        document.getElementById('nav-clear-selection').style.display = 'none';
        document.getElementById('nav-other-fibers-section').style.display = 'none';
    }

    updateNavigationUI() {
        // Update back/forward button states
        document.getElementById('nav-back').disabled = this.navHistoryPosition <= 0;
        document.getElementById('nav-forward').disabled =
            this.navHistoryPosition >= this.navHistory.length - 1;

        // Render history list
        this.renderHistoryList();
    }

    renderHistoryList() {
        const container = document.getElementById('nav-history-list');

        if (this.navHistory.length === 0) {
            container.innerHTML = '<p class="empty-message">No navigation history</p>';
            return;
        }

        container.innerHTML = '';

        this.navHistory.forEach((entry, index) => {
            const fiber = this.allFibers.find(f => f.id === entry.fiberId);
            if (!fiber) return;

            const item = document.createElement('div');
            item.className = 'nav-item';
            if (index === this.navHistoryPosition) {
                item.classList.add('current');
            }

            item.innerHTML = `
                <div class="nav-item-type">${fiber.fiber_type}</div>
                <div class="nav-item-id">${fiber.id.substring(0, 8)}...</div>
            `;

            item.addEventListener('click', () => {
                this.navHistoryPosition = index;
                const historyEntry = this.navHistory[index];

                // Update timeline selection without triggering callback
                this.timeline.selectedFiberId = historyEntry.fiberId;
                this.timeline.render();

                // Load fiber logs
                this.logViewer.loadFiber(historyEntry.fiberId);

                if (historyEntry.logId) {
                    this.selectedLogId = historyEntry.logId;
                    this.logViewer.highlightLog(historyEntry.logId);
                    this.filterTimelineByLog(historyEntry.logId);
                } else {
                    this.clearLogSelection();
                }

                this.updateNavigationUI();
            });

            container.appendChild(item);
        });
    }

    updateOtherFibersList(fibers) {
        const container = document.getElementById('nav-other-fibers-list');
        const currentFiberId = this.timeline.selectedFiberId;

        // Filter out current fiber
        const otherFibers = fibers.filter(f => f.id !== currentFiberId);

        if (otherFibers.length === 0) {
            container.innerHTML = '<p class="empty-message">No other fibers</p>';
            return;
        }

        container.innerHTML = '';

        otherFibers.forEach(fiber => {
            const item = document.createElement('div');
            item.className = 'nav-item';

            const color = colorManager.getFiberTypeColor(fiber.fiber_type);
            item.style.borderLeftColor = color;

            item.innerHTML = `
                <div class="nav-item-type">${fiber.fiber_type}</div>
                <div class="nav-item-id">${fiber.id.substring(0, 8)}...</div>
            `;

            item.addEventListener('click', () => {
                // Push to history with current log selection
                this.pushNavHistory(fiber.id, this.selectedLogId);

                // Update timeline selection without triggering callback
                this.timeline.selectedFiberId = fiber.id;
                this.timeline.render();

                // Load fiber logs
                this.logViewer.loadFiber(fiber.id);
            });

            container.appendChild(item);
        });
    }

    applyFilters() {
        // Get filter values
        this.filters.showClosed = document.getElementById('filter-closed').checked;

        const fiberTypeSelect = document.getElementById('filter-fiber-type');
        const selectedTypes = Array.from(fiberTypeSelect.selectedOptions).map(opt => opt.value);
        this.filters.fiberTypes = selectedTypes.filter(t => t !== '');

        this.filters.timeRange = document.getElementById('filter-time-range').value;

        // Apply filters to fibers
        this.filterFibers();
    }

    filterFibers() {
        this.filteredFibers = this.allFibers.filter(fiber => {
            // Filter by closed status
            if (!this.filters.showClosed && fiber.closed) {
                return false;
            }

            // Filter by fiber type
            if (this.filters.fiberTypes.length > 0) {
                if (!this.filters.fiberTypes.includes(fiber.fiber_type)) {
                    return false;
                }
            }

            // Filter by time range
            if (this.filters.timeRange !== 'all') {
                const now = Date.now();
                const lastActivity = new Date(fiber.last_activity).getTime();
                const ranges = {
                    '1h': 3600000,
                    '6h': 21600000,
                    '24h': 86400000,
                    '7d': 604800000,
                };

                if (now - lastActivity > ranges[this.filters.timeRange]) {
                    return false;
                }
            }

            return true;
        });

        this.timeline.setFibers(this.filteredFibers);
    }

    async loadData() {
        try {
            this.showLoading(true);

            // Get all fiber types
            const fiberTypes = await api.getAllFiberTypes();

            // Fetch fibers for each type
            const allFibers = [];
            for (const type of fiberTypes) {
                try {
                    const response = await api.listFibers({
                        type: type,
                        limit: 1000,
                    });
                    allFibers.push(...response.fibers);
                } catch (error) {
                    console.warn(`Failed to fetch fibers of type ${type}:`, error);
                }
            }

            this.allFibers = allFibers;
            this.applyFilters();
        } catch (error) {
            console.error('Failed to load data:', error);
        } finally {
            this.showLoading(false);
        }
    }

    async refresh() {
        await this.loadData();
        // Re-render current fiber if one is selected
        if (this.timeline.selectedFiberId) {
            await this.logViewer.loadFiber(this.timeline.selectedFiberId);
        }
    }

    async onFiberSelected(fiberId) {
        // Skip if selecting the same fiber
        if (this.navHistory[this.navHistoryPosition]?.fiberId === fiberId) {
            await this.logViewer.loadFiber(fiberId);
            return;
        }

        if (this.selectedLogId) {
            // If a log is selected, add to navigation history (building a path)
            this.pushNavHistory(fiberId, this.selectedLogId);
        } else {
            // If no log is selected, replace history with new root
            this.navHistory = [{ fiberId, logId: null }];
            this.navHistoryPosition = 0;
            this.updateNavigationUI();
        }

        await this.logViewer.loadFiber(fiberId);
    }

    showLoading(show) {
        const overlay = document.getElementById('loading-overlay');
        overlay.style.display = show ? 'flex' : 'none';
    }
}

// Start the application when DOM is ready
document.addEventListener('DOMContentLoaded', () => {
    window.app = new NoilApp();
});
