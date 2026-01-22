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
    }

    async init() {
        // Get DOM elements
        this.elements = {
            editor: document.getElementById('config-yaml-editor'),
            saveBtn: document.getElementById('config-save'),
            resetBtn: document.getElementById('config-reset'),
            historyBtn: document.getElementById('config-history'),
            status: document.querySelector('.config-status'),
        };

        // Load current config
        await this.loadCurrentConfig();

        // Setup event listeners
        this.setupEventListeners();
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
            this.elements.editor.value = this.originalYaml;
            this.showStatus('Configuration loaded', 'success');
        } catch (error) {
            this.showStatus(`Error loading config: ${error.message}`, 'error');
            console.error('Failed to load config:', error);
        }
    }


    async saveConfig() {
        const yaml = this.elements.editor.value;

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
        if (!confirm('Save configuration? Note: A restart is required for changes to take effect.')) {
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
                alert('Configuration has been saved.\n\nIMPORTANT: You must restart the Noil server for changes to take effect.');
            }, 500);
        } catch (error) {
            this.showStatus(`Error saving config: ${error.message}`, 'error');
            console.error('Failed to save config:', error);
        }
    }

    resetConfig() {
        if (this.elements.editor.value !== this.originalYaml) {
            if (confirm('Discard unsaved changes?')) {
                this.elements.editor.value = this.originalYaml;
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
