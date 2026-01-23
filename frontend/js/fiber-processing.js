/**
 * Fiber Processing Editor Module
 * Handles fiber type editing with hot-reload and reprocessing
 */
class FiberProcessingEditor {
    constructor(api) {
        this.api = api;
        this.fiberTypes = [];
        this.selectedType = null;
        this.originalYaml = '';
        this.hasUnsavedChanges = false;
        this.reprocessPollInterval = null;
        this.elements = {};
    }

    async init() {
        // Get DOM elements
        this.elements = {
            typeList: document.getElementById('fiber-type-list'),
            typeName: document.getElementById('fiber-type-name'),
            editor: document.getElementById('fiber-yaml-editor'),
            saveBtn: document.getElementById('fiber-save'),
            hotReloadBtn: document.getElementById('fiber-hot-reload'),
            deleteBtn: document.getElementById('fiber-delete'),
            newBtn: document.getElementById('new-fiber-type'),
            status: document.getElementById('fiber-status'),
            reprocessBtn: document.getElementById('start-reprocess'),
            reprocessProgress: document.getElementById('reprocess-progress'),
            reprocessProgressBar: document.getElementById('reprocess-progress-bar'),
            reprocessStatusText: document.getElementById('reprocess-status-text'),
            reprocessProgressText: document.getElementById('reprocess-progress-text'),
            cancelReprocessBtn: document.getElementById('cancel-reprocess'),
        };

        // Initialize editor to empty state (no fiber type selected)
        this.elements.editor.value = '';
        this.elements.editor.disabled = true;

        // Load fiber types
        await this.loadFiberTypes();

        // Setup event listeners
        this.setupEventListeners();

        // Update button states to reflect no selection
        this.updateButtonStates();

        // Check for running reprocess
        this.checkReprocessStatus();
    }

    setupEventListeners() {
        // Save button
        this.elements.saveBtn.addEventListener('click', () => {
            this.saveFiberType();
        });

        // Hot reload button
        this.elements.hotReloadBtn.addEventListener('click', () => {
            this.hotReload();
        });

        // Delete button
        this.elements.deleteBtn.addEventListener('click', () => {
            this.deleteFiberType();
        });

        // New fiber type button
        this.elements.newBtn.addEventListener('click', () => {
            this.showNewFiberTypeDialog();
        });

        // Editor change tracking
        this.elements.editor.addEventListener('input', () => {
            this.hasUnsavedChanges = this.elements.editor.value !== this.originalYaml;

            // Update displayed name if the fiber type name changed in the YAML
            try {
                const yaml = this.elements.editor.value;
                const parsed = jsyaml.load(yaml);
                if (parsed && typeof parsed === 'object') {
                    const keys = Object.keys(parsed);
                    if (keys.length === 1) {
                        const newName = keys[0];
                        if (newName !== this.selectedType) {
                            this.elements.typeName.textContent = `${this.selectedType} → ${newName}`;
                        } else {
                            this.elements.typeName.textContent = this.selectedType;
                        }
                    }
                }
            } catch (e) {
                // Invalid YAML, ignore
            }

            this.updateButtonStates();
        });

        // Reprocess button
        this.elements.reprocessBtn.addEventListener('click', () => {
            this.showReprocessDialog();
        });

        // Cancel reprocess button
        this.elements.cancelReprocessBtn.addEventListener('click', () => {
            this.cancelReprocessing();
        });
    }

    async loadFiberTypes() {
        try {
            this.fiberTypes = await this.api.getAllFiberTypes();
            this.renderTypeList();
        } catch (error) {
            this.showStatus(`Error loading fiber types: ${error.message}`, 'error');
            console.error('Failed to load fiber types:', error);
        }
    }

