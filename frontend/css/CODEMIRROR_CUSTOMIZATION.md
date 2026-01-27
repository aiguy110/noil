# CodeMirror Customization Guide

## Quick Start: Changing the Theme

The default CodeMirror theme is integrated into `style.css` and designed to match Noil's dark interface. To use a different color scheme:

### Option 1: Use a Pre-made Theme

1. Open `frontend/css/codemirror-themes.css`
2. Find the theme you want (High Contrast Dark, Light, Monokai, Solarized Dark)
3. Uncomment the entire theme block
4. Add this line to `index.html` in the `<head>` section:
   ```html
   <link rel="stylesheet" href="css/codemirror-themes.css">
   ```
5. Refresh your browser

### Option 2: Edit the Default Theme

The default theme is defined in `style.css` starting around line 1556. You can modify these CSS classes:

#### Main Editor Colors
```css
.cm-editor {
    background-color: var(--bg-tertiary);  /* Editor background */
    color: var(--text-primary);             /* Default text color */
}

.cm-content {
    color: var(--text-primary);             /* Content text color */
}
```

#### Syntax Highlighting Colors

| Element | Class | Default Color | Purpose |
|---------|-------|---------------|---------|
| Strings | `.cm-string`, `.cm-atom` | #ce9178 (orange) | String values, quoted text |
| Numbers | `.cm-number` | #b5cea8 (light green) | Numeric values |
| Keywords | `.cm-keyword` | #569cd6 (blue) | YAML keywords |
| Properties | `.cm-property`, `.cm-variable` | #9cdcfe (light blue) | Property names |
| Comments | `.cm-comment` | #6a9955 (green) | Comments (#) |
| Meta | `.cm-meta` | #dcdcaa (yellow) | Special syntax |
| Operators | `.cm-operator` | #d4d4d4 (light gray) | Operators like : - |

#### Gutter and Line Numbers
```css
.cm-gutters {
    background-color: var(--bg-secondary);  /* Line number area background */
    color: var(--text-secondary);           /* Line number text */
}

.cm-lineNumbers .cm-gutterElement {
    color: var(--text-secondary);
    padding-right: 8px;
}
```

#### Selection and Active Line
```css
.cm-activeLine {
    background-color: rgba(255, 255, 255, 0.05);  /* Highlight current line */
}

.cm-selectionBackground {
    background-color: var(--accent-color);        /* Selected text */
    opacity: 0.3;
}
```

## Making Text More Visible

If text is hard to read on the dark background, try these adjustments:

### 1. Increase Overall Brightness
```css
.cm-content {
    color: #ffffff !important;  /* Brighter white */
}
```

### 2. Use High Contrast Colors
```css
.cm-string {
    color: #ffaa44 !important;  /* Brighter orange */
}

.cm-property {
    color: #44ddff !important;  /* Brighter cyan */
}

.cm-keyword {
    color: #8888ff !important;  /* Brighter blue */
}
```

### 3. Adjust Background
```css
.cm-editor {
    background-color: #1a1a1a !important;  /* Darker background for more contrast */
}
```

### 4. Increase Font Size
```css
.cm-editor {
    font-size: 14px !important;  /* Default is 13px */
}
```

### 5. Adjust Line Height for Readability
```css
.cm-editor {
    line-height: 1.8 !important;  /* Default is 1.6 */
}
```

### 6. Brighter Cursor
```css
.cm-cursor {
    border-left-color: #00ff00 !important;  /* Bright green cursor */
    border-left-width: 2px !important;       /* Thicker cursor */
}
```

## Example: High Visibility Setup

Add this to the end of `style.css` for maximum readability:

```css
/* High visibility CodeMirror overrides */
.cm-editor {
    background-color: #0d0d0d !important;
    font-size: 14px !important;
    line-height: 1.8 !important;
}

.cm-content {
    color: #f0f0f0 !important;
}

.cm-string {
    color: #ffaa44 !important;
    font-weight: 500;
}

.cm-property {
    color: #44ddff !important;
    font-weight: 500;
}

.cm-keyword {
    color: #ff6b9d !important;
    font-weight: 600;
}

.cm-comment {
    color: #88cc88 !important;
}

.cm-cursor {
    border-left-color: #00ff00 !important;
    border-left-width: 2px !important;
}
```

## Testing Your Changes

After making changes:
1. Save the CSS file
2. Refresh your browser (Ctrl+F5 or Cmd+Shift+R to clear cache)
3. Navigate to the Fiber Rules page
4. Select a fiber type to see the editor with your new colors

## Available Pre-made Themes

1. **High Contrast Dark** - Brighter colors on very dark background
2. **Light Theme** - For bright environments or daytime use
3. **Monokai** - Popular dark theme with vibrant colors
4. **Solarized Dark** - Carefully designed color palette

All themes are in `codemirror-themes.css` (commented out by default).

## Advanced: Creating Your Own Theme

To create a custom theme:

1. Copy one of the theme blocks from `codemirror-themes.css`
2. Modify the colors to your preference
3. Test and refine
4. Share your theme by creating a new block in `codemirror-themes.css`

Color picker tools: Use browser DevTools color picker or sites like [coolors.co](https://coolors.co) to choose colors.

## Troubleshooting

**Q: My changes aren't showing up**
- Clear browser cache (Ctrl+F5 / Cmd+Shift+R)
- Check browser console for CSS errors
- Verify you're editing the correct file

**Q: Some colors are still hard to see**
- Use `!important` flag to override default styles
- Check that you're targeting the right CSS class
- Increase font-weight for specific elements

**Q: Editor looks broken after changes**
- Revert your changes and test incrementally
- Check for CSS syntax errors (missing semicolons, braces)
- Compare with the original `style.css` from git

## Color Resources

- [VS Code Themes](https://vscodethemes.com/) - Browse color schemes for inspiration
- [Color Contrast Checker](https://webaim.org/resources/contrastchecker/) - Ensure readable contrast ratios
- [Coolors](https://coolors.co/) - Generate color palettes
