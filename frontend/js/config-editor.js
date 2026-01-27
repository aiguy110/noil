/**
 * Config Editor Module
 * Handles YAML configuration editing with version history
 */
class ConfigEditor {
    constructor(api) {
        this.api = api;
        this.currentConfig = null;
        this.originalYaml = '';
        this.elements = {};
        this.editorView = null; // CodeMirror editor instance
        this.editableCompartment = null; // Compartment for dynamic reconfiguration
        this.selectedVersionHash = null; // Currently selected version in history modal
        this.viewingHistoricalVersion = false; // Whether we're viewing a historical version
    }

    async init() {
        // Get DOM elements
        this.elements = {
            editorContainer: document.getElementById('config-yaml-editor'),
            saveBtn: document.getElementById('config-save'),
            resetBtn: document.getElementById('config-reset'),
            historyBtn: document.getElementById('config-history'),
            status: document.querySelector('.config-status'),
        };

        // Initialize CodeMirror editor
        try {
            await this.initCodeMirror();
        } catch (error) {
            console.warn('Failed to initialize CodeMirror, falling back to textarea:', error);
            this.useFallbackEditor();
        }

        // Load current config
        await this.loadCurrentConfig();

        // Setup event listeners
        this.setupEventListeners();
    }

    useFallbackEditor() {
        // If CodeMirror fails, just use the textarea
        const textarea = this.elements.editorContainer;
        if (textarea) {
            textarea.style.display = 'block';
            textarea.value = '';
            this.editorView = null;
        }
    }

