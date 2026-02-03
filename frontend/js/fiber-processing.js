/**
 * Fiber Processing Editor Module
 * Handles fiber type editing with activation and reprocessing
 */
class FiberProcessingEditor {
    constructor(api, elementIds = {}) {
        this.api = api;
        this.fiberTypes = [];
        this.selectedType = null;
        this.originalYaml = '';
        this.hasUnsavedChanges = false;
        this.reprocessPollInterval = null;
        this.elements = {};
        this.editorView = null; // CodeMirror editor instance
        this.editableCompartment = null; // Compartment for dynamic reconfiguration
        this.currentConfigVersion = null; // Current active config version
        this.selectedVersionHash = null; // Selected version in history modal
        this.workingSetViewMode = 'pretty'; // 'pretty' or 'raw'
        this.workingSetViewStorageKey = 'noil_working_set_view_mode';
        this.elementIds = {
            typeListEl: 'fiber-type-list',
            typeNameEl: 'fiber-type-name',
            editorEl: 'fiber-yaml-editor',
            saveBtnEl: 'fiber-save',
            activateBtnEl: 'fiber-hot-reload',
            deleteBtnEl: 'fiber-delete',
            newBtnEl: 'new-fiber-type',
            statusEl: 'fiber-status',
            reprocessBtnEl: 'start-reprocess',
            reprocessProgressEl: 'reprocess-progress',
            reprocessProgressBarEl: 'reprocess-progress-bar',
            reprocessStatusTextEl: 'reprocess-status-text',
            reprocessProgressTextEl: 'reprocess-progress-text',
            cancelReprocessBtnEl: 'cancel-reprocess',
            configVersionIndicatorEl: 'fiber-config-version-indicator',
            configVersionHashEl: 'fiber-config-version-hash',
            configVersionStatusEl: 'fiber-config-version-status',
            ...elementIds
        };
    }

    async init() {
        // Get DOM elements
        this.elements = {
            typeList: document.getElementById(this.elementIds.typeListEl),
            typeName: document.getElementById(this.elementIds.typeNameEl),
            editorContainer: document.getElementById(this.elementIds.editorEl),
            saveBtn: document.getElementById(this.elementIds.saveBtnEl),
            activateBtn: document.getElementById(this.elementIds.activateBtnEl),
            deleteBtn: document.getElementById(this.elementIds.deleteBtnEl),
            newBtn: document.getElementById(this.elementIds.newBtnEl),
            status: document.getElementById(this.elementIds.statusEl),
            reprocessBtn: document.getElementById(this.elementIds.reprocessBtnEl),
            reprocessProgress: document.getElementById(this.elementIds.reprocessProgressEl),
            reprocessProgressBar: document.getElementById(this.elementIds.reprocessProgressBarEl),
            reprocessStatusText: document.getElementById(this.elementIds.reprocessStatusTextEl),
            reprocessProgressText: document.getElementById(this.elementIds.reprocessProgressTextEl),
            cancelReprocessBtn: document.getElementById(this.elementIds.cancelReprocessBtnEl),
            configVersionIndicator: document.getElementById(this.elementIds.configVersionIndicatorEl),
            configVersionHash: document.getElementById(this.elementIds.configVersionHashEl),
            configVersionStatus: document.getElementById(this.elementIds.configVersionStatusEl),
        };

        // Initialize CodeMirror editor
        try {
            await this.initCodeMirror();
        } catch (error) {
            console.warn('Failed to initialize CodeMirror, falling back to textarea:', error);
            // Fall back to using the textarea directly
            this.useFallbackEditor();
        }

        // Load current config version
        await this.loadConfigVersion();

        // Load fiber types
        await this.loadFiberTypes();

        // Setup event listeners
        this.setupEventListeners();

        // Initialize working set panel
        this.initWorkingSetPanel();

        // Update button states to reflect no selection
        this.updateButtonStates();

        // Check for running reprocess
        this.checkReprocessStatus();
    }

    useFallbackEditor() {
        // If CodeMirror fails, just use the textarea
        const textarea = this.elements.editorContainer;
        if (textarea) {
            textarea.style.display = 'block';
            textarea.disabled = true;
            textarea.value = '';
            // Set up editor wrapper functions to use textarea
            this.editorView = null;
        }
    }

