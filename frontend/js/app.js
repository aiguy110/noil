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
            attributes: [],          // Array of attribute filters
            attributeLogic: 'AND'    // 'AND' or 'OR'
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
        this.fiberProcessingEditorPage = null; // Fiber processing editor instance for full page

        // Page state
        this.currentPage = 'log-view';

        // Working Set state (global, persists across fiber types)
        this.workingSet = this.loadWorkingSetFromStorage();
        this.contextMenu = null; // Will be initialized in init()

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
        this.initAttributesDrawer();
        this.initModalColorConfig();
        this.initFilters();
        this.initNavigationTab();
        this.initFiberRulesPage();
        this.initContextMenu();
        this.initFiberContextMenu();
        this.initWorkingSetIndicators();

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

        // Toggle dropdown on hamburger click
        hamburgerBtn.addEventListener('click', (e) => {
            e.stopPropagation();
            dropdown.classList.toggle('open');
        });

        // Close dropdown when clicking outside
        document.addEventListener('click', (e) => {
            if (!dropdown.contains(e.target) && !hamburgerBtn.contains(e.target)) {
                dropdown.classList.remove('open');
            }
        });

        // Handle menu item clicks
        const menuItems = dropdown.querySelectorAll('.hamburger-menu-item');
        menuItems.forEach(item => {
            item.addEventListener('click', () => {
                const page = item.dataset.page;
                const action = item.dataset.action;

                if (page) {
                    this.switchPage(page);
                } else if (action === 'settings') {
                    this.openSettingsModal();
                }

                dropdown.classList.remove('open');
            });
        });

        // Set initial active state
        this.updateHamburgerMenuActiveState();
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
            btn.addEventListener('click', async () => {
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

    async openSettingsModal() {
        document.getElementById('settings-modal').style.display = 'flex';

        // Initialize config editor on first open
        if (!this.configEditor) {
            this.configEditor = new ConfigEditor(api);
            await this.configEditor.init();
        }
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

    initAttributesDrawer() {
        const closeBtn = document.getElementById('close-attributes-drawer');
        const toggleBtn = document.getElementById('toggle-attributes-view');
        const drawer = document.getElementById('attributes-drawer');

        closeBtn.addEventListener('click', () => {
            this.logViewer.closeAttributesDrawer();
        });

        toggleBtn.addEventListener('click', () => {
            this.logViewer.toggleAttributesView();
        });

        // Close drawer when clicking outside
        drawer.addEventListener('click', (e) => {
            if (e.target === drawer) {
                this.logViewer.closeAttributesDrawer();
            }
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
        this.timeline = new Timeline(
            container,
            (fiberId) => {
                this.onFiberSelected(fiberId);
            },
            (event, fiberId) => {
                this.showFiberContextMenu(event, fiberId);
            }
        );

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

        // Initialize attribute filter chip container
        this.renderAttributeFilterChips();

        // Attach event listener to attribute logic toggle
        const logicRadios = document.querySelectorAll('input[name="attr-logic"]');
        logicRadios.forEach(radio => {
            radio.addEventListener('change', () => {
                this.filters.attributeLogic = radio.value;
                this.filterFibers();
            });
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

            // Add right-click context menu
            item.addEventListener('contextmenu', (e) => {
                e.preventDefault();
                this.showFiberContextMenu(e, entry.fiberId);
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

            // Add right-click context menu
            item.addEventListener('contextmenu', (e) => {
                e.preventDefault();
                this.showFiberContextMenu(e, fiber.id);
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

        // Get attribute filter logic
        const logicRadio = document.querySelector('input[name="attr-logic"]:checked');
        this.filters.attributeLogic = logicRadio ? logicRadio.value : 'AND';

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

            // Filter by attributes
            if (this.filters.attributes.length > 0) {
                const attributeMatch = this.matchesAttributeFilters(fiber);
                if (!attributeMatch) {
                    return false;
                }
            }

            return true;
        });

        this.timeline.setFibers(this.filteredFibers);
    }

    matchesAttributeFilters(fiber) {
        if (this.filters.attributes.length === 0) {
            return true;
        }

        const fiberAttrs = fiber.attributes || {};
        const logic = this.filters.attributeLogic;

        if (logic === 'AND') {
            // All filters must match
            return this.filters.attributes.every(filter => {
                return this.matchesAttributeFilter(fiberAttrs, filter);
            });
        } else {
            // At least one filter must match
            return this.filters.attributes.some(filter => {
                return this.matchesAttributeFilter(fiberAttrs, filter);
            });
        }
    }

    matchesAttributeFilter(fiberAttrs, filter) {
        const fiberValue = fiberAttrs[filter.key];

        // If fiber doesn't have this attribute, no match
        if (fiberValue === undefined || fiberValue === null) {
            return false;
        }

        const fiberValueStr = String(fiberValue);

        if (filter.regex) {
            // Regex matching
            try {
                const regex = new RegExp(filter.value, 'i'); // Case-insensitive
                return regex.test(fiberValueStr);
            } catch (e) {
                // Invalid regex, fall back to exact match
                console.warn('Invalid regex pattern:', filter.value, e);
                return fiberValueStr === filter.value;
            }
        } else {
            // Exact match
            return fiberValueStr === filter.value;
        }
    }

    addAttributeFilter(key, value, regex = false) {
        // Check if this exact filter already exists
        const exists = this.filters.attributes.some(
            f => f.key === key && f.value === value && f.regex === regex
        );

        if (exists) {
            // Switch to Filters tab to show the existing filter
            this.switchToFiltersTab();
            return;
        }

        // Add filter
        const filter = {
            id: `attr-${Date.now()}-${Math.random()}`,
            key,
            value,
            regex
        };

        this.filters.attributes.push(filter);

        // Update UI
        this.renderAttributeFilterChips();

        // Auto-apply filters
        this.filterFibers();

        // Switch to Filters tab to show the new chip
        this.switchToFiltersTab();
    }

    removeAttributeFilter(filterId) {
        this.filters.attributes = this.filters.attributes.filter(f => f.id !== filterId);
        this.renderAttributeFilterChips();
        this.filterFibers();
    }

    toggleAttributeFilterRegex(filterId) {
        const filter = this.filters.attributes.find(f => f.id === filterId);
        if (filter) {
            filter.regex = !filter.regex;
            this.renderAttributeFilterChips();
            this.filterFibers();
        }
    }

    renderAttributeFilterChips() {
        const container = document.getElementById('attribute-filter-chips');
        if (!container) return;

        if (this.filters.attributes.length === 0) {
            container.innerHTML = '<div class="empty-chips">No attribute filters</div>';
            return;
        }

        let html = '';
        this.filters.attributes.forEach(filter => {
            const regexBadge = filter.regex ? '<span class="regex-badge">regex</span>' : '';
            html += `
                <div class="filter-chip">
                    <span class="chip-content">
                        <strong>${this.escapeHtml(filter.key)}</strong>: ${this.escapeHtml(filter.value)}
                        ${regexBadge}
                    </span>
                    <button class="chip-edit" data-filter-id="${filter.id}" title="Toggle regex">⚙</button>
                    <button class="chip-remove" data-filter-id="${filter.id}" title="Remove filter">×</button>
                </div>
            `;
        });

        container.innerHTML = html;

        // Attach event listeners
        container.querySelectorAll('.chip-remove').forEach(btn => {
            btn.addEventListener('click', (e) => {
                this.removeAttributeFilter(e.target.getAttribute('data-filter-id'));
            });
        });

        container.querySelectorAll('.chip-edit').forEach(btn => {
            btn.addEventListener('click', (e) => {
                this.toggleAttributeFilterRegex(e.target.getAttribute('data-filter-id'));
            });
        });
    }

    switchToFiltersTab() {
        // Simulate click on Filters tab
        const filtersTab = document.querySelector('[data-tab="filters"]');
        if (filtersTab && !filtersTab.classList.contains('active')) {
            filtersTab.click();
        }
    }

    escapeHtml(text) {
        const div = document.createElement('div');
        div.textContent = text;
        return div.innerHTML;
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

    async initFiberRulesPage() {
        // Initialize fiber processing editor for the full page
        // We'll initialize it lazily when the page is first opened
    }

    updateHamburgerMenuActiveState() {
        document.querySelectorAll('.hamburger-menu-item').forEach(item => {
            if (item.dataset.page === this.currentPage) {
                item.classList.add('active');
            } else {
                item.classList.remove('active');
            }
        });
    }

    async switchPage(pageName) {
        // Hide all pages
        document.querySelectorAll('.page').forEach(page => {
            page.classList.remove('active');
        });

        // Show the selected page
        const pageEl = document.getElementById(`${pageName}-page`);
        if (pageEl) {
            pageEl.classList.add('active');
            this.currentPage = pageName;

            // Update active menu item
            this.updateHamburgerMenuActiveState();

            // Initialize fiber processing editor on first open
            if (pageName === 'fiber-rules' && !this.fiberProcessingEditorPage) {
                this.fiberProcessingEditorPage = new FiberProcessingEditor(api, {
                    typeListEl: 'fiber-type-list-page',
                    typeNameEl: 'fiber-type-name-page',
                    editorEl: 'fiber-yaml-editor-page',
                    saveBtnEl: 'fiber-save-page',
                    activateBtnEl: 'fiber-hot-reload-page',
                    deleteBtnEl: 'fiber-delete-page',
                    newBtnEl: 'new-fiber-type-page',
                    statusEl: 'fiber-status-page',
                    reprocessBtnEl: 'start-reprocess-page',
                    reprocessProgressEl: 'reprocess-progress-page',
                    reprocessProgressBarEl: 'reprocess-progress-bar-page',
                    reprocessStatusTextEl: 'reprocess-status-text-page',
                    reprocessProgressTextEl: 'reprocess-progress-text-page',
                    cancelReprocessBtnEl: 'cancel-reprocess-page',
                });
                await this.fiberProcessingEditorPage.init();
            }
        }
    }

    showLoading(show) {
        const overlay = document.getElementById('loading-overlay');
        overlay.style.display = show ? 'flex' : 'none';
    }

    // =========================================================================
    // Working Set Management
    // =========================================================================

    loadWorkingSetFromStorage() {
        try {
            const stored = localStorage.getItem('noil_working_set');
            if (!stored) {
                return { logIds: [], logs: {}, timestamp: new Date().toISOString() };
            }

            const data = JSON.parse(stored);
            return {
                logIds: data.logIds || [],
                logs: {}, // Will be lazy-loaded when needed
                timestamp: data.timestamp || new Date().toISOString()
            };
        } catch (error) {
            console.error('Failed to load working set from localStorage:', error);
            return { logIds: [], logs: {}, timestamp: new Date().toISOString() };
        }
    }

    saveWorkingSetToStorage() {
        try {
            const data = {
                logIds: this.workingSet.logIds,
                timestamp: new Date().toISOString()
            };
            localStorage.setItem('noil_working_set', JSON.stringify(data));
        } catch (error) {
            console.error('Failed to save working set to localStorage:', error);
            if (error.name === 'QuotaExceededError') {
                alert('Warning: Browser storage is full. Working set will only be stored in memory for this session.');
            }
        }
    }

    async addToWorkingSet(logId) {
        if (this.workingSet.logIds.includes(logId)) {
            return; // Already in working set
        }

        this.workingSet.logIds.push(logId);

        // Fetch log details if not already cached
        if (!this.workingSet.logs[logId]) {
            try {
                const log = this.logViewer.logs.find(l => l.id === logId);
                if (log) {
                    this.workingSet.logs[logId] = log;
                }
            } catch (error) {
                console.error('Failed to fetch log details:', error);
            }
        }

        this.saveWorkingSetToStorage();
        this.updateWorkingSetIndicators();
    }

    removeFromWorkingSet(logId) {
        const index = this.workingSet.logIds.indexOf(logId);
        if (index !== -1) {
            this.workingSet.logIds.splice(index, 1);
            delete this.workingSet.logs[logId];
            this.saveWorkingSetToStorage();
            this.updateWorkingSetIndicators();
        }
    }

    clearWorkingSet() {
        this.workingSet.logIds = [];
        this.workingSet.logs = {};
        this.saveWorkingSetToStorage();
        this.updateWorkingSetIndicators();
    }

    getWorkingSet() {
        return this.workingSet;
    }

    isInWorkingSet(logId) {
        return this.workingSet.logIds.includes(logId);
    }

    async addFiberToWorkingSet(fiberId) {
        try {
            // Fetch all logs for this fiber
            const response = await api.getFiberLogs(fiberId, { limit: 10000 });
            const logs = response.logs || [];

            // Add each log to working set
            for (const log of logs) {
                if (!this.workingSet.logIds.includes(log.id)) {
                    this.workingSet.logIds.push(log.id);
                    this.workingSet.logs[log.id] = log;
                }
            }

            this.saveWorkingSetToStorage();
            this.updateWorkingSetIndicators();

            return logs.length;
        } catch (error) {
            console.error('Failed to add fiber to working set:', error);
            throw error;
        }
    }

    async removeFiberFromWorkingSet(fiberId) {
        try {
            // Fetch all logs for this fiber
            const response = await api.getFiberLogs(fiberId, { limit: 10000 });
            const logs = response.logs || [];

            // Remove each log from working set
            for (const log of logs) {
                const index = this.workingSet.logIds.indexOf(log.id);
                if (index !== -1) {
                    this.workingSet.logIds.splice(index, 1);
                    delete this.workingSet.logs[log.id];
                }
            }

            this.saveWorkingSetToStorage();
            this.updateWorkingSetIndicators();

            return logs.length;
        } catch (error) {
            console.error('Failed to remove fiber from working set:', error);
            throw error;
        }
    }

    async isFiberInWorkingSet(fiberId) {
        try {
            // Fetch all logs for this fiber
            const response = await api.getFiberLogs(fiberId, { limit: 10000 });
            const logs = response.logs || [];

            if (logs.length === 0) return false;

            // Check if all logs are in working set
            return logs.every(log => this.workingSet.logIds.includes(log.id));
        } catch (error) {
            console.error('Failed to check fiber in working set:', error);
            return false;
        }
    }

    // =========================================================================
    // Context Menu
    // =========================================================================

    initContextMenu() {
        // Create context menu element
        const menu = document.createElement('div');
        menu.id = 'context-menu';
        menu.className = 'context-menu';
        menu.style.display = 'none';
        menu.innerHTML = `
            <button class="context-menu-item" data-action="add">Add to Working Set</button>
            <button class="context-menu-item" data-action="remove">Remove from Working Set</button>
            <div class="context-menu-separator"></div>
            <button class="context-menu-item" data-action="copy-id">Copy Log ID</button>
            <button class="context-menu-item" data-action="copy-timestamp">Copy Timestamp</button>
        `;
        document.body.appendChild(menu);
        this.contextMenu = menu;

        // Add right-click event listener to log content
        document.getElementById('log-content').addEventListener('contextmenu', (e) => {
            const logLine = e.target.closest('.log-line');
            if (logLine) {
                e.preventDefault();
                this.showContextMenu(e, logLine);
            }
        });

        // Close context menu when clicking outside
        document.addEventListener('click', () => {
            this.hideContextMenu();
        });

        // Handle context menu item clicks
        menu.addEventListener('click', (e) => {
            const item = e.target.closest('.context-menu-item');
            if (!item) return;

            const action = item.getAttribute('data-action');
            const logId = menu.getAttribute('data-log-id');

            switch (action) {
                case 'add':
                    this.addToWorkingSet(logId);
                    break;
                case 'remove':
                    this.removeFromWorkingSet(logId);
                    break;
                case 'copy-id':
                    navigator.clipboard.writeText(logId);
                    break;
                case 'copy-timestamp':
                    const log = this.logViewer.logs.find(l => l.id === logId);
                    if (log) {
                        navigator.clipboard.writeText(log.timestamp);
                    }
                    break;
            }

            this.hideContextMenu();
        });
    }

    showContextMenu(event, logLine) {
        const logId = logLine.getAttribute('data-log-id');
        if (!logId) return;

        const menu = this.contextMenu;
        menu.setAttribute('data-log-id', logId);

        // Show/hide appropriate menu items
        const inWorkingSet = this.isInWorkingSet(logId);
        const addItem = menu.querySelector('[data-action="add"]');
        const removeItem = menu.querySelector('[data-action="remove"]');

        addItem.style.display = inWorkingSet ? 'none' : 'block';
        removeItem.style.display = inWorkingSet ? 'block' : 'none';

        // Position menu near cursor
        let x = event.clientX;
        let y = event.clientY;

        // Show menu temporarily to measure dimensions
        menu.style.display = 'block';
        menu.style.opacity = '0';

        const menuRect = menu.getBoundingClientRect();
        const viewportWidth = window.innerWidth;
        const viewportHeight = window.innerHeight;

        // Adjust position to keep menu on screen
        if (x + menuRect.width > viewportWidth) {
            x = viewportWidth - menuRect.width - 5;
        }
        if (y + menuRect.height > viewportHeight) {
            y = viewportHeight - menuRect.height - 5;
        }

        menu.style.left = `${x}px`;
        menu.style.top = `${y}px`;
        menu.style.opacity = '1';
    }

    hideContextMenu() {
        if (this.contextMenu) {
            this.contextMenu.style.display = 'none';
        }
        if (this.fiberContextMenu) {
            this.fiberContextMenu.style.display = 'none';
        }
    }

    initFiberContextMenu() {
        // Create fiber context menu element
        const menu = document.createElement('div');
        menu.id = 'fiber-context-menu';
        menu.className = 'context-menu';
        menu.style.display = 'none';
        menu.innerHTML = `
            <button class="context-menu-item" data-action="add-fiber">Add Fiber to Working Set</button>
            <button class="context-menu-item" data-action="remove-fiber">Remove Fiber from Working Set</button>
            <div class="context-menu-separator"></div>
            <button class="context-menu-item" data-action="copy-fiber-id">Copy Fiber ID</button>
        `;
        document.body.appendChild(menu);
        this.fiberContextMenu = menu;

        // Handle fiber context menu item clicks
        menu.addEventListener('click', async (e) => {
            const item = e.target.closest('.context-menu-item');
            if (!item) return;

            const action = item.getAttribute('data-action');
            const fiberId = menu.getAttribute('data-fiber-id');

            try {
                switch (action) {
                    case 'add-fiber':
                        const addedCount = await this.addFiberToWorkingSet(fiberId);
                        console.log(`Added ${addedCount} logs from fiber to working set`);
                        break;
                    case 'remove-fiber':
                        const removedCount = await this.removeFiberFromWorkingSet(fiberId);
                        console.log(`Removed ${removedCount} logs from fiber from working set`);
                        break;
                    case 'copy-fiber-id':
                        navigator.clipboard.writeText(fiberId);
                        break;
                }
            } catch (error) {
                console.error('Fiber context menu action failed:', error);
            }

            this.hideContextMenu();
        });
    }

    async showFiberContextMenu(event, fiberId) {
        if (!fiberId) return;

        const menu = this.fiberContextMenu;
        menu.setAttribute('data-fiber-id', fiberId);

        // Check if fiber is in working set
        const inWorkingSet = await this.isFiberInWorkingSet(fiberId);
        const addItem = menu.querySelector('[data-action="add-fiber"]');
        const removeItem = menu.querySelector('[data-action="remove-fiber"]');

        addItem.style.display = inWorkingSet ? 'none' : 'block';
        removeItem.style.display = inWorkingSet ? 'block' : 'none';

        // Position menu near cursor
        let x = event.clientX;
        let y = event.clientY;

        // Show menu temporarily to measure dimensions
        menu.style.display = 'block';
        menu.style.opacity = '0';

        const menuRect = menu.getBoundingClientRect();
        const viewportWidth = window.innerWidth;
        const viewportHeight = window.innerHeight;

        // Adjust position to keep menu on screen
        if (x + menuRect.width > viewportWidth) {
            x = viewportWidth - menuRect.width - 5;
        }
        if (y + menuRect.height > viewportHeight) {
            y = viewportHeight - menuRect.height - 5;
        }

        menu.style.left = `${x}px`;
        menu.style.top = `${y}px`;
        menu.style.opacity = '1';
    }

    // =========================================================================
    // Working Set Visual Indicators
    // =========================================================================

    initWorkingSetIndicators() {
        // This will be called after initial render to add indicators
        // The actual indicators are added in updateWorkingSetIndicators
    }

    updateWorkingSetIndicators() {
        // Add star indicators to log lines in working set
        const logLines = document.querySelectorAll('.log-line');
        logLines.forEach(line => {
            const logId = line.getAttribute('data-log-id');
            if (!logId) return;

            const inWorkingSet = this.isInWorkingSet(logId);

            // Remove existing indicator
            const existingIndicator = line.querySelector('.working-set-indicator');
            if (existingIndicator) {
                existingIndicator.remove();
            }

            // Add indicator if in working set
            if (inWorkingSet) {
                const indicator = document.createElement('span');
                indicator.className = 'working-set-indicator';
                indicator.textContent = '★';
                indicator.title = 'In Working Set';
                line.appendChild(indicator);
                line.classList.add('in-working-set');
            } else {
                line.classList.remove('in-working-set');
            }
        });

        // Update working set panel if it exists
        if (this.fiberProcessingEditorPage && this.fiberProcessingEditorPage.renderWorkingSetPanel) {
            this.fiberProcessingEditorPage.renderWorkingSetPanel();
        }
    }
}

// Start the application when DOM is ready
document.addEventListener('DOMContentLoaded', () => {
    window.app = new NoilApp();
});
