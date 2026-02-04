/**
 * Theme Manager Module
 * Handles CodeMirror editor theme selection and persistence
 */

const THEMES = {
    dracula: {
        name: 'Dracula',
        description: 'Dark theme with vibrant colors (default)',
        css: {
            '.cm-editor': {
                'background-color': '#282a36',
                'color': '#f8f8f2',
                'border-color': '#44475a'
            },
            '.cm-editor.cm-focused': {
                'border-color': '#bd93f9'
            },
            '.cm-content': {
                'color': '#f8f8f2'
            },
            '.cm-gutters': {
                'background-color': '#21222c',
                'border-right': '1px solid #44475a',
                'color': '#6272a4'
            },
            '.cm-activeLineGutter': {
                'background-color': '#44475a'
            },
            '.cm-activeLine': {
                'background-color': 'rgba(68, 71, 90, 0.3)'
            },
            '.cm-selectionBackground': {
                'background-color': 'rgba(189, 147, 249, 0.5) !important'
            },
            '.cm-selectionLayer .cm-selectionBackground': {
                'background-color': 'rgba(189, 147, 249, 0.5) !important'
            },
            '.cm-line ::selection': {
                'background-color': 'rgba(189, 147, 249, 0.5) !important',
                'color': 'inherit'
            },
            '.cm-line ::-moz-selection': {
                'background-color': 'rgba(189, 147, 249, 0.5) !important',
                'color': 'inherit'
            },
            '.cm-cursor': {
                'border-left-color': '#f8f8f2'
            },
            '.cm-atom, .cm-string': {
                'color': '#f1fa8c'
            },
            '.cm-number': {
                'color': '#bd93f9'
            },
            '.cm-keyword': {
                'color': '#ff79c6'
            },
            '.cm-variable, .cm-property': {
                'color': '#8be9fd'
            },
            '.cm-comment': {
                'color': '#6272a4',
                'font-style': 'italic'
            },
            '.cm-meta': {
                'color': '#ffb86c'
            },
            '.cm-operator': {
                'color': '#f8f8f2'
            },
            '.cm-lineNumbers .cm-gutterElement': {
                'color': '#6272a4'
            },
            '.ͼc': {
                'color': '#8be9fd'
            },
            '.ͼ5': {
                'color': '#ff79c6'
            }
        }
    },
    vscode: {
        name: 'VS Code Dark',
        description: 'Dark theme inspired by VS Code',
        css: {
            '.cm-editor': {
                'background-color': '#2d2d30',
                'color': '#cccccc',
                'border-color': '#3e3e42'
            },
            '.cm-editor.cm-focused': {
                'border-color': '#007acc'
            },
            '.cm-content': {
                'color': '#cccccc'
            },
            '.cm-gutters': {
                'background-color': '#252526',
                'border-right': '1px solid #3e3e42',
                'color': '#858585'
            },
            '.cm-activeLineGutter': {
                'background-color': '#2d2d30'
            },
            '.cm-activeLine': {
                'background-color': 'rgba(255, 255, 255, 0.05)'
            },
            '.cm-selectionBackground': {
                'background-color': 'rgba(0, 122, 204, 0.5) !important'
            },
            '.cm-selectionLayer .cm-selectionBackground': {
                'background-color': 'rgba(0, 122, 204, 0.5) !important'
            },
            '.cm-line ::selection': {
                'background-color': 'rgba(0, 122, 204, 0.5) !important',
                'color': 'inherit'
            },
            '.cm-line ::-moz-selection': {
                'background-color': 'rgba(0, 122, 204, 0.5) !important',
                'color': 'inherit'
            },
            '.cm-cursor': {
                'border-left-color': '#cccccc'
            },
            '.cm-atom, .cm-string': {
                'color': '#ce9178'
            },
            '.cm-number': {
                'color': '#b5cea8'
            },
            '.cm-keyword': {
                'color': '#569cd6'
            },
            '.cm-variable, .cm-property': {
                'color': '#9cdcfe'
            },
            '.cm-comment': {
                'color': '#6a9955',
                'font-style': 'italic'
            },
            '.cm-meta': {
                'color': '#dcdcaa'
            },
            '.cm-operator': {
                'color': '#cccccc'
            },
            '.cm-lineNumbers .cm-gutterElement': {
                'color': '#858585'
            },
            '.ͼc': {
                'color': '#9cdcfe'
            },
            '.ͼ5': {
                'color': '#569cd6'
            }
        }
    },
    monokai: {
        name: 'Monokai',
        description: 'Popular dark theme with warm colors',
        css: {
            '.cm-editor': {
                'background-color': '#272822',
                'color': '#f8f8f2',
                'border-color': '#3e3d32'
            },
            '.cm-editor.cm-focused': {
                'border-color': '#f92672'
            },
            '.cm-content': {
                'color': '#f8f8f2'
            },
            '.cm-gutters': {
                'background-color': '#1e1f1c',
                'border-right': '1px solid #3e3d32',
                'color': '#75715e'
            },
            '.cm-activeLineGutter': {
                'background-color': '#3e3d32'
            },
            '.cm-activeLine': {
                'background-color': 'rgba(255, 255, 255, 0.05)'
            },
            '.cm-selectionBackground': {
                'background-color': 'rgba(102, 217, 239, 0.5) !important'
            },
            '.cm-selectionLayer .cm-selectionBackground': {
                'background-color': 'rgba(102, 217, 239, 0.5) !important'
            },
            '.cm-line ::selection': {
                'background-color': 'rgba(102, 217, 239, 0.5) !important',
                'color': 'inherit'
            },
            '.cm-line ::-moz-selection': {
                'background-color': 'rgba(102, 217, 239, 0.5) !important',
                'color': 'inherit'
            },
            '.cm-cursor': {
                'border-left-color': '#f8f8f0'
            },
            '.cm-atom, .cm-string': {
                'color': '#e6db74'
            },
            '.cm-number': {
                'color': '#ae81ff'
            },
            '.cm-keyword': {
                'color': '#f92672'
            },
            '.cm-variable, .cm-property': {
                'color': '#a6e22e'
            },
            '.cm-comment': {
                'color': '#75715e',
                'font-style': 'italic'
            },
            '.cm-meta': {
                'color': '#66d9ef'
            },
            '.cm-operator': {
                'color': '#f92672'
            },
            '.cm-lineNumbers .cm-gutterElement': {
                'color': '#75715e'
            },
            '.ͼc': {
                'color': '#a6e22e'
            },
            '.ͼ5': {
                'color': '#f92672'
            }
        }
    },
    solarized: {
        name: 'Solarized Dark',
        description: 'Low-contrast dark theme',
        css: {
            '.cm-editor': {
                'background-color': '#002b36',
                'color': '#839496',
                'border-color': '#073642'
            },
            '.cm-editor.cm-focused': {
                'border-color': '#268bd2'
            },
            '.cm-content': {
                'color': '#839496'
            },
            '.cm-gutters': {
                'background-color': '#073642',
                'border-right': '1px solid #002b36',
                'color': '#586e75'
            },
            '.cm-activeLineGutter': {
                'background-color': '#002b36'
            },
            '.cm-activeLine': {
                'background-color': 'rgba(7, 54, 66, 0.5)'
            },
            '.cm-selectionBackground': {
                'background-color': 'rgba(38, 139, 210, 0.5) !important'
            },
            '.cm-selectionLayer .cm-selectionBackground': {
                'background-color': 'rgba(38, 139, 210, 0.5) !important'
            },
            '.cm-line ::selection': {
                'background-color': 'rgba(38, 139, 210, 0.5) !important',
                'color': 'inherit'
            },
            '.cm-line ::-moz-selection': {
                'background-color': 'rgba(38, 139, 210, 0.5) !important',
                'color': 'inherit'
            },
            '.cm-cursor': {
                'border-left-color': '#839496'
            },
            '.cm-atom, .cm-string': {
                'color': '#2aa198'
            },
            '.cm-number': {
                'color': '#d33682'
            },
            '.cm-keyword': {
                'color': '#859900'
            },
            '.cm-variable, .cm-property': {
                'color': '#268bd2'
            },
            '.cm-comment': {
                'color': '#586e75',
                'font-style': 'italic'
            },
            '.cm-meta': {
                'color': '#cb4b16'
            },
            '.cm-operator': {
                'color': '#839496'
            },
            '.cm-lineNumbers .cm-gutterElement': {
                'color': '#586e75'
            },
            '.ͼc': {
                'color': '#268bd2'
            },
            '.ͼ5': {
                'color': '#859900'
            }
        }
    },
    github_light: {
        name: 'GitHub Light',
        description: 'Light theme for bright environments',
        css: {
            '.cm-editor': {
                'background-color': '#ffffff',
                'color': '#24292f',
                'border-color': '#d0d7de'
            },
            '.cm-editor.cm-focused': {
                'border-color': '#0969da'
            },
            '.cm-content': {
                'color': '#24292f'
            },
            '.cm-gutters': {
                'background-color': '#f6f8fa',
                'border-right': '1px solid #d0d7de',
                'color': '#57606a'
            },
            '.cm-activeLineGutter': {
                'background-color': '#ffffff'
            },
            '.cm-activeLine': {
                'background-color': 'rgba(0, 0, 0, 0.03)'
            },
            '.cm-selectionBackground': {
                'background-color': 'rgba(9, 105, 218, 0.4) !important'
            },
            '.cm-selectionLayer .cm-selectionBackground': {
                'background-color': 'rgba(9, 105, 218, 0.4) !important'
            },
            '.cm-line ::selection': {
                'background-color': 'rgba(9, 105, 218, 0.4) !important',
                'color': 'inherit'
            },
            '.cm-line ::-moz-selection': {
                'background-color': 'rgba(9, 105, 218, 0.4) !important',
                'color': 'inherit'
            },
            '.cm-cursor': {
                'border-left-color': '#24292f'
            },
            '.cm-atom, .cm-string': {
                'color': '#0a3069'
            },
            '.cm-number': {
                'color': '#0550ae'
            },
            '.cm-keyword': {
                'color': '#cf222e'
            },
            '.cm-variable, .cm-property': {
                'color': '#953800'
            },
            '.cm-comment': {
                'color': '#6e7781',
                'font-style': 'italic'
            },
            '.cm-meta': {
                'color': '#8250df'
            },
            '.cm-operator': {
                'color': '#24292f'
            },
            '.cm-lineNumbers .cm-gutterElement': {
                'color': '#57606a'
            },
            '.ͼc': {
                'color': '#0969da'
            }
        }
    },
    onedark: {
        name: 'One Dark',
        description: 'Popular dark theme from Atom editor',
        css: {
            '.cm-editor': {
                'background-color': '#282c34',
                'color': '#abb2bf',
                'border-color': '#3e4451'
            },
            '.cm-editor.cm-focused': {
                'border-color': '#61afef'
            },
            '.cm-content': {
                'color': '#abb2bf'
            },
            '.cm-gutters': {
                'background-color': '#21252b',
                'border-right': '1px solid #181a1f',
                'color': '#5c6370'
            },
            '.cm-activeLineGutter': {
                'background-color': '#2c323c'
            },
            '.cm-activeLine': {
                'background-color': '#2c323c'
            },
            '.cm-selectionBackground': {
                'background-color': 'rgba(97, 175, 239, 0.5) !important'
            },
            '.cm-selectionLayer .cm-selectionBackground': {
                'background-color': 'rgba(97, 175, 239, 0.5) !important'
            },
            '.cm-line ::selection': {
                'background-color': 'rgba(97, 175, 239, 0.5) !important',
                'color': 'inherit'
            },
            '.cm-line ::-moz-selection': {
                'background-color': 'rgba(97, 175, 239, 0.5) !important',
                'color': 'inherit'
            },
            '.cm-cursor': {
                'border-left-color': '#528bff'
            },
            '.cm-atom, .cm-string': {
                'color': '#98c379'
            },
            '.cm-number': {
                'color': '#d19a66'
            },
            '.cm-keyword': {
                'color': '#c678dd'
            },
            '.cm-variable': {
                'color': '#e06c75'
            },
            '.cm-property': {
                'color': '#61dafb'
            },
            '.cm-comment': {
                'color': '#5c6370',
                'font-style': 'italic'
            },
            '.cm-meta': {
                'color': '#d19a66'
            },
            '.cm-operator': {
                'color': '#abb2bf'
            },
            '.cm-lineNumbers .cm-gutterElement': {
                'color': '#5c6370'
            },
            '.ͼb': {
                'color': '#e5c07b'
            },
            '.ͼc': {
                'color': '#61dafb'
            },
            '.ͼ5': {
                'color': '#c678dd'
            },
            '.ͼe': {
                'color': '#98c379'
            }
        }
    }
};