    async initCodeMirror() {
        // Wait for CodeMirror to be loaded (with timeout)
        if (!window.CodeMirror && !window.CodeMirrorLoadError) {
            // Wait for the codemirror-ready event or error
            await new Promise((resolve, reject) => {
                const timeout = setTimeout(() => {
                    reject(new Error('CodeMirror failed to load (timeout)'));
                }, 10000); // 10 second timeout

                const onReady = () => {
                    clearTimeout(timeout);
                    window.removeEventListener('codemirror-ready', onReady);
                    resolve();
                };

                const checkError = setInterval(() => {
                    if (window.CodeMirrorLoadError) {
                        clearTimeout(timeout);
                        clearInterval(checkError);
                        window.removeEventListener('codemirror-ready', onReady);
                        reject(window.CodeMirrorLoadError);
                    }
                }, 100);

                window.addEventListener('codemirror-ready', onReady);

                // Check if it's already loaded
                if (window.CodeMirror) {
                    clearTimeout(timeout);
                    clearInterval(checkError);
                    window.removeEventListener('codemirror-ready', onReady);
                    resolve();
                }
            });
        }

        if (window.CodeMirrorLoadError) {
            throw new Error('CodeMirror failed to load: ' + window.CodeMirrorLoadError.message);
        }

        if (!window.CodeMirror) {
            throw new Error('CodeMirror is not available');
        }

        const { EditorView, EditorState, Compartment, basicSetup, yaml, keymap, indentWithTab } = window.CodeMirror;

        // Verify all required components are present
        if (!EditorView || !EditorState || !Compartment || !basicSetup || !yaml || !keymap || !indentWithTab) {
            throw new Error('CodeMirror components are incomplete');
        }

        // Create compartment for editable state (allows dynamic reconfiguration)
        this.editableCompartment = new Compartment();

        // Create editor state with YAML syntax highlighting
        const startState = EditorState.create({
            doc: '',
            extensions: [
                basicSetup,
                yaml,
                EditorView.updateListener.of((update) => {
                    if (update.docChanged) {
                        this.handleEditorChange();
                    }
                }),
                EditorState.tabSize.of(2),
                keymap.of([indentWithTab]),
                this.editableCompartment.of(EditorView.editable.of(false)), // Start disabled
            ]
        });

        // Replace textarea with CodeMirror
        const textarea = this.elements.editorContainer;
        if (!textarea) {
            throw new Error('Editor textarea element not found');
        }

        // Hide textarea but keep it in DOM for reference
        textarea.style.display = 'none';

        // Create a wrapper div for CodeMirror to maintain proper flex layout
        const editorWrapper = document.createElement('div');
        editorWrapper.className = 'cm-editor-wrapper';

        // Insert wrapper right after textarea
        textarea.parentElement.insertBefore(editorWrapper, textarea.nextSibling);

        // Create CodeMirror editor inside the wrapper
        this.editorView = new EditorView({
            state: startState,
            parent: editorWrapper
        });

        if (!this.editorView || !this.editorView.dom) {
            throw new Error('Failed to create CodeMirror editor');
        }

        // Add placeholder attribute
        this.editorView.dom.setAttribute('data-placeholder', 'Select a fiber type to edit...');

        // Set initial disabled styling
        this.setEditorEnabled(false);
    }