    renderTypeList() {
        if (!this.elements.typeList) return;

        // Filter out auto-generated source fibers
        const userFiberTypes = this.fiberTypes.filter(ft => !ft.is_source_fiber);

        if (userFiberTypes.length === 0) {
            this.elements.typeList.innerHTML = '<p class="empty-message">No fiber types defined</p>';
            return;
        }

        // Sort alphabetically
        const sorted = [...userFiberTypes].sort((a, b) => a.name.localeCompare(b.name));

        this.elements.typeList.innerHTML = sorted.map(ft => `
            <div class="fiber-type-item ${ft.name === this.selectedType ? 'selected' : ''}"
                 data-name="${this.escapeHtml(ft.name)}">
                <span class="fiber-type-name">${this.escapeHtml(ft.name)}</span>
            </div>
        `).join('');

        // Attach click handlers
        this.elements.typeList.querySelectorAll('.fiber-type-item').forEach(item => {
            item.addEventListener('click', () => {
                const name = item.getAttribute('data-name');
                this.selectFiberType(name);
            });
        });
    }

    async selectFiberType(name) {
        // Check for unsaved changes
        if (this.hasUnsavedChanges) {
            if (!confirm('You have unsaved changes. Discard them?')) {
                return;
            }
        }

        try {
            const data = await this.api.getFiberType(name);
            this.selectedType = name;
            this.originalYaml = data.yaml_content;
            this.hasUnsavedChanges = false;

            // Update UI
            this.elements.typeName.textContent = name;
            this.elements.editor.value = data.yaml_content;
            this.elements.editor.disabled = false;

            // Update selected state in list
            this.elements.typeList.querySelectorAll('.fiber-type-item').forEach(item => {
                item.classList.toggle('selected', item.getAttribute('data-name') === name);
            });

            // Update button states
            this.updateButtonStates();

            // Clear any previous status
            this.clearStatus();
        } catch (error) {
            this.showStatus(`Error loading fiber type: ${error.message}`, 'error');
            console.error('Failed to load fiber type:', error);
        }
    }

    async saveFiberType() {
        if (!this.selectedType) {
            this.showStatus('No fiber type selected', 'error');
            return;
        }

        // Check if it's an auto-generated source fiber
        const fiberType = this.fiberTypes.find(ft => ft.name === this.selectedType);
        if (fiberType?.is_source_fiber) {
            this.showStatus('Cannot edit auto-generated source fiber types', 'error');
            return;
        }

        const yaml = this.elements.editor.value;
        console.log('YAML to save:', yaml);

        // Validate YAML and check for name change
        let newName = this.selectedType;
        try {
            const parsed = jsyaml.load(yaml);
            console.log('Parsed YAML:', parsed);
            console.log('Type:', typeof parsed);

            if (!parsed || typeof parsed !== 'object') {
                this.showStatus('YAML must be an object with a fiber type name', 'error');
                return;
            }
            const keys = Object.keys(parsed);
            console.log('Keys:', keys);

            if (keys.length !== 1) {
                this.showStatus(`YAML must contain exactly one fiber type definition (found ${keys.length}: ${keys.join(', ')})`, 'error');
                return;
            }
            newName = keys[0];
        } catch (error) {
            this.showStatus(`Invalid YAML: ${error.message}`, 'error');
            console.error('YAML parse error:', error);
            return;
        }

        // Check if changed
        if (yaml === this.originalYaml) {
            this.showStatus('No changes to save', 'info');
            return;
        }

        // Confirm rename if name changed
        if (newName !== this.selectedType) {
            const confirmed = confirm(
                `Rename fiber type "${this.selectedType}" to "${newName}"?\n\n` +
                'This will:\n' +
                `- Delete the "${this.selectedType}" fiber type\n` +
                `- Create a new "${newName}" fiber type\n` +
                '- Require Hot Reload to take effect\n\n' +
                'Continue?'
            );
            if (!confirmed) return;
        }

        try {
            this.showStatus('Saving...', 'info');
            const result = await this.api.updateFiberType(this.selectedType, yaml);

            this.originalYaml = yaml;
            this.hasUnsavedChanges = false;

            // Update selected type to new name if renamed
            if (newName !== this.selectedType) {
                this.selectedType = newName;
                this.elements.typeName.textContent = newName;

                // Reload the fiber types list to show the rename
                await this.loadFiberTypes();

                // Select the renamed fiber type in the list
                this.elements.typeList.querySelectorAll('.fiber-type-item').forEach(item => {
                    item.classList.toggle('selected', item.getAttribute('data-name') === newName);
                });

                this.showStatus(
                    `Saved and renamed to "${newName}"! Use Hot Reload to apply changes.`,
                    'success'
                );
            } else {
                // Show success with warning about hot reload
                const warnings = result.warnings && result.warnings.length > 0
                    ? `\nWarnings: ${result.warnings.join(', ')}`
                    : '';
                this.showStatus(`Saved! Use Hot Reload to apply changes.${warnings}`, 'success');
            }

            this.updateButtonStates();
        } catch (error) {
            this.showStatus(`Error saving: ${error.message}`, 'error');
            console.error('Failed to save fiber type:', error);
        }
    }