const SAMPLE_YAML = `# Sample YAML Configuration
fiber_types:
  example_trace:
    description: "Example fiber type for theme preview"
    temporal:
      max_gap: 5s  # Maximum gap between logs
      gap_mode: session

    attributes:
      # Keys used for fiber matching
      - name: request_id
        type: string
        key: true

      - name: user_ip
        type: ip
        key: false

      # Derived attribute example
      - name: connection
        type: string
        derived: "\${user_ip}:\${port}"

    sources:
      application_log:
        patterns:
          - regex: 'request-(?P<request_id>\\\\w+)'
            release_matching_peer_keys: [request_id]
          - regex: 'IP: (?P<user_ip>\\\\d+\\\\.\\\\d+\\\\.\\\\d+\\\\.\\\\d+)'`;

class ThemeManager {
    constructor() {
        this.currentTheme = this.loadTheme();
        this.styleElement = null;
        this.previewEditor = null;
    }

    init() {
        this.createStyleElement();
        this.applyTheme(this.currentTheme);
        this.renderThemeList();
        this.initPreviewEditor();
    }

    loadTheme() {
        const saved = localStorage.getItem('noil_editor_theme');
        return saved && THEMES[saved] ? saved : 'dracula';
    }

    saveTheme(themeId) {
        localStorage.setItem('noil_editor_theme', themeId);
    }

