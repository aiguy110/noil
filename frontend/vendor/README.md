# CodeMirror 6 Local Bundle

This directory contains a locally bundled version of CodeMirror 6 with YAML support.

## Files

- `codemirror/codemirror.bundle.js` - Single-file bundle containing all CodeMirror dependencies (378KB)
- `codemirror-loader.js` - Loader script that imports the bundle and exposes it on `window.CodeMirror`
- `build-codemirror.mjs` - Build script for regenerating the bundle
- `package.json` - Dependencies for building the bundle

## Rebuilding the Bundle

If you need to update CodeMirror or rebuild the bundle:

```bash
cd frontend/vendor
npm install
npm run build
```

This will regenerate `codemirror/codemirror.bundle.js`.

## How It Works

1. The HTML loads `vendor/codemirror-loader.js` as an ES module
2. The loader imports the local bundle file
3. CodeMirror components are exposed on `window.CodeMirror` for use by the app
4. A `codemirror-ready` event is dispatched when loading is complete

## Dependencies Bundled

- codemirror@6.0.1 (core + basicSetup)
- @codemirror/state@6.4.0 (EditorState, Compartment)
- @codemirror/view@6.23.0 (EditorView)
- @codemirror/language@6.10.0 (StreamLanguage)
- @codemirror/legacy-modes@6.3.3 (YAML syntax mode)

All dependencies are bundled into a single file with no external runtime dependencies.