    async hotReload() {
        if (!this.selectedType) {
            this.showStatus('No fiber type selected', 'error');
            return;
        }

        // Confirm hot reload
        const confirmed = confirm(
            'Hot Reload will:\n' +
            '- Close all open fibers of this type\n' +
            '- Apply new rules to incoming logs\n\n' +
            'Existing fiber data will NOT be reprocessed.\n' +
            'Use "Reprocess" to apply changes to historical logs.\n\n' +
            'Continue?'
        );

        if (!confirmed) return;

        try {
            this.showStatus('Hot reloading...', 'info');
            await this.api.hotReloadFiberType(this.selectedType);

            this.showStatus('Hot reload complete! New logs will use updated rules.', 'success');

            // Refresh fiber types list in case anything changed
            await this.loadFiberTypes();
        } catch (error) {
            if (error.message.includes('409') || error.message.includes('conflict')) {
                this.showStatus('Cannot hot reload while reprocessing is in progress', 'error');
            } else {
                this.showStatus(`Error: ${error.message}`, 'error');
            }
            console.error('Failed to hot reload:', error);
        }
    }

    async deleteFiberType() {
        if (!this.selectedType) {
            this.showStatus('No fiber type selected', 'error');
            return;
        }

        // Check if it's a source fiber
        const fiberType = this.fiberTypes.find(ft => ft.name === this.selectedType);
        if (fiberType?.is_source_fiber) {
            this.showStatus('Cannot delete auto-generated source fiber types', 'error');
            return;
        }

        const confirmed = confirm(
            `Delete fiber type "${this.selectedType}"?\n\n` +
            'This will remove the fiber type from the configuration.\n' +
            'Existing fibers and memberships will NOT be deleted.\n' +
            'You will need to Hot Reload for changes to take effect.'
        );

        if (!confirmed) return;

        try {
            this.showStatus('Deleting...', 'info');
            await this.api.deleteFiberType(this.selectedType);

            this.showStatus('Deleted! Hot Reload to apply changes.', 'success');

            // Clear selection and refresh list
            this.selectedType = null;
            this.originalYaml = '';
            this.hasUnsavedChanges = false;
            this.elements.typeName.textContent = '-';
            this.elements.editor.value = '';
            this.updateButtonStates();

            await this.loadFiberTypes();
        } catch (error) {
            this.showStatus(`Error: ${error.message}`, 'error');
            console.error('Failed to delete fiber type:', error);
        }
    }

    showNewFiberTypeDialog() {
        const name = prompt('Enter new fiber type name:');
        if (!name || !name.trim()) return;

        const trimmedName = name.trim();

        // Check for existing name
        if (this.fiberTypes.some(ft => ft.name === trimmedName)) {
            this.showStatus(`Fiber type "${trimmedName}" already exists`, 'error');
            return;
        }

        // Create default YAML template
        const template = `description: "New fiber type"
temporal:
  max_gap: 5s
  gap_mode: session
attributes:
  - name: example_key
    type: string
    key: true
sources:
  # Add source patterns here
  # example_source:
  #   patterns:
  #     - regex: 'pattern-(?P<example_key>\\w+)'
`;

        this.createFiberType(trimmedName, template);
    }