    handleEditorChange() {
        const yaml = this.getEditorValue();
        // Trim both values to avoid false positives from trailing whitespace differences
        this.hasUnsavedChanges = yaml.trim() !== this.originalYaml.trim();

        // Update displayed name if the fiber type name changed in the YAML
        try {
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

        // Update config version indicator to show unsaved changes
        this.updateConfigVersionIndicator();
        this.updateButtonStates();
    }

    setEditorValue(value) {
        if (this.editorView) {
            // CodeMirror mode
            this.editorView.dispatch({
                changes: {
                    from: 0,
                    to: this.editorView.state.doc.length,
                    insert: value
                }
            });
        } else if (this.elements.editorContainer) {
            // Fallback textarea mode
            this.elements.editorContainer.value = value;
        }
    }

    getEditorValue() {
        if (this.editorView) {
            // CodeMirror mode
            return this.editorView.state.doc.toString();
        } else if (this.elements.editorContainer) {
            // Fallback textarea mode
            return this.elements.editorContainer.value;
        }
        return '';
    }

    setEditorEnabled(enabled) {
        if (this.editorView && this.editableCompartment && window.CodeMirror) {
            // CodeMirror mode
            const { EditorView } = window.CodeMirror;

            // Reconfigure the editable state using the compartment
            this.editorView.dispatch({
                effects: this.editableCompartment.reconfigure(EditorView.editable.of(enabled))
            });

            // Update styling to show disabled state
            if (enabled) {
                this.editorView.dom.style.opacity = '1';
                this.editorView.dom.style.cursor = 'text';
            } else {
                this.editorView.dom.style.opacity = '0.6';
                this.editorView.dom.style.cursor = 'not-allowed';
            }
        } else if (this.elements.editorContainer) {
            // Fallback textarea mode
            this.elements.editorContainer.disabled = !enabled;
        }
    }

    setupEventListeners() {
        // Save button
        this.elements.saveBtn.addEventListener('click', () => {
            this.saveFiberType();
        });

        // Activate button
        this.elements.activateBtn.addEventListener('click', () => {
            this.activateFiberType();
        });

        // Delete button
        this.elements.deleteBtn.addEventListener('click', () => {
            this.deleteFiberType();
        });

        // New fiber type button
        this.elements.newBtn.addEventListener('click', () => {
            this.showNewFiberTypeDialog();
        });

        // Reprocess button
        this.elements.reprocessBtn.addEventListener('click', () => {
            this.showReprocessDialog();
        });

        // Cancel reprocess button
        this.elements.cancelReprocessBtn.addEventListener('click', () => {
            this.cancelReprocessing();
        });

        // Config version indicator click handler
        if (this.elements.configVersionIndicator) {
            this.elements.configVersionIndicator.addEventListener('click', () => {
                this.showConfigHistory();
            });
        }

        // Editor change tracking for fallback textarea mode
        if (!this.editorView && this.elements.editorContainer) {
            this.elements.editorContainer.addEventListener('input', () => {
                this.handleEditorChange();
            });
        }
    }

    async loadConfigVersion() {
        try {
            const config = await this.api.getCurrentConfig();
            this.currentConfigVersion = config;
            this.updateConfigVersionIndicator();
        } catch (error) {
            console.error('Failed to load config version:', error);
            // Don't show error to user - this is background info
        }
    }

    updateConfigVersionIndicator() {
        if (!this.currentConfigVersion || !this.elements.configVersionHash || !this.elements.configVersionStatus) {
            return;
        }

        const hashShort = this.currentConfigVersion.version_hash.substring(0, 8);

        // Show unsaved changes indicator if content differs from loaded version
        const currentYaml = this.getEditorValue();
        const hasChanges = this.selectedType && currentYaml.trim() !== this.originalYaml.trim();
        const unsavedMarker = hasChanges ? ' *' : '';

        // Show active/inactive status
        const status = this.currentConfigVersion.is_active ? ' (active)' : ' (inactive)';
        const statusColor = this.currentConfigVersion.is_active ? '#4caf50' : '#ff9800';

        this.elements.configVersionHash.textContent = hashShort + unsavedMarker;
        this.elements.configVersionStatus.textContent = status;
        this.elements.configVersionStatus.style.color = statusColor;
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
        if (!this.elements.typeList) {
            console.error('typeList element not found!');
            return;
        }

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
            this.setEditorValue(data.yaml_content);
            this.setEditorEnabled(true);

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

        const yaml = this.getEditorValue();

        // Validate YAML and check for name change
        let newName = this.selectedType;
        try {
            const parsed = jsyaml.load(yaml);

            if (!parsed || typeof parsed !== 'object') {
                this.showStatus('YAML must be an object with a fiber type name', 'error');
                return;
            }
            const keys = Object.keys(parsed);

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

        // Check if changed (trim to avoid false positives from whitespace)
        if (yaml.trim() === this.originalYaml.trim()) {
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
                '- Require Activate to take effect\n\n' +
                'Continue?'
            );
            if (!confirmed) return;
        }

        try {
            this.showStatus('Saving...', 'info');
            const result = await this.api.updateFiberType(this.selectedType, yaml);

            this.originalYaml = yaml;
            this.hasUnsavedChanges = false;

            // Update current config version to the newly saved version
            // The save created a new config version (inactive), so we need to show that
            this.currentConfigVersion = {
                version_hash: result.new_version_hash,
                is_active: false,
            };
            this.updateConfigVersionIndicator();

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
                    `Saved and renamed to "${newName}"! Use Activate to apply changes.`,
                    'success'
                );
            } else {
                // Show success with warning about activate
                const warnings = result.validation_warnings && result.validation_warnings.length > 0
                    ? `\nWarnings: ${result.validation_warnings.join(', ')}`
                    : '';
                this.showStatus(`Saved! Use Activate to apply changes.${warnings}`, 'success');
            }

            this.updateButtonStates();
        } catch (error) {
            this.showStatus(`Error saving: ${error.message}`, 'error');
            console.error('Failed to save fiber type:', error);
        }
    }

    async activateFiberType() {
        if (!this.selectedType) {
            this.showStatus('No fiber type selected', 'error');
            return;
        }

        try {
            this.showStatus('Activating...', 'info');
            await this.api.activateFiberType(this.selectedType);

            this.showStatus('Activation complete! New logs will use updated rules.', 'success');

            // Reload config version to update the indicator (the version is now active)
            await this.loadConfigVersion();
            this.updateConfigVersionIndicator();

            // Refresh fiber types list in case anything changed
            await this.loadFiberTypes();
        } catch (error) {
            if (error.message.includes('409') || error.message.includes('conflict')) {
                this.showStatus('Cannot activate while reprocessing is in progress', 'error');
            } else {
                this.showStatus(`Error: ${error.message}`, 'error');
            }
            console.error('Failed to activate:', error);
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
            'You will need to Activate for changes to take effect.'
        );

        if (!confirmed) return;

        try {
            this.showStatus('Deleting...', 'info');
            await this.api.deleteFiberType(this.selectedType);

            this.showStatus('Deleted! Activate to apply changes.', 'success');

            // Clear selection and refresh list
            this.selectedType = null;
            this.originalYaml = '';
            this.hasUnsavedChanges = false;
            this.elements.typeName.textContent = '-';
            this.setEditorValue('');
            this.setEditorEnabled(false);
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

            this.showStatus(`Created! Use Activate to start using "${name}".`, 'success');

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
                    <p class="option-help">Leave empty to reprocess all logs.</p>
                    <div class="time-range-inputs">
                        <div class="time-range-input-group">
                            <label for="reprocess-start">From:</label>
                            <input type="datetime-local" id="reprocess-start">
                        </div>
                        <div class="time-range-input-group">
                            <label for="reprocess-end">To:</label>
                            <input type="datetime-local" id="reprocess-end">
                        </div>
                    </div>
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

            if (window.app && typeof window.app.invalidateFiberCache === 'function') {
                window.app.invalidateFiberCache();
            }

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

        // Check if currently viewed version is already active
        const isCurrentlyActive = this.currentConfigVersion && this.currentConfigVersion.is_active;

        // Save button: disabled if no selection OR no unsaved changes OR source fiber
        this.elements.saveBtn.disabled = !hasSelection || !this.hasUnsavedChanges || isSourceFiber;

        // Activate button: disabled if no selection OR has unsaved changes (must save first) OR already active
        this.elements.activateBtn.disabled = !hasSelection || this.hasUnsavedChanges || isCurrentlyActive;

        // Delete button: disabled if no selection OR source fiber
        this.elements.deleteBtn.disabled = !hasSelection || isSourceFiber;

        // Editor: enabled if has selection
        this.setEditorEnabled(hasSelection);

        // Activate button style - highlight if ready to activate (saved changes exist)
        // Gray out if unsaved changes exist or already active
        if (hasSelection && !this.hasUnsavedChanges && !isCurrentlyActive) {
            this.elements.activateBtn.classList.remove('btn-success-muted');
        } else {
            this.elements.activateBtn.classList.add('btn-success-muted');
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

    // =========================================================================
    // Working Set Panel
    // =========================================================================

    initWorkingSetPanel() {
        // Get working set panel elements
        const clearBtn = document.getElementById('working-set-clear');
        const testBtn = document.getElementById('working-set-test');
        const viewToggleBtn = document.getElementById('working-set-view-toggle');

        if (clearBtn) {
            clearBtn.addEventListener('click', () => {
                if (window.app) {
                    window.app.clearWorkingSet();
                }
            });
        }

        if (testBtn) {
            testBtn.addEventListener('click', () => {
                this.testWorkingSet();
            });
        }

        if (viewToggleBtn) {
            const storedViewMode = this.loadWorkingSetViewMode();
            if (storedViewMode) {
                this.workingSetViewMode = storedViewMode;
                viewToggleBtn.checked = storedViewMode === 'raw';
            } else {
                this.workingSetViewMode = viewToggleBtn.checked ? 'raw' : 'pretty';
            }
            viewToggleBtn.addEventListener('change', () => {
                this.workingSetViewMode = viewToggleBtn.checked ? 'raw' : 'pretty';
                this.saveWorkingSetViewMode(this.workingSetViewMode);
                this.renderWorkingSetPanel();
            });
        }

        // Initial render
        this.renderWorkingSetPanel();
    }

    loadWorkingSetViewMode() {
        try {
            const stored = localStorage.getItem(this.workingSetViewStorageKey);
            if (stored === 'raw' || stored === 'pretty') {
                return stored;
            }
        } catch (error) {
            console.error('Failed to load working set view mode from localStorage:', error);
        }
        return null;
    }

    saveWorkingSetViewMode(mode) {
        try {
            localStorage.setItem(this.workingSetViewStorageKey, mode);
        } catch (error) {
            console.error('Failed to save working set view mode to localStorage:', error);
        }
    }

    async renderWorkingSetPanel() {
        const countEl = document.getElementById('working-set-count');
        const contentEl = document.getElementById('working-set-content');

        if (!window.app || !countEl || !contentEl) return;

        // Toggle raw mode class on container
        contentEl.classList.toggle('raw-mode', this.workingSetViewMode === 'raw');

        const workingSet = window.app.getWorkingSet();
        const logIds = workingSet.logIds;

        // Update count
        countEl.textContent = logIds.length;

        // If empty, show empty state
        if (logIds.length === 0) {
            contentEl.innerHTML = `
                <div class="working-set-empty">
                    <p>No logs in working set.</p>
                    <p>Right-click logs in the Log View to add them.</p>
                </div>
            `;
            return;
        }

        // Render log items
        contentEl.innerHTML = '';

        const batchSize = 100;
        const logIdList = logIds.slice();
        const logsById = {};
        const fibersByLogId = {};
        const missingIds = new Set();

        for (let i = 0; i < logIdList.length; i += batchSize) {
            const batch = logIdList.slice(i, i + batchSize);
            try {
                const response = await this.api.getLogsBatch(batch, { includeFiberMembership: true });
                (response.logs || []).forEach((log) => {
                    logsById[log.id] = log;
                    workingSet.logs[log.id] = log;
                });
                if (response.fiber_memberships) {
                    Object.entries(response.fiber_memberships).forEach(([logId, fibers]) => {
                        fibersByLogId[logId] = fibers;
                    });
                }
                (response.missing_log_ids || []).forEach((logId) => {
                    missingIds.add(logId);
                });
            } catch (error) {
                console.error('Failed to fetch working set batch:', error);
            }
        }

        if (missingIds.size > 0) {
            missingIds.forEach((logId) => {
                window.app.removeFromWorkingSet(logId);
            });
            return;
        }

        if (workingSet.logIds.length === 0) {
            contentEl.innerHTML = `
                <div class="working-set-empty">
                    <p>No logs in working set.</p>
                    <p>Right-click logs in the Log View to add them.</p>
                </div>
            `;
            return;
        }

        // Sort logIds by timestamp before rendering
        const sortedLogIds = workingSet.logIds.slice().sort((idA, idB) => {
            const logA = logsById[idA] || workingSet.logs[idA];
            const logB = logsById[idB] || workingSet.logs[idB];

            // Handle missing logs (should be rare due to cleanup at line 1040-1044)
            if (!logA) return 1;  // Push missing logs to end
            if (!logB) return -1;

            // Compare timestamps (convert to milliseconds for numeric comparison)
            const tsA = new Date(logA.timestamp).getTime();
            const tsB = new Date(logB.timestamp).getTime();
            return tsA - tsB;
        });

        for (const logId of sortedLogIds) {
            const log = logsById[logId] || workingSet.logs[logId];
            if (!log) {
                continue;
            }
            const fibers = fibersByLogId[logId] || [];
            const logItem = this.createWorkingSetLogItem(log, fibers);
            contentEl.appendChild(logItem);
        }
    }

    createWorkingSetLogItem(log, fibers) {
        const item = document.createElement('div');
        item.setAttribute('data-log-id', log.id);

        // Format timestamp
        const timestamp = new Date(log.timestamp);
        const year = timestamp.getFullYear();
        const month = String(timestamp.getMonth() + 1).padStart(2, '0');
        const day = String(timestamp.getDate()).padStart(2, '0');
        const hours = String(timestamp.getHours()).padStart(2, '0');
        const minutes = String(timestamp.getMinutes()).padStart(2, '0');
        const seconds = String(timestamp.getSeconds()).padStart(2, '0');
        const ms = String(timestamp.getMilliseconds()).padStart(3, '0');
        const timeStr = `${year}-${month}-${day} ${hours}:${minutes}:${seconds}.${ms}`;

        // Source color
        const sourceColor = colorManager.getFiberTypeColor(log.source_id);

        if (this.workingSetViewMode === 'raw') {
            // Raw mode: exact same structure as Log View
            item.className = 'log-line working-set-raw-item';
            item.style.backgroundColor = this.adjustColorOpacity(sourceColor, 0.15);
            item.style.setProperty('--log-line-color', sourceColor);

            const timestampSpan = document.createElement('span');
            timestampSpan.className = 'timestamp';
            timestampSpan.textContent = timeStr;

            const sourceSpan = document.createElement('span');
            sourceSpan.className = 'source';
            sourceSpan.textContent = `[${log.source_id}]`;

            const textSpan = document.createElement('span');
            textSpan.textContent = log.raw_text;

            const removeBtn = document.createElement('button');
            removeBtn.className = 'working-set-raw-remove';
            removeBtn.title = 'Remove from working set';
            removeBtn.textContent = '\u00d7';
            removeBtn.addEventListener('click', (e) => {
                e.stopPropagation();
                if (window.app) {
                    window.app.removeFromWorkingSet(log.id);
                }
            });

            item.appendChild(timestampSpan);
            item.appendChild(sourceSpan);
            item.appendChild(textSpan);
            item.appendChild(removeBtn);
        } else {
            // Pretty mode: card layout with truncated text
            item.className = 'working-set-log-item';

            // Build fiber badges
            let fiberBadgesHtml = '';
            if (fibers && fibers.length > 0) {
                fiberBadgesHtml = '<div class="working-set-log-fibers">';
                fibers.forEach(fiber => {
                    const fiberColor = colorManager.getFiberTypeColor(fiber.fiber_type);
                    fiberBadgesHtml += `
                        <span class="working-set-fiber-badge" style="background-color: ${fiberColor};">
                            ${this.escapeHtml(fiber.fiber_type)}
                        </span>
                    `;
                });
                fiberBadgesHtml += '</div>';
            }

            const truncatedText = log.raw_text.length > 50
                ? log.raw_text.substring(0, 50) + '...'
                : log.raw_text;

            item.innerHTML = `
                <div class="working-set-log-timestamp">
                    ${timeStr}
                    <span class="working-set-log-source" style="background-color: ${sourceColor};">
                        ${this.escapeHtml(log.source_id)}
                    </span>
                </div>
                <div class="working-set-log-text" title="${this.escapeHtml(log.raw_text)}">
                    ${this.escapeHtml(truncatedText)}
                </div>
                ${fiberBadgesHtml}
                <button class="working-set-log-remove" title="Remove from working set">&times;</button>
            `;

            // Add remove button handler
            const removeBtn = item.querySelector('.working-set-log-remove');
            if (removeBtn) {
                removeBtn.addEventListener('click', (e) => {
                    e.stopPropagation();
                    if (window.app) {
                        window.app.removeFromWorkingSet(log.id);
                    }
                });
            }
        }

        return item;
    }

    adjustColorOpacity(color, opacity) {
        // Convert color to rgba with specified opacity
        if (color.startsWith('#')) {
            const r = parseInt(color.slice(1, 3), 16);
            const g = parseInt(color.slice(3, 5), 16);
            const b = parseInt(color.slice(5, 7), 16);
            return `rgba(${r}, ${g}, ${b}, ${opacity})`;
        } else if (color.startsWith('hsl')) {
            return color.replace('hsl', 'hsla').replace(')', `, ${opacity})`);
        }
        return color;
    }

    async testWorkingSet() {
        if (!this.selectedType) {
            this.showStatus('Please select a fiber type first', 'error');
            return;
        }

        if (!window.app) {
            this.showStatus('Application not initialized', 'error');
            return;
        }

        const workingSet = window.app.getWorkingSet();
        if (workingSet.logIds.length === 0) {
            this.showStatus('Working set is empty. Add logs first.', 'error');
            return;
        }

        // Get current YAML content from editor
        const yamlContent = this.getEditorValue();

        // Validate YAML
        try {
            const parsed = jsyaml.load(yamlContent);
            if (!parsed || typeof parsed !== 'object') {
                this.showStatus('Invalid YAML: must be an object', 'error');
                return;
            }
            const keys = Object.keys(parsed);
            if (keys.length !== 1) {
                this.showStatus('Invalid YAML: must contain exactly one fiber type definition', 'error');
                return;
            }
        } catch (error) {
            this.showStatus(`Invalid YAML: ${error.message}`, 'error');
            return;
        }

        // Show loading
        this.showStatus('Testing working set...', 'info');

        try {
            // Call backend API to test working set
            const result = await this.api.testWorkingSet(this.selectedType, workingSet.logIds, yamlContent);

            // Show results modal
            this.showTestResults(result);

            this.showStatus('Test complete', 'success');
        } catch (error) {
            this.showStatus(`Test failed: ${error.message}`, 'error');
            console.error('Failed to test working set:', error);
        }
    }

    showTestResults(result) {
        const modal = document.getElementById('test-results-modal');
        const content = document.getElementById('test-results-content');

        if (!modal || !content) {
            console.error('Test results modal not found');
            return;
        }

        // Determine status
        let statusHtml = '';
        const bestMatch = result.fibers_generated[result.best_match_index];

        if (bestMatch && bestMatch.iou === 1.0) {
            statusHtml = `
                <div class="test-result-status test-result-status-success">
                    <span class="test-result-status-icon">✓</span>
                    <div>
                        <div>Perfect Match</div>
                        <div style="font-size: 13px; font-weight: normal; margin-top: 4px;">
                            Found fiber with exact match: ${result.expected_logs.length}/${result.expected_logs.length} logs
                        </div>
                        <div style="font-size: 12px; font-weight: normal; margin-top: 2px;">
                            Fiber ID: ${bestMatch.fiber_id}
                        </div>
                    </div>
                </div>
            `;
        } else if (bestMatch && bestMatch.iou > 0) {
            statusHtml = `
                <div class="test-result-status test-result-status-warning">
                    <span class="test-result-status-icon">⚠</span>
                    <div>
                        <div>Partial Match - Best Result (IoU: ${bestMatch.iou.toFixed(2)})</div>
                        <div style="font-size: 13px; font-weight: normal; margin-top: 4px;">
                            Fiber ID: ${bestMatch.fiber_id}
                        </div>
                        <div style="font-size: 12px; font-weight: normal; margin-top: 2px;">
                            • Missing from fiber: ${bestMatch.missing_logs.length} log${bestMatch.missing_logs.length !== 1 ? 's' : ''}
                            <br>
                            • Extra in fiber: ${bestMatch.extra_log_ids.length} log${bestMatch.extra_log_ids.length !== 1 ? 's' : ''}
                        </div>
                    </div>
                </div>
            `;
        } else {
            statusHtml = `
                <div class="test-result-status test-result-status-error">
                    <span class="test-result-status-icon">✗</span>
                    <div>
                        <div>No Match Found</div>
                        <div style="font-size: 13px; font-weight: normal; margin-top: 4px;">
                            No fibers were generated that matched the working set logs.
                        </div>
                    </div>
                </div>
            `;
        }

        // Build expected logs section
        let expectedLogsHtml = '<div class="test-result-section"><h3>Expected Logs (' + result.expected_logs.length + ')</h3><div class="test-logs-list">';
        result.expected_logs.forEach(log => {
            const inBestMatch = bestMatch && bestMatch.matching_logs.includes(log.id);
            const statusClass = inBestMatch ? 'match-success' : 'match-missing';
            const statusText = inBestMatch ? '✓ In fiber' : '✗ Missing';
            const statusTextClass = inBestMatch ? 'success' : 'missing';

            const timestamp = new Date(log.timestamp);
            const timeStr = this.formatTimestamp(timestamp);
            const sourceColor = colorManager.getFiberTypeColor(log.source_id);

            expectedLogsHtml += `
                <div class="test-log-item ${statusClass}">
                    <div class="test-log-timestamp">
                        ${timeStr}
                        <span class="test-log-source" style="background-color: ${sourceColor};">
                            ${this.escapeHtml(log.source_id)}
                        </span>
                    </div>
                    <div class="test-log-text">${this.escapeHtml(log.raw_text)}</div>
                    <div class="test-log-status ${statusTextClass}">${statusText}</div>
                </div>
            `;
        });
        expectedLogsHtml += '</div></div>';

        // Build best match fiber section
        let bestMatchHtml = '';
        if (bestMatch) {
            bestMatchHtml = '<div class="test-result-section"><h3>Best Match Fiber Logs (' + bestMatch.logs.length + ')</h3><div class="test-logs-list">';
            bestMatch.logs.forEach(log => {
                const expectedLogIds = result.expected_logs.map(l => l.id);
                const inExpected = expectedLogIds.includes(log.id);
                const statusClass = inExpected ? 'match-success' : 'match-extra';
                const statusText = inExpected ? '✓ Match' : '+ Extra';
                const statusTextClass = inExpected ? 'success' : 'extra';

                const timestamp = new Date(log.timestamp);
                const timeStr = this.formatTimestamp(timestamp);
                const sourceColor = colorManager.getFiberTypeColor(log.source_id);

                bestMatchHtml += `
                    <div class="test-log-item ${statusClass}">
                        <div class="test-log-timestamp">
                            ${timeStr}
                            <span class="test-log-source" style="background-color: ${sourceColor};">
                                ${this.escapeHtml(log.source_id)}
                            </span>
                        </div>
                        <div class="test-log-text">${this.escapeHtml(log.raw_text)}</div>
                        <div class="test-log-status ${statusTextClass}">${statusText}</div>
                    </div>
                `;
            });
            bestMatchHtml += '</div></div>';
        }

        // Build all fibers section
        let allFibersHtml = '<div class="test-result-section"><h3>All Generated Fibers (' + result.fibers_generated.length + ')</h3><div class="test-fibers-list">';
        result.fibers_generated.forEach((fiber, index) => {
            const isBest = index === result.best_match_index;
            const bestLabel = isBest ? ' - BEST' : '';
            const bestClass = isBest ? 'best-match' : '';

            allFibersHtml += `
                <div class="test-fiber-item ${bestClass}">
                    <div>
                        <span class="test-fiber-id">Fiber ${fiber.fiber_id.substring(0, 8)}...</span>
                        <span class="test-fiber-iou">IoU: ${fiber.iou.toFixed(2)}${bestLabel}</span>
                    </div>
                    <div style="font-size: 11px; color: var(--text-secondary);">
                        ${fiber.logs.length} logs (${fiber.matching_logs.length} match, ${fiber.missing_logs.length} missing, ${fiber.extra_log_ids.length} extra)
                    </div>
                </div>
            `;
        });
        allFibersHtml += '</div></div>';

        // Build actions
        const actionsHtml = `
            <div class="test-modal-actions">
                <button id="test-modal-close" class="btn btn-secondary">Close</button>
            </div>
        `;

        // Combine all sections
        content.innerHTML = statusHtml + expectedLogsHtml + bestMatchHtml + allFibersHtml + actionsHtml;

        // Show modal
        modal.style.display = 'flex';

        // Add event listeners
        const closeBtn = document.getElementById('test-modal-close');
        const closeX = document.getElementById('close-test-results-modal');

        if (closeBtn) {
            closeBtn.addEventListener('click', () => {
                modal.style.display = 'none';
            });
        }

        if (closeX) {
            closeX.addEventListener('click', () => {
                modal.style.display = 'none';
            });
        }

        // Close on outside click
        modal.addEventListener('click', (e) => {
            if (e.target === modal) {
                modal.style.display = 'none';
            }
        });

        // Close on ESC key
        const escHandler = (e) => {
            if (e.key === 'Escape') {
                modal.style.display = 'none';
                document.removeEventListener('keydown', escHandler);
            }
        };
        document.addEventListener('keydown', escHandler);
    }

    formatTimestamp(date) {
        const year = date.getFullYear();
        const month = String(date.getMonth() + 1).padStart(2, '0');
        const day = String(date.getDate()).padStart(2, '0');
        const hours = String(date.getHours()).padStart(2, '0');
        const minutes = String(date.getMinutes()).padStart(2, '0');
        const seconds = String(date.getSeconds()).padStart(2, '0');
        const ms = String(date.getMilliseconds()).padStart(3, '0');
        return `${year}-${month}-${day} ${hours}:${minutes}:${seconds}.${ms}`;
    }

    // =========================================================================
    // Config History Integration
    // =========================================================================

    async showConfigHistory() {
        try {
            const history = await this.api.getConfigHistory({ limit: 20, offset: 0 });

            // Create history modal (similar to config-editor.js but with fiber-specific handling)
            const modal = this.createConfigHistoryModal(history);
            document.body.appendChild(modal);

            // Show modal
            modal.style.display = 'flex';

            // Close handler
            const closeBtn = modal.querySelector('.history-close');
            closeBtn.addEventListener('click', () => {
                modal.remove();
            });
        } catch (error) {
            this.showStatus(`Error loading history: ${error.message}`, 'error');
            console.error('Failed to load history:', error);
        }
    }

    createConfigHistoryModal(history) {
        const modal = document.createElement('div');
        modal.className = 'config-history-modal';
        modal.style.cssText = `
            display: none;
            position: fixed;
            z-index: 10000;
            left: 0;
            top: 0;
            width: 100%;
            height: 100%;
            background-color: rgba(0,0,0,0.5);
            align-items: center;
            justify-content: center;
        `;

        const content = document.createElement('div');
        content.style.cssText = `
            background: var(--bg-secondary, #2a2a2a);
            padding: 20px;
            border-radius: 8px;
            max-width: 800px;
            max-height: 80vh;
            overflow-y: auto;
            color: var(--text-primary, #e0e0e0);
        `;

        content.innerHTML = `
            <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 20px;">
                <h3>Configuration History</h3>
                <div style="display: flex; gap: 10px; align-items: center;">
                    <button class="history-view-btn btn btn-secondary" disabled>View</button>
                    <button class="history-activate-btn btn btn-primary" disabled>Activate</button>
                    <button class="history-close" style="background: none; border: none; color: inherit; font-size: 24px; cursor: pointer;">&times;</button>
                </div>
            </div>
            <div class="history-list">
                ${this.renderConfigHistoryList(history.versions)}
            </div>
        `;

        modal.appendChild(content);

        // Setup event handlers for fiber-specific behavior
        this.setupConfigHistoryModalHandlers(modal, history.versions);

        return modal;
    }

    renderConfigHistoryList(versions) {
        if (versions.length === 0) {
            return '<p>No version history available.</p>';
        }

        return versions.map(version => {
            const date = new Date(version.created_at).toLocaleString();
            const hashShort = version.version_hash.substring(0, 8);
            const activeLabel = version.is_active ? ' <span style="color: #4caf50;">(Active)</span>' : '';

            return `
                <div class="history-item" style="border-bottom: 1px solid #444; padding: 10px 0; cursor: pointer; transition: background-color 0.2s;" data-hash="${version.version_hash}">
                    <div><strong>${date}</strong>  ${activeLabel}</div>
                    <div style="font-family: monospace; font-size: 0.9em; color: #aaa;">
                        ${hashShort} - ${version.source}
                        ${version.parent_hash ? `(parent: ${version.parent_hash.substring(0, 8)})` : '(root)'}
                    </div>
                </div>
            `;
        }).join('');
    }

    setupConfigHistoryModalHandlers(modal, versions) {
        const viewBtn = modal.querySelector('.history-view-btn');
        const activateBtn = modal.querySelector('.history-activate-btn');
        const historyItems = modal.querySelectorAll('.history-item');

        // Reset selected version
        this.selectedVersionHash = null;

        // Add click handlers to history items
        historyItems.forEach(item => {
            item.addEventListener('click', () => {
                const hash = item.getAttribute('data-hash');

                // Update selection
                this.selectedVersionHash = hash;

                // Update visual selection
                historyItems.forEach(i => {
                    if (i === item) {
                        i.style.backgroundColor = 'rgba(76, 175, 80, 0.2)';
                        i.style.borderLeft = '3px solid #4caf50';
                        i.style.paddingLeft = '7px';
                    } else {
                        i.style.backgroundColor = '';
                        i.style.borderLeft = '';
                        i.style.paddingLeft = '';
                    }
                });

                // Enable buttons
                viewBtn.disabled = false;
                activateBtn.disabled = false;
            });
        });

        // View button handler - fiber-specific: extract just the fiber_type YAML
        viewBtn.addEventListener('click', async () => {
            if (!this.selectedVersionHash || !this.selectedType) {
                if (!this.selectedType) {
                    this.showStatus('Please select a fiber type first', 'error');
                }
                return;
            }

            try {
                // Use the new API endpoint to get the fiber type from the specific version
                // This preserves the original YAML formatting without re-serialization
                const fiberTypeData = await this.api.getFiberTypeFromVersion(
                    this.selectedVersionHash,
                    this.selectedType
                );

                // Get the full config version metadata
                const version = await this.api.getConfigVersion(this.selectedVersionHash);

                // Update the current config version to the one being viewed
                this.currentConfigVersion = version;

                // Load into editor (this will be the preserved original YAML)
                this.setEditorValue(fiberTypeData.yaml_content);
                this.originalYaml = fiberTypeData.yaml_content;
                this.hasUnsavedChanges = false;

                // Update the config version indicator
                this.updateConfigVersionIndicator();

                // Update button states
                this.updateButtonStates();

                // Close modal
                modal.remove();

                const hashShort = version.version_hash.substring(0, 8);
                this.showStatus(`Viewing fiber type from version ${hashShort}`, 'info');
            } catch (error) {
                this.showStatus(`Error loading version: ${error.message}`, 'error');
                console.error('Failed to load config version:', error);
            }
        });

        // Activate button handler - activates the selected config version
        activateBtn.addEventListener('click', async () => {
            if (!this.selectedVersionHash) return;

            const hashShort = this.selectedVersionHash.substring(0, 8);

            const confirmed = confirm(
                `Activate config version ${hashShort}?\n\n` +
                'This will:\n' +
                '- Mark this version as active\n' +
                '- Activate ALL fiber types with rules from this version\n' +
                '- Close all open fibers\n\n' +
                'Continue?'
            );

            if (!confirmed) return;

            try {
                this.showStatus('Activating config version...', 'info');
                await this.api.activateConfigVersion(this.selectedVersionHash);

                // Reload config version
                await this.loadConfigVersion();

                // Reload fiber types list in case anything changed
                await this.loadFiberTypes();

                // If current fiber type is still selected, reload it
                if (this.selectedType) {
                    await this.selectFiberType(this.selectedType);
                }

                // Close modal
                modal.remove();

                this.showStatus(`Config version ${hashShort} activated successfully!`, 'success');
            } catch (error) {
                this.showStatus(`Error activating version: ${error.message}`, 'error');
                console.error('Failed to activate config version:', error);
            }
        });
    }
}

// Export for use by app.js
let fiberProcessingEditor = null;