    async initCodeMirror() {
        // Wait for CodeMirror to be loaded (with timeout)
        if (!window.CodeMirror && !window.CodeMirrorLoadError) {
            await new Promise((resolve, reject) => {
                const timeout = setTimeout(() => {
                    reject(new Error('CodeMirror failed to load (timeout)'));
                }, 10000);

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

        if (!EditorView || !EditorState || !Compartment || !basicSetup || !yaml) {
            throw new Error('CodeMirror components are incomplete');
        }

        // Create compartment for editable state
        this.editableCompartment = new Compartment();

        // Create editor state with YAML syntax highlighting
        const startState = EditorState.create({
            doc: '',
            extensions: [
                basicSetup,
                yaml,
                EditorState.tabSize.of(2),
                this.editableCompartment.of(EditorView.editable.of(true))
            ]
        });

        // Replace textarea with CodeMirror
        const textarea = this.elements.editorContainer;
        if (!textarea) {
            throw new Error('Editor textarea element not found');
        }

        // Hide textarea
        textarea.style.display = 'none';

        // Create wrapper div
        const editorWrapper = document.createElement('div');
        editorWrapper.className = 'cm-editor-wrapper';
        textarea.parentElement.insertBefore(editorWrapper, textarea.nextSibling);

        // Create CodeMirror editor
        this.editorView = new EditorView({
            state: startState,
            parent: editorWrapper
        });

        if (!this.editorView || !this.editorView.dom) {
            throw new Error('Failed to create CodeMirror editor');
        }
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

    setupEventListeners() {
        // Save button
        this.elements.saveBtn.addEventListener('click', () => {
            this.saveConfig();
        });

        // Reset button
        this.elements.resetBtn.addEventListener('click', () => {
            this.resetConfig();
        });

        // History button
        this.elements.historyBtn.addEventListener('click', () => {
            this.showHistory();
        });
    }

    async loadCurrentConfig() {
        try {
            this.showStatus('Loading configuration...', 'info');
            this.currentConfig = await this.api.getCurrentConfig();
            this.originalYaml = this.currentConfig.yaml_content;
            this.setEditorValue(this.originalYaml);
            this.showStatus('Configuration loaded', 'success');
        } catch (error) {
            this.showStatus(`Error loading config: ${error.message}`, 'error');
            console.error('Failed to load config:', error);
        }
    }


    async saveConfig() {
        const yaml = this.getEditorValue();

        // Validate first
        try {
            jsyaml.load(yaml);
        } catch (error) {
            this.showStatus(`Invalid YAML: ${error.message}`, 'error');
            return;
        }

        // Check if changed (trim to avoid false positives from whitespace)
        if (yaml.trim() === this.originalYaml.trim()) {
            this.showStatus('No changes to save', 'info');
            return;
        }

        // Confirm save
        if (!confirm('Save configuration? Note: Global config changes require a restart. Fiber types can be activated separately.')) {
            return;
        }

        try {
            this.showStatus('Saving configuration...', 'info');
            const result = await this.api.updateConfig(yaml);

            this.showStatus('Configuration saved! Restart required to activate.', 'success');
            this.originalYaml = yaml;
            this.currentConfig = result;

            // Show restart reminder
            setTimeout(() => {
                alert('Configuration has been saved.\n\nIMPORTANT: You must restart the Noil server for global config changes to take effect.\n\nNote: Fiber type changes can be activated in the Fiber Processing tab without restarting.');
            }, 500);
        } catch (error) {
            this.showStatus(`Error saving config: ${error.message}`, 'error');
            console.error('Failed to save config:', error);
        }
    }

    resetConfig() {
        if (this.getEditorValue().trim() !== this.originalYaml.trim()) {
            if (confirm('Discard unsaved changes?')) {
                this.setEditorValue(this.originalYaml);
                this.showStatus('Changes discarded', 'info');
            }
        } else {
            this.showStatus('No changes to discard', 'info');
        }
    }

    async showHistory() {
        try {
            this.showStatus('Loading version history...', 'info');
            const history = await this.api.getConfigHistory({ limit: 20, offset: 0 });

            // Create history modal
            const modal = this.createHistoryModal(history);
            document.body.appendChild(modal);

            // Show modal
            modal.style.display = 'flex';

            // Close handler
            const closeBtn = modal.querySelector('.history-close');
            closeBtn.addEventListener('click', () => {
                modal.remove();
            });

            this.showStatus('', '');
        } catch (error) {
            this.showStatus(`Error loading history: ${error.message}`, 'error');
            console.error('Failed to load history:', error);
        }
    }

    createHistoryModal(history) {
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
                ${this.renderHistoryList(history.versions)}
            </div>
        `;

        modal.appendChild(content);

        // Setup event handlers
        this.setupHistoryModalHandlers(modal, history.versions);

        return modal;
    }

    renderHistoryList(versions) {
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

    setupHistoryModalHandlers(modal, versions) {
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

        // View button handler
        viewBtn.addEventListener('click', async () => {
            if (!this.selectedVersionHash) return;

            try {
                this.showStatus('Loading config version...', 'info');
                const version = await this.api.getConfigVersion(this.selectedVersionHash);

                // Load the YAML content into editor
                this.setEditorValue(version.yaml_content);
                this.viewingHistoricalVersion = true;

                // Add indicator banner
                this.showHistoricalVersionBanner(version);

                // Close modal
                modal.remove();

                this.showStatus(`Viewing version ${version.version_hash.substring(0, 8)}`, 'info');
            } catch (error) {
                this.showStatus(`Error loading version: ${error.message}`, 'error');
                console.error('Failed to load config version:', error);
            }
        });

        // Activate button handler
        activateBtn.addEventListener('click', async () => {
            if (!this.selectedVersionHash) return;

            const version = versions.find(v => v.version_hash === this.selectedVersionHash);
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

                // Reload current config
                await this.loadCurrentConfig();

                // Remove historical version banner if present
                this.removeHistoricalVersionBanner();
                this.viewingHistoricalVersion = false;

                // Close modal
                modal.remove();

                this.showStatus(`Config version ${hashShort} activated successfully!`, 'success');
            } catch (error) {
                this.showStatus(`Error activating version: ${error.message}`, 'error');
                console.error('Failed to activate config version:', error);
            }
        });
    }

    showHistoricalVersionBanner(version) {
        // Remove existing banner if present
        this.removeHistoricalVersionBanner();

        const banner = document.createElement('div');
        banner.className = 'historical-version-banner';
        banner.style.cssText = `
            background: #ff9800;
            color: #000;
            padding: 10px;
            margin-bottom: 10px;
            border-radius: 4px;
            display: flex;
            justify-content: space-between;
            align-items: center;
            font-weight: bold;
        `;

        const hashShort = version.version_hash.substring(0, 8);
        const date = new Date(version.created_at).toLocaleString();

        banner.innerHTML = `
            <span>⚠ Viewing historical version: ${hashShort} (${date})</span>
            <button class="btn-restore-current" style="background: #fff; color: #000; border: none; padding: 5px 10px; border-radius: 4px; cursor: pointer; font-weight: bold;">
                Restore Current
            </button>
        `;

        // Insert banner before editor
        const editorContainer = this.elements.editorContainer;
        if (this.editorView) {
            // CodeMirror mode - insert before wrapper
            const wrapper = editorContainer.parentElement.querySelector('.cm-editor-wrapper');
            wrapper.parentElement.insertBefore(banner, wrapper);
        } else {
            // Fallback textarea mode
            editorContainer.parentElement.insertBefore(banner, editorContainer);
        }

        // Restore button handler
        banner.querySelector('.btn-restore-current').addEventListener('click', () => {
            this.restoreCurrentConfig();
        });
    }

    removeHistoricalVersionBanner() {
        const banner = document.querySelector('.historical-version-banner');
        if (banner) {
            banner.remove();
        }
    }

    restoreCurrentConfig() {
        this.setEditorValue(this.originalYaml);
        this.removeHistoricalVersionBanner();
        this.viewingHistoricalVersion = false;
        this.showStatus('Restored to current active config', 'info');
    }

    showStatus(message, type) {
        this.elements.status.textContent = message;
        this.elements.status.className = `config-status config-status-${type}`;

        if (type === 'success' || type === 'info') {
            setTimeout(() => {
                if (this.elements.status.textContent === message) {
                    this.elements.status.textContent = '';
                    this.elements.status.className = 'config-status';
                }
            }, 5000);
        }
    }
}

// Initialize when DOM is loaded
let configEditor = null;

document.addEventListener('DOMContentLoaded', () => {
    // Config editor will be initialized when Settings modal opens
    // See app.js for initialization logic
});
