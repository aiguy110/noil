#!/usr/bin/env node
// Builds a single CodeMirror bundle file
import * as esbuild from 'esbuild';

const code = `
// CodeMirror 6 Bundle
import { EditorView, basicSetup } from 'codemirror';
import { EditorState, Compartment } from '@codemirror/state';
import { keymap } from '@codemirror/view';
import { StreamLanguage } from '@codemirror/language';
import { yaml } from '@codemirror/legacy-modes/mode/yaml';
import { indentWithTab } from '@codemirror/commands';

// Create YAML language support
const yamlLanguage = StreamLanguage.define(yaml);

// Export everything
export {
    EditorView,
    EditorState,
    Compartment,
    basicSetup,
    yamlLanguage as yaml,
    keymap,
    indentWithTab
};
`;

await esbuild.build({
    stdin: {
        contents: code,
        resolveDir: process.cwd(),
        loader: 'js',
    },
    bundle: true,
    format: 'esm',
    minify: true,
    outfile: 'codemirror/codemirror.bundle.js',
    external: [],
});

console.log('CodeMirror bundle created successfully!');
