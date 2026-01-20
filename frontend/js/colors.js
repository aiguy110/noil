/**
 * Color management for fiber types and sources
 */
class ColorManager {
    constructor() {
        this.storageKey = 'noil-colors';
        this.defaultFiberColors = {
            'request_trace': '#4a9eff',
            'simple_log': '#ff6b6b',
            'nginx_all': '#51cf66',
        };
        this.defaultSourceColors = {
            'nginx_access': '#1e3a8a',
            'application_log': '#134e4a',
            'program1': '#581c87',
            'program2': '#831843',
            'simple_service': '#713f12',
        };

        // Load saved colors or use defaults
        this.colors = this.loadColors();
    }

    loadColors() {
        try {
            const saved = localStorage.getItem(this.storageKey);
            if (saved) {
                return JSON.parse(saved);
            }
        } catch (error) {
            console.error('Failed to load colors from localStorage:', error);
        }

        return {
            fiberTypes: { ...this.defaultFiberColors },
            sources: { ...this.defaultSourceColors },
        };
    }

    saveColors() {
        try {
            localStorage.setItem(this.storageKey, JSON.stringify(this.colors));
        } catch (error) {
            console.error('Failed to save colors to localStorage:', error);
        }
    }

    getFiberTypeColor(fiberType) {
        if (!this.colors.fiberTypes[fiberType]) {
            // Generate a new color for unknown fiber types
            this.colors.fiberTypes[fiberType] = this.generateColor(fiberType);
            this.saveColors();
        }
        return this.colors.fiberTypes[fiberType];
    }

    getSourceColor(sourceId) {
        if (!this.colors.sources[sourceId]) {
            // Generate a new color for unknown sources
            this.colors.sources[sourceId] = this.generateColor(sourceId);
            this.saveColors();
        }
        return this.colors.sources[sourceId];
    }

    setFiberTypeColor(fiberType, color) {
        this.colors.fiberTypes[fiberType] = color;
        this.saveColors();
    }

    setSourceColor(sourceId, color) {
        this.colors.sources[sourceId] = color;
        this.saveColors();
    }

    resetToDefaults() {
        this.colors = {
            fiberTypes: { ...this.defaultFiberColors },
            sources: { ...this.defaultSourceColors },
        };
        this.saveColors();
    }

    getAllFiberTypeColors() {
        return { ...this.colors.fiberTypes };
    }

    getAllSourceColors() {
        return { ...this.colors.sources };
    }

    // Generate a deterministic color from a string
    generateColor(str) {
        let hash = 0;
        for (let i = 0; i < str.length; i++) {
            hash = str.charCodeAt(i) + ((hash << 5) - hash);
        }

        const hue = Math.abs(hash % 360);
        const saturation = 60 + (Math.abs(hash % 20));
        const lightness = 45 + (Math.abs(hash % 15));

        return `hsl(${hue}, ${saturation}%, ${lightness}%)`;
    }

    // Convert HSL to hex for color picker
    hslToHex(hsl) {
        const match = hsl.match(/hsl\((\d+),\s*(\d+)%,\s*(\d+)%\)/);
        if (!match) return hsl;

        const h = parseInt(match[1]) / 360;
        const s = parseInt(match[2]) / 100;
        const l = parseInt(match[3]) / 100;

        let r, g, b;

        if (s === 0) {
            r = g = b = l;
        } else {
            const hue2rgb = (p, q, t) => {
                if (t < 0) t += 1;
                if (t > 1) t -= 1;
                if (t < 1/6) return p + (q - p) * 6 * t;
                if (t < 1/2) return q;
                if (t < 2/3) return p + (q - p) * (2/3 - t) * 6;
                return p;
            };

            const q = l < 0.5 ? l * (1 + s) : l + s - l * s;
            const p = 2 * l - q;
            r = hue2rgb(p, q, h + 1/3);
            g = hue2rgb(p, q, h);
            b = hue2rgb(p, q, h - 1/3);
        }

        const toHex = x => {
            const hex = Math.round(x * 255).toString(16);
            return hex.length === 1 ? '0' + hex : hex;
        };

        return `#${toHex(r)}${toHex(g)}${toHex(b)}`;
    }
}

// Export singleton instance
const colorManager = new ColorManager();