    async createFiberType(name, yaml) {
        try {
            this.showStatus('Creating...', 'info');
            await this.api.createFiberType(name, yaml);

            this.showStatus(`Created! Use Hot Reload to start using "${name}".`, 'success');

            // Refresh and select the new type
            await this.loadFiberTypes();
            await this.selectFiberType(name);
        } catch (error) {
            this.showStatus(`Error: ${error.message}`, 'error');
            console.error('Failed to create fiber type:', error);
        }
    }

    showReprocessDialog() {
        // Create modal for reprocess options
        const modal = document.createElement('div');
        modal.className = 'reprocess-modal';
        modal.innerHTML = `
            <div class="reprocess-modal-content">
                <h3>Reprocess Historical Logs</h3>
                <p>Re-run fiber processing on stored logs with current rules.</p>

                <div class="reprocess-option">
                    <label>
                        <input type="checkbox" id="reprocess-clear-old" checked>
                        Clear old fiber results before reprocessing
                    </label>
                    <p class="option-help">Recommended. Removes old fibers and memberships first.</p>
                </div>

                <div class="reprocess-option">
                    <label>Time Range (optional):</label>
                    <div class="time-range-inputs">
                        <input type="datetime-local" id="reprocess-start" placeholder="Start">
                        <span>to</span>
                        <input type="datetime-local" id="reprocess-end" placeholder="End">
                    </div>
                    <p class="option-help">Leave empty to reprocess all logs.</p>
                </div>

                <div class="reprocess-modal-actions">
                    <button id="reprocess-cancel-modal" class="btn btn-secondary">Cancel</button>
                    <button id="reprocess-start-modal" class="btn btn-primary">Start Reprocessing</button>
                </div>
            </div>
        `;

        document.body.appendChild(modal);

        // Event handlers
        const closeModal = () => modal.remove();

        modal.querySelector('#reprocess-cancel-modal').addEventListener('click', closeModal);
        modal.addEventListener('click', (e) => {
            if (e.target === modal) closeModal();
        });

        modal.querySelector('#reprocess-start-modal').addEventListener('click', () => {
            const clearOld = document.getElementById('reprocess-clear-old').checked;
            const startInput = document.getElementById('reprocess-start').value;
            const endInput = document.getElementById('reprocess-end').value;

            const options = {
                clear_old_results: clearOld,
            };

            if (startInput || endInput) {
                options.time_range = {};
                if (startInput) options.time_range.start = new Date(startInput).toISOString();
                if (endInput) options.time_range.end = new Date(endInput).toISOString();
            }

            closeModal();
            this.startReprocessing(options);
        });
    }

    async startReprocessing(options) {
        try {
            this.showStatus('Starting reprocessing...', 'info');
            const result = await this.api.startReprocessing(options);

            this.showStatus('Reprocessing started', 'success');

            // Show progress UI
            this.elements.reprocessProgress.style.display = 'block';
            this.elements.reprocessBtn.disabled = true;

            // Start polling for progress
            this.startReprocessPolling();
        } catch (error) {
            if (error.message.includes('409') || error.message.includes('already')) {
                this.showStatus('Reprocessing is already in progress', 'error');
                this.checkReprocessStatus();
            } else {
                this.showStatus(`Error: ${error.message}`, 'error');
            }
            console.error('Failed to start reprocessing:', error);
        }
    }

    startReprocessPolling() {
        // Clear any existing interval
        if (this.reprocessPollInterval) {
            clearInterval(this.reprocessPollInterval);
        }

        // Poll every second
        this.reprocessPollInterval = setInterval(() => {
            this.pollReprocessStatus();
        }, 1000);

        // Also poll immediately
        this.pollReprocessStatus();
    }