    createStyleElement() {
        // Check if we already have a dynamic theme style element
        this.styleElement = document.getElementById('dynamic-theme-styles');
        if (!this.styleElement) {
            this.styleElement = document.createElement('style');
            this.styleElement.id = 'dynamic-theme-styles';
            document.head.appendChild(this.styleElement);
        }
    }

    applyTheme(themeId) {
        if (!THEMES[themeId]) {
            console.error('Unknown theme:', themeId);
            return;
        }

        this.currentTheme = themeId;
        this.saveTheme(themeId);

        const theme = THEMES[themeId];
        let css = '/* Dynamic CodeMirror Theme */\n';

        // Convert theme object to CSS
        for (const [selector, rules] of Object.entries(theme.css)) {
            css += `${selector} {\n`;
            for (const [property, value] of Object.entries(rules)) {
                css += `  ${property}: ${value} !important;\n`;
            }
            css += '}\n\n';
        }

        this.styleElement.textContent = css;

        // Update selected theme in UI
        this.updateThemeSelection();
    }

    renderThemeList() {
        const container = document.getElementById('theme-list');
        if (!container) return;

        container.innerHTML = '';

        for (const [themeId, theme] of Object.entries(THEMES)) {
            const button = document.createElement('button');
            button.className = 'theme-item';
            if (themeId === this.currentTheme) {
                button.classList.add('selected');
            }

            button.innerHTML = `
                <div class="theme-name">${theme.name}</div>
                <div class="theme-description">${theme.description}</div>
            `;

            button.addEventListener('click', () => {
                this.applyTheme(themeId);
            });

            container.appendChild(button);
        }
    }

