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

        // Check if changed
        if (yaml === this.originalYaml) {
            this.showStatus('No changes to save', 'info');
            return;
        }

        // Confirm save
        if (!confirm('Save configuration? Note: Global config changes require a restart. Fiber types can be hot-reloaded separately.')) {
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
                alert('Configuration has been saved.\n\nIMPORTANT: You must restart the Noil server for global config changes to take effect.\n\nNote: Fiber type changes can be hot-reloaded in the Fiber Processing tab without restarting.');
            }, 500);
        } catch (error) {
            this.showStatus(`Error saving config: ${error.message}`, 'error');
            console.error('Failed to save config:', error);
        }
    }

    resetConfig() {
        if (this.getEditorValue() !== this.originalYaml) {
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
                <button class="history-close" style="background: none; border: none; color: inherit; font-size: 24px; cursor: pointer;">&times;</button>
            </div>
            <div class="history-list">
                ${this.renderHistoryList(history.versions)}
            </div>
        `;

        modal.appendChild(content);
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
                <div class="history-item" style="border-bottom: 1px solid #444; padding: 10px 0; cursor: pointer;" data-hash="${version.version_hash}">
                    <div><strong>${date}</strong>  ${activeLabel}</div>
                    <div style="font-family: monospace; font-size: 0.9em; color: #aaa;">
                        ${hashShort} - ${version.source}
                        ${version.parent_hash ? `(parent: ${version.parent_hash.substring(0, 8)})` : '(root)'}
                    </div>
                </div>
            `;
        }).join('');
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
