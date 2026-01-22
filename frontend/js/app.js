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
        this.navHistory = [];           // [{fiberId, logId, timestamp, sourceId}]
        this.navHistoryPosition = -1;   // Current position in history
        this.selectedLogId = null;      // Currently selected log line
        this.selectedLogTimestamp = null; // Timestamp of selected log line
        this.selectedLogSourceId = null; // Source ID of selected log line
        this.logFibersCache = {};       // Map: logId -> fiber list

        this.init();
    }

    async init() {
        // Initialize components
        this.initHeader();
        this.initHamburgerMenu();
        this.initSettingsModal();
        this.initDrawer();
        this.initTabs();
        this.initTimeline();
        this.initLogViewer();
        this.initModalColorConfig();
        this.initFilters();
        this.initNavigationTab();

        // Load initial data
        await this.loadData();

        // Set up periodic refresh (silent)
        setInterval(() => this.refresh(true), 30000); // Refresh every 30 seconds, silently
    }

    async initHeader() {
        // Fetch and display version
        try {
            const health = await api.health();
            document.getElementById('header-version').textContent = `(v${health.version})`;
        } catch (error) {
            console.error('Failed to fetch version:', error);
            document.getElementById('header-version').textContent = '(v?.?.?)';
        }
    }

    initHamburgerMenu() {
        const hamburgerBtn = document.getElementById('hamburger-menu');
        const dropdown = document.getElementById('hamburger-dropdown');

        // Toggle dropdown
        hamburgerBtn.addEventListener('click', (e) => {
            e.stopPropagation();
            const isVisible = dropdown.style.display === 'block';
            dropdown.style.display = isVisible ? 'none' : 'block';
        });

        // Close dropdown when clicking outside
        document.addEventListener('click', (e) => {
            if (!dropdown.contains(e.target) && e.target !== hamburgerBtn) {
                dropdown.style.display = 'none';
            }
        });

        // Menu items
        document.getElementById('menu-fiber-processing').addEventListener('click', () => {
            dropdown.style.display = 'none';
            // Stub for now
            alert('Fiber Processing (coming soon)');
        });

        document.getElementById('menu-settings').addEventListener('click', () => {
            dropdown.style.display = 'none';
            this.openSettingsModal();
        });
    }

    initSettingsModal() {
        const modal = document.getElementById('settings-modal');
        const closeBtn = document.getElementById('close-settings-modal');

        // Close modal
        closeBtn.addEventListener('click', () => {
            this.closeSettingsModal();
        });

        // Close modal when clicking outside
        modal.addEventListener('click', (e) => {
            if (e.target === modal) {
                this.closeSettingsModal();
            }
        });

        // Modal tabs
        const tabButtons = document.querySelectorAll('.modal-tab-btn');
        const tabContents = document.querySelectorAll('.modal-tab-content');

        tabButtons.forEach(btn => {
            btn.addEventListener('click', () => {
                const tabName = btn.dataset.modalTab;

                // Update active tab button
                tabButtons.forEach(b => b.classList.remove('active'));
                btn.classList.add('active');

                // Update active tab content
                tabContents.forEach(content => {
                    if (content.id === `modal-tab-${tabName}`) {
                        content.classList.add('active');
                    } else {
                        content.classList.remove('active');
                    }
                });
            });
        });
    }

    openSettingsModal() {
        document.getElementById('settings-modal').style.display = 'flex';
    }

    closeSettingsModal() {
        document.getElementById('settings-modal').style.display = 'none';
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

        // Refresh button - reset timeline view
        document.getElementById('refresh').addEventListener('click', () => {
            this.timeline.resetView();
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

    async initModalColorConfig() {
        // Populate fiber type colors in modal
        const fiberTypeContainer = document.getElementById('modal-fiber-type-colors');

        // Get all fiber type metadata
        const fiberTypeMetadata = await api.getAllFiberTypes();

        // Sort fiber types: source fiber types first, then traced
        const sortedMetadata = this.sortFiberTypeMetadata(fiberTypeMetadata);

        // Create color pickers for fiber types
        sortedMetadata.forEach(metadata => {
            const item = this.createColorItem(
                metadata.name,
                colorManager.getFiberTypeColor(metadata.name),
                (color) => {
                    colorManager.setFiberTypeColor(metadata.name, color);
                    this.timeline.render();
                    this.logViewer.render();
                }
            );
            fiberTypeContainer.appendChild(item);
        });

        // Reset colors button
        document.getElementById('modal-reset-colors').addEventListener('click', () => {
            colorManager.resetToDefaults();
            // Clear and re-populate
            fiberTypeContainer.innerHTML = '';
            this.initModalColorConfig();
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

        const fiberTypeMetadata = await api.getAllFiberTypes();

        // Sort fiber types: source fiber types first, then traced
        const sortedMetadata = this.sortFiberTypeMetadata(fiberTypeMetadata);

        sortedMetadata.forEach(metadata => {
            const option = document.createElement('option');
            option.value = metadata.name;
            option.textContent = metadata.name;
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

    pushNavHistory(fiberId, logId = null, timestamp = null, sourceId = null) {
        // Check if this fiber+log combo already exists in history
        const existingIndex = this.navHistory.findIndex(entry =>
            entry.fiberId === fiberId && entry.logId === logId
        );

        if (existingIndex !== -1) {
            // Found existing entry - just navigate to it
            this.navHistoryPosition = existingIndex;
            this.updateNavigationUI();
            return;
        }

        // New entry - truncate forward history if we're not at the end
        if (this.navHistoryPosition < this.navHistory.length - 1) {
            this.navHistory = this.navHistory.slice(0, this.navHistoryPosition + 1);
        }

        // Push new entry
        this.navHistory.push({ fiberId, logId, timestamp, sourceId });
        this.navHistoryPosition = this.navHistory.length - 1;

        this.updateNavigationUI();

        // Update timeline filter if a log is selected (to reflect truncated history)
        if (logId) {
            this.filterTimelineByLog(logId);
        }
    }

    async navigateBack() {
        if (this.navHistoryPosition > 0) {
            this.navHistoryPosition--;
            const entry = this.navHistory[this.navHistoryPosition];

            // Update timeline selection without triggering callback
            this.timeline.selectedFiberId = entry.fiberId;
            this.timeline.render();

            // Load fiber logs
            await this.logViewer.loadFiber(entry.fiberId);

            // Restore log selection if any
            if (entry.logId) {
                this.selectedLogId = entry.logId;
                this.selectedLogTimestamp = entry.timestamp;
                this.logViewer.highlightLog(entry.logId);
                this.filterTimelineByLog(entry.logId);

                // Update other fibers list to reflect current fiber
                if (this.logFibersCache[entry.logId]) {
                    this.updateOtherFibersList(this.logFibersCache[entry.logId]);
                }
            } else {
                this.clearLogSelection();
            }

            this.updateNavigationUI();
        }
    }

    async navigateForward() {
        if (this.navHistoryPosition < this.navHistory.length - 1) {
            this.navHistoryPosition++;
            const entry = this.navHistory[this.navHistoryPosition];

            // Update timeline selection without triggering callback
            this.timeline.selectedFiberId = entry.fiberId;
            this.timeline.render();

            // Load fiber logs
            await this.logViewer.loadFiber(entry.fiberId);

            // Restore log selection if any
            if (entry.logId) {
                this.selectedLogId = entry.logId;
                this.selectedLogTimestamp = entry.timestamp;
                this.logViewer.highlightLog(entry.logId);
                this.filterTimelineByLog(entry.logId);

                // Update other fibers list to reflect current fiber
                if (this.logFibersCache[entry.logId]) {
                    this.updateOtherFibersList(this.logFibersCache[entry.logId]);
                }
            } else {
                this.clearLogSelection();
            }

            this.updateNavigationUI();
        }
    }

    async selectLogLine(logId) {
        this.selectedLogId = logId;

        // Find the log's timestamp and source from loaded logs
        console.log('App: Looking for log', logId, 'in', this.logViewer.logs.length, 'logs');
        const logData = this.logViewer.logs.find(log => log.id === logId);
        console.log('App: Found logData =', logData);
        let logTimestamp = null;
        let logSourceId = null;
        if (logData) {
            console.log('App: Setting timestamp =', logData.timestamp);
            logTimestamp = logData.timestamp;
            logSourceId = logData.source_id;
            this.timeline.setSelectedLogTimestamp(logData.timestamp);
        } else {
            console.log('App: Log not found in logViewer.logs');
        }

        // Store timestamp and source for potential use in navigation history
        this.selectedLogTimestamp = logTimestamp;
        this.selectedLogSourceId = logSourceId;

        // Update the current navigation entry with the selected log's info
        if (this.navHistory.length > 0 && this.navHistoryPosition >= 0) {
            const currentEntry = this.navHistory[this.navHistoryPosition];
            currentEntry.logId = logId;
            currentEntry.timestamp = logTimestamp;
            currentEntry.sourceId = logSourceId;
            this.updateNavigationUI();
        }

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
        // Get fibers containing the selected log
        const fiberIds = this.logFibersCache[logId]?.map(f => f.id) || [];

        // Get fiber IDs from navigation history up to current position
        // (don't include "future" history that user has navigated back from)
        const navHistoryFiberIds = this.navHistory
            .slice(0, this.navHistoryPosition + 1)
            .map(entry => entry.fiberId);

        // Combine both sets (using Set to avoid duplicates)
        const allFiberIds = new Set([...fiberIds, ...navHistoryFiberIds]);

        // Filter timeline to show all these fibers
        const filteredFibers = this.allFibers.filter(f => allFiberIds.has(f.id));
        this.timeline.setFibers(filteredFibers);
    }

    clearLogSelection() {
        this.selectedLogId = null;
        this.selectedLogTimestamp = null;
        this.selectedLogSourceId = null;
        this.logViewer.highlightLog(null);
        this.timeline.clearSelectedLogTimestamp();

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

            // Set fiber type color for the border
            const color = colorManager.getFiberTypeColor(fiber.fiber_type);
            item.style.borderLeftColor = color;

            // Format timestamp and source badge if available
            let timestampInfoHtml = '';
            if (entry.logId && entry.timestamp) {
                const date = new Date(entry.timestamp);
                const year = date.getFullYear();
                const month = String(date.getMonth() + 1).padStart(2, '0');
                const day = String(date.getDate()).padStart(2, '0');
                const hours = String(date.getHours()).padStart(2, '0');
                const minutes = String(date.getMinutes()).padStart(2, '0');
                const seconds = String(date.getSeconds()).padStart(2, '0');
                const ms = String(date.getMilliseconds()).padStart(3, '0');
                const timeStr = `${year}-${month}-${day} ${hours}:${minutes}:${seconds}.${ms}`;

                // Add source badge if available
                let sourceBadgeHtml = '';
                if (entry.sourceId) {
                    const fiberTypeColor = colorManager.getFiberTypeColor(entry.sourceId);
                    sourceBadgeHtml = `<span class="nav-item-source" style="background-color: ${fiberTypeColor};">${entry.sourceId}</span>`;
                }

                timestampInfoHtml = `
                    <div class="nav-item-section-label">Line:</div>
                    <div class="nav-item-timestamp-row">
                        <span class="nav-item-timestamp">${timeStr}</span>
                        ${sourceBadgeHtml}
                    </div>`;
            }

            item.innerHTML = `
                <div class="nav-item-section-label">Fiber:</div>
                <div class="nav-item-fiber-row">
                    <span class="nav-item-type">${fiber.fiber_type}</span>
                    <span class="nav-item-id">${fiber.id.substring(0, 8)}...</span>
                </div>
                ${timestampInfoHtml}
            `;

            item.addEventListener('click', async () => {
                this.navHistoryPosition = index;
                const historyEntry = this.navHistory[index];

                // Update timeline selection without triggering callback
                this.timeline.selectedFiberId = historyEntry.fiberId;
                this.timeline.render();

                // Load fiber logs
                await this.logViewer.loadFiber(historyEntry.fiberId);

                if (historyEntry.logId) {
                    this.selectedLogId = historyEntry.logId;
                    this.selectedLogTimestamp = historyEntry.timestamp;
                    this.logViewer.highlightLog(historyEntry.logId);
                    this.filterTimelineByLog(historyEntry.logId);

                    // Update other fibers list to reflect current fiber
                    if (this.logFibersCache[historyEntry.logId]) {
                        this.updateOtherFibersList(this.logFibersCache[historyEntry.logId]);
                    }
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
                <div class="nav-item-section-label">Fiber:</div>
                <div class="nav-item-fiber-row">
                    <span class="nav-item-type">${fiber.fiber_type}</span>
                    <span class="nav-item-id">${fiber.id.substring(0, 8)}...</span>
                </div>
            `;

            item.addEventListener('click', async () => {
                // Push to history with current log selection
                this.pushNavHistory(fiber.id, this.selectedLogId, this.selectedLogTimestamp, this.selectedLogSourceId);

                // Update timeline selection without triggering callback
                this.timeline.selectedFiberId = fiber.id;
                this.timeline.render();

                // Load fiber logs
                await this.logViewer.loadFiber(fiber.id);

                // Highlight the selected log in the new fiber (if it exists)
                if (this.selectedLogId) {
                    this.logViewer.highlightLog(this.selectedLogId);
                }

                // Update the other fibers list to reflect the new current fiber
                if (this.selectedLogId && this.logFibersCache[this.selectedLogId]) {
                    this.updateOtherFibersList(this.logFibersCache[this.selectedLogId]);
                }
            });

            container.appendChild(item);
        });
    }

    sortFiberTypeMetadata(fiberTypeMetadata) {
        // Separate source fiber types from traced fiber types
        const sourceFiberTypes = fiberTypeMetadata.filter(ft => ft.is_source_fiber);
        const tracedFiberTypes = fiberTypeMetadata.filter(ft => !ft.is_source_fiber);

        // Sort each group alphabetically by name
        sourceFiberTypes.sort((a, b) => a.name.localeCompare(b.name));
        tracedFiberTypes.sort((a, b) => a.name.localeCompare(b.name));

        // Return source fiber types first, then traced
        return [...sourceFiberTypes, ...tracedFiberTypes];
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

    async loadData(silent = false) {
        try {
            if (!silent) {
                this.showLoading(true);
            }

            // Get all fiber types
            const fiberTypes = await api.getAllFiberTypes();

            // Fetch fibers for each type
            const allFibers = [];
            for (const metadata of fiberTypes) {
                try {
                    const response = await api.listFibers({
                        type: metadata.name,
                        limit: 1000,
                    });
                    allFibers.push(...response.fibers);
                } catch (error) {
                    console.warn(`Failed to fetch fibers of type ${metadata.name}:`, error);
                }
            }

            this.allFibers = allFibers;
            this.applyFilters();
        } catch (error) {
            console.error('Failed to load data:', error);
        } finally {
            if (!silent) {
                this.showLoading(false);
            }
        }
    }

    async refresh(silent = false) {
        await this.loadData(silent);

        // Re-apply log-based filter if a log is selected
        if (this.selectedLogId) {
            this.filterTimelineByLog(this.selectedLogId);
        }

        // Re-render current fiber if one is selected
        if (this.timeline.selectedFiberId) {
            // If silent refresh, check if there are any changes before reloading
            if (silent) {
                const hasChanges = await this.checkFiberHasChanges(this.timeline.selectedFiberId);
                if (!hasChanges) {
                    // No changes, skip reload
                    return;
                }
            }

            await this.logViewer.loadFiber(this.timeline.selectedFiberId, silent);

            // Restore selection highlight if a log was selected
            if (this.selectedLogId) {
                this.logViewer.highlightLog(this.selectedLogId);
            }
        }
    }

    async checkFiberHasChanges(fiberId) {
        try {
            // Fetch current fiber metadata
            const freshFiber = await api.getFiber(fiberId);
            const currentFiber = this.logViewer.currentFiber;

            if (!currentFiber) {
                return true; // No current fiber loaded, should reload
            }

            // Check if last_activity has changed (indicates new logs)
            if (freshFiber.last_activity !== currentFiber.last_activity) {
                return true;
            }

            // Check if closed status changed
            if (freshFiber.closed !== currentFiber.closed) {
                return true;
            }

            return false; // No changes detected
        } catch (error) {
            console.error('Failed to check fiber changes:', error);
            return true; // On error, assume there are changes to be safe
        }
    }

    async onFiberSelected(fiberId) {
        // Skip if selecting the same fiber
        if (this.navHistory[this.navHistoryPosition]?.fiberId === fiberId) {
            await this.logViewer.loadFiber(fiberId);
            // Restore highlight if log was selected
            if (this.selectedLogId) {
                this.logViewer.highlightLog(this.selectedLogId);
            }
            return;
        }

        if (this.selectedLogId) {
            // If a log is selected, add to navigation history (building a path)
            this.pushNavHistory(fiberId, this.selectedLogId, this.selectedLogTimestamp, this.selectedLogSourceId);
        } else {
            // If no log is selected, replace history with new root
            this.navHistory = [{ fiberId, logId: null, timestamp: null, sourceId: null }];
            this.navHistoryPosition = 0;
            this.updateNavigationUI();
        }

        await this.logViewer.loadFiber(fiberId);

        // Highlight the selected log in the new fiber (if it exists)
        if (this.selectedLogId) {
            this.logViewer.highlightLog(this.selectedLogId);
        }
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
