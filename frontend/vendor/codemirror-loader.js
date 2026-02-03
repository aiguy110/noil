// CodeMirror 6 Local Bundle Loader
// Loads CodeMirror from a locally bundled file and exposes it on window.CodeMirror

(async function() {
    try {
        // Import from local bundle
        const module = await import('./codemirror/codemirror.bundle.js');

        // Extract components
        const { EditorView, EditorState, Compartment, basicSetup, yaml, keymap, indentWithTab } = module;

        // Verify all components are present
        if (!EditorView || !EditorState || !Compartment || !basicSetup || !yaml || !keymap || !indentWithTab) {
            throw new Error('One or more CodeMirror components failed to load from bundle');
        }

        // Expose on window
        window.CodeMirror = {
            EditorView,
            EditorState,
            Compartment,
            basicSetup,
            yaml,
            keymap,
            indentWithTab
        };

        console.log('CodeMirror loaded successfully from local bundle');

        // Dispatch event to notify that CodeMirror is ready
        window.dispatchEvent(new Event('codemirror-ready'));
    } catch (error) {
        console.error('Failed to load CodeMirror from local bundle:', error);
        window.CodeMirrorLoadError = error;
    }
})();
