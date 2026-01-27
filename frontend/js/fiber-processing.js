/**
 * Fiber Processing Editor Module
 * Handles fiber type editing with hot-reload and reprocessing
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
        this.elementIds = {
            typeListEl: 'fiber-type-list',
            typeNameEl: 'fiber-type-name',
            editorEl: 'fiber-yaml-editor',
            saveBtnEl: 'fiber-save',
            hotReloadBtnEl: 'fiber-hot-reload',
            deleteBtnEl: 'fiber-delete',
            newBtnEl: 'new-fiber-type',
            statusEl: 'fiber-status',
            reprocessBtnEl: 'start-reprocess',
            reprocessProgressEl: 'reprocess-progress',
            reprocessProgressBarEl: 'reprocess-progress-bar',
            reprocessStatusTextEl: 'reprocess-status-text',
            reprocessProgressTextEl: 'reprocess-progress-text',
            cancelReprocessBtnEl: 'cancel-reprocess',
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
            hotReloadBtn: document.getElementById(this.elementIds.hotReloadBtnEl),
            deleteBtn: document.getElementById(this.elementIds.deleteBtnEl),
            newBtn: document.getElementById(this.elementIds.newBtnEl),
            status: document.getElementById(this.elementIds.statusEl),
            reprocessBtn: document.getElementById(this.elementIds.reprocessBtnEl),
            reprocessProgress: document.getElementById(this.elementIds.reprocessProgressEl),
            reprocessProgressBar: document.getElementById(this.elementIds.reprocessProgressBarEl),
            reprocessStatusText: document.getElementById(this.elementIds.reprocessStatusTextEl),
            reprocessProgressText: document.getElementById(this.elementIds.reprocessProgressTextEl),
            cancelReprocessBtn: document.getElementById(this.elementIds.cancelReprocessBtnEl),
        };

        // Initialize CodeMirror editor
        try {
            await this.initCodeMirror();
        } catch (error) {
            console.warn('Failed to initialize CodeMirror, falling back to textarea:', error);
            // Fall back to using the textarea directly
            this.useFallbackEditor();
        }

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

        const { EditorView, EditorState, Compartment, basicSetup, yaml } = window.CodeMirror;

        // Verify all required components are present
        if (!EditorView || !EditorState || !Compartment || !basicSetup || !yaml) {
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
        const yaml = this.editorView.state.doc.toString();
        this.hasUnsavedChanges = yaml !== this.originalYaml;

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

        // Reprocess button
        this.elements.reprocessBtn.addEventListener('click', () => {
            this.showReprocessDialog();
        });

        // Cancel reprocess button
        this.elements.cancelReprocessBtn.addEventListener('click', () => {
            this.cancelReprocessing();
        });

        // Editor change tracking for fallback textarea mode
        if (!this.editorView && this.elements.editorContainer) {
            this.elements.editorContainer.addEventListener('input', () => {
                this.handleEditorChange();
            });
        }
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
        this.setEditorEnabled(hasSelection);

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

    // =========================================================================
    // Working Set Panel
    // =========================================================================

    initWorkingSetPanel() {
        // Get working set panel elements
        const clearBtn = document.getElementById('working-set-clear');
        const testBtn = document.getElementById('working-set-test');

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

        // Initial render
        this.renderWorkingSetPanel();
    }

    async renderWorkingSetPanel() {
        const countEl = document.getElementById('working-set-count');
        const contentEl = document.getElementById('working-set-content');

        if (!window.app || !countEl || !contentEl) return;

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

        for (const logId of logIds) {
            try {
                // Fetch log details if not in cache
                let log = workingSet.logs[logId];
                if (!log) {
                    log = await this.api.getLog(logId);
                    workingSet.logs[logId] = log;
                }

                // Fetch fiber memberships
                let fiberData = await this.api.getLogFibers(logId);

                const logItem = this.createWorkingSetLogItem(log, fiberData.fibers);
                contentEl.appendChild(logItem);
            } catch (error) {
                console.error(`Failed to fetch log ${logId}:`, error);
                // Remove from working set if it doesn't exist
                if (error.message.includes('404')) {
                    window.app.removeFromWorkingSet(logId);
                }
            }
        }
    }

    createWorkingSetLogItem(log, fibers) {
        const item = document.createElement('div');
        item.className = 'working-set-log-item';
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

        // Source badge color
        const sourceBadgeColor = colorManager.getFiberTypeColor(log.source_id);

        // Truncate log text
        const truncatedText = log.raw_text.length > 50
            ? log.raw_text.substring(0, 50) + '...'
            : log.raw_text;

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

        item.innerHTML = `
            <div class="working-set-log-timestamp">
                ${timeStr}
                <span class="working-set-log-source" style="background-color: ${sourceBadgeColor};">
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

        return item;
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
            // Check if backend endpoint is not implemented yet
            if (error.message.includes('404') || error.message.includes('not found')) {
                this.showStatus('Backend API not yet implemented. Please implement the test-working-set endpoint first.', 'info');
            } else {
                this.showStatus(`Test failed: ${error.message}`, 'error');
            }
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
}

// Export for use by app.js
let fiberProcessingEditor = null;