    async pollReprocessStatus() {
        try {
            const status = await this.api.getReprocessStatus();

            if (!status.task_id) {
                // No active reprocessing
                this.hideReprocessProgress();
                return;
            }

            this.updateReprocessProgress(status);

            if (status.status === 'Completed') {
                this.showReprocessComplete(status);
            } else if (status.status === 'Failed') {
                this.showReprocessFailed(status.error);
            } else if (status.status === 'Cancelled') {
                this.showStatus('Reprocessing cancelled', 'info');
                this.hideReprocessProgress();
            }
        } catch (error) {
            console.error('Failed to get reprocess status:', error);
        }
    }

    updateReprocessProgress(status) {
        const progress = status.progress;
        const percent = progress.logs_total > 0
            ? Math.round((progress.logs_processed / progress.logs_total) * 100)
            : 0;

        this.elements.reprocessProgressBar.style.width = `${percent}%`;
        this.elements.reprocessStatusText.textContent = `${status.status}...`;
        this.elements.reprocessProgressText.textContent =
            `${progress.logs_processed.toLocaleString()} / ${progress.logs_total.toLocaleString()} logs (${progress.fibers_created} fibers)`;
    }

    showReprocessComplete(status) {
        this.showStatus(
            `Reprocessing complete! ${status.progress.logs_processed.toLocaleString()} logs, ` +
            `${status.progress.fibers_created} fibers created.`,
            'success'
        );
        this.hideReprocessProgress();
    }

    showReprocessFailed(error) {
        this.showStatus(`Reprocessing failed: ${error || 'Unknown error'}`, 'error');
        this.hideReprocessProgress();
    }

    hideReprocessProgress() {
        if (this.reprocessPollInterval) {
            clearInterval(this.reprocessPollInterval);
            this.reprocessPollInterval = null;
        }

        this.elements.reprocessProgress.style.display = 'none';
        this.elements.reprocessBtn.disabled = false;
        this.elements.reprocessProgressBar.style.width = '0%';
    }

    async cancelReprocessing() {
        try {
            await this.api.cancelReprocessing();
            this.showStatus('Cancellation requested...', 'info');
        } catch (error) {
            this.showStatus(`Error: ${error.message}`, 'error');
            console.error('Failed to cancel reprocessing:', error);
        }
    }

    async checkReprocessStatus() {
        try {
            const status = await this.api.getReprocessStatus();

            if (status.task_id && status.status === 'Running') {
                // Reprocessing is active, show progress and start polling
                this.elements.reprocessProgress.style.display = 'block';
                this.elements.reprocessBtn.disabled = true;
                this.startReprocessPolling();
            }
        } catch (error) {
            // Ignore errors on initial check
            console.log('No active reprocessing');
        }
    }

    updateButtonStates() {
        const hasSelection = !!this.selectedType;
        const fiberType = this.fiberTypes.find(ft => ft.name === this.selectedType);
        const isSourceFiber = fiberType?.is_source_fiber;

        this.elements.saveBtn.disabled = !hasSelection;
        this.elements.hotReloadBtn.disabled = !hasSelection;
        this.elements.deleteBtn.disabled = !hasSelection || isSourceFiber;
        this.elements.editor.disabled = !hasSelection;

        // Hot reload button style - highlight if there are saved changes
        if (hasSelection && !this.hasUnsavedChanges) {
            this.elements.hotReloadBtn.classList.remove('btn-success-muted');
        } else {
            this.elements.hotReloadBtn.classList.add('btn-success-muted');
        }
    }

    showStatus(message, type) {
        this.elements.status.textContent = message;
        this.elements.status.className = `fiber-status fiber-status-${type}`;
        this.elements.status.style.display = 'block';

        // Auto-hide success/info messages after 5 seconds
        if (type === 'success' || type === 'info') {
            setTimeout(() => {
                if (this.elements.status.textContent === message) {
                    this.clearStatus();
                }
            }, 5000);
        }
    }

    clearStatus() {
        this.elements.status.textContent = '';
        this.elements.status.className = 'fiber-status';
        this.elements.status.style.display = 'none';
    }

    escapeHtml(text) {
        const div = document.createElement('div');
        div.textContent = text;
        return div.innerHTML;
    }
}

// Export for use by app.js
let fiberProcessingEditor = null;
