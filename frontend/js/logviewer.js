/**
 * Log viewer component
 */
class LogViewer {
    constructor(containerEl, titleEl, infoEl) {
        this.container = containerEl;
        this.titleEl = titleEl;
        this.infoEl = infoEl;
        this.currentFiber = null;
        this.logs = [];
        this.offset = 0;
        this.limit = 100;
        this.hasMore = false;
    }

    async loadFiber(fiberId) {
        try {
            // Show loading state
            this.showLoading();

            // Fetch fiber details
            const fiber = await api.getFiber(fiberId);
            this.currentFiber = fiber;

            // Fetch logs for this fiber
            const response = await api.getFiberLogs(fiberId, {
                limit: this.limit,
                offset: 0,
            });

            this.logs = response.logs;
            this.offset = response.logs.length;
            this.hasMore = response.logs.length >= this.limit;

            // Update UI
            this.updateHeader();
            this.render();
        } catch (error) {
            console.error('Failed to load fiber:', error);
            this.showError('Failed to load fiber logs');
        }
    }

    async loadMore() {
        if (!this.currentFiber || !this.hasMore) return;

        try {
            const response = await api.getFiberLogs(this.currentFiber.id, {
                limit: this.limit,
                offset: this.offset,
            });

            this.logs.push(...response.logs);
            this.offset += response.logs.length;
            this.hasMore = response.logs.length >= this.limit;

            this.render();
        } catch (error) {
            console.error('Failed to load more logs:', error);
        }
    }

    updateHeader() {
        if (!this.currentFiber) {
            this.titleEl.textContent = 'Select a fiber to view logs';
            this.infoEl.textContent = '';
            return;
        }

        this.titleEl.textContent = `Fiber: ${this.currentFiber.fiber_type}`;

        const logCount = this.logs.length;
        const status = this.currentFiber.closed ? 'Closed' : 'Open';
        const start = new Date(this.currentFiber.first_activity).toLocaleString();
        const end = new Date(this.currentFiber.last_activity).toLocaleString();

        this.infoEl.innerHTML = `
            <span>${logCount} log${logCount !== 1 ? 's' : ''}</span>
            <span style="margin-left: 15px;">${status}</span>
            <span style="margin-left: 15px;">ID: ${this.currentFiber.id}</span>
        `;
    }

    render() {
        if (this.logs.length === 0) {
            this.container.innerHTML = `
                <div class="empty-state">
                    <div class="empty-state-icon">📝</div>
                    <div class="empty-state-text">No logs found for this fiber</div>
                </div>
            `;
            return;
        }

        // Sort logs by timestamp
        const sortedLogs = [...this.logs].sort((a, b) => {
            return new Date(a.timestamp) - new Date(b.timestamp);
        });

        this.container.innerHTML = '';

        sortedLogs.forEach(log => {
            const logLine = this.createLogLine(log);
            this.container.appendChild(logLine);
        });

        // Update load more button
        const loadMoreBtn = document.getElementById('load-more-logs');
        if (loadMoreBtn) {
            loadMoreBtn.style.display = this.hasMore ? 'inline-block' : 'none';
        }
    }

    createLogLine(log) {
        const line = document.createElement('div');
        line.className = 'log-line';

        // Apply source color as background
        const sourceColor = colorManager.getSourceColor(log.source_id);
        line.style.backgroundColor = this.adjustColorOpacity(sourceColor, 0.15);
        line.style.borderLeftColor = sourceColor;

        // Format timestamp
        const timestamp = new Date(log.timestamp);
        const timestampStr = this.formatTimestamp(timestamp);

        // Create line content
        const timestampSpan = document.createElement('span');
        timestampSpan.className = 'timestamp';
        timestampSpan.textContent = timestampStr;

        const sourceSpan = document.createElement('span');
        sourceSpan.className = 'source';
        sourceSpan.textContent = `[${log.source_id}]`;

        const textSpan = document.createElement('span');
        textSpan.textContent = log.raw_text;

        line.appendChild(timestampSpan);
        line.appendChild(sourceSpan);
        line.appendChild(textSpan);

        // Add tooltip with full info
        line.title = `ID: ${log.id}\nSource: ${log.source_id}\nTime: ${timestamp.toLocaleString()}`;

        return line;
    }

    formatTimestamp(date) {
        const hours = String(date.getHours()).padStart(2, '0');
        const minutes = String(date.getMinutes()).padStart(2, '0');
        const seconds = String(date.getSeconds()).padStart(2, '0');
        const ms = String(date.getMilliseconds()).padStart(3, '0');
        return `${hours}:${minutes}:${seconds}.${ms}`;
    }

    adjustColorOpacity(color, opacity) {
        // Convert color to rgba with specified opacity
        if (color.startsWith('#')) {
            const r = parseInt(color.slice(1, 3), 16);
            const g = parseInt(color.slice(3, 5), 16);
            const b = parseInt(color.slice(5, 7), 16);
            return `rgba(${r}, ${g}, ${b}, ${opacity})`;
        } else if (color.startsWith('hsl')) {
            // Convert HSL to HSLA
            return color.replace('hsl', 'hsla').replace(')', `, ${opacity})`);
        }
        return color;
    }

    showLoading() {
        this.container.innerHTML = `
            <div class="empty-state">
                <div class="empty-state-icon">⏳</div>
                <div class="empty-state-text">Loading logs...</div>
            </div>
        `;
    }

    showError(message) {
        this.container.innerHTML = `
            <div class="empty-state">
                <div class="empty-state-icon">❌</div>
                <div class="empty-state-text">${message}</div>
            </div>
        `;
    }
}