    updateThemeSelection() {
        const buttons = document.querySelectorAll('.theme-item');
        buttons.forEach((button, index) => {
            const themeId = Object.keys(THEMES)[index];
            button.classList.toggle('selected', themeId === this.currentTheme);
        });
    }

    async initPreviewEditor() {
        // Wait for CodeMirror to be loaded
        if (!window.CodeMirror) {
            await new Promise((resolve) => {
                const checkInterval = setInterval(() => {
                    if (window.CodeMirror) {
                        clearInterval(checkInterval);
                        resolve();
                    }
                }, 100);
            });
        }

        const { EditorView, EditorState, basicSetup, yaml } = window.CodeMirror;

        const textarea = document.getElementById('theme-preview-editor');
        if (!textarea) return;

        // Hide textarea
        textarea.style.display = 'none';

        // Create wrapper
        const wrapper = document.createElement('div');
        wrapper.className = 'cm-editor-wrapper';
        textarea.parentElement.insertBefore(wrapper, textarea.nextSibling);

        // Create preview editor
        const startState = EditorState.create({
            doc: SAMPLE_YAML,
            extensions: [
                basicSetup,
                yaml,
                EditorView.editable.of(false), // Read-only
                EditorState.tabSize.of(2)
            ]
        });

        this.previewEditor = new EditorView({
            state: startState,
            parent: wrapper
        });
    }
}

// Global instance
let themeManager = null;

// Initialize when DOM is loaded
document.addEventListener('DOMContentLoaded', () => {
    // Wait for CodeMirror to be ready
    const initThemeManager = () => {
        if (!themeManager) {
            themeManager = new ThemeManager();
            themeManager.init();
        }
    };

    if (window.CodeMirror) {
        initThemeManager();
    } else {
        window.addEventListener('codemirror-ready', initThemeManager);
    }
});
