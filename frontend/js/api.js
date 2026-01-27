/**
 * API client for Noil backend
 */
class NoilAPI {
    constructor(baseUrl = '') {
        this.baseUrl = baseUrl;
    }

    async request(path, options = {}) {
        const url = `${this.baseUrl}${path}`;
        try {
            const response = await fetch(url, {
                ...options,
                headers: {
                    'Content-Type': 'application/json',
                    ...options.headers,
                },
            });

            if (!response.ok) {
                const error = await response.json();
                throw new Error(error.error?.message || `HTTP ${response.status}`);
            }

            return await response.json();
        } catch (error) {
            console.error(`API request failed: ${path}`, error);
            throw error;
        }
    }

    // Health check
    async health() {
        return this.request('/health');
    }

    // Logs
    async listLogs(params = {}) {
        const query = new URLSearchParams();
        if (params.start) query.append('start', params.start.toISOString());
        if (params.end) query.append('end', params.end.toISOString());
        if (params.source) query.append('source', params.source);
        if (params.limit) query.append('limit', params.limit);
        if (params.offset) query.append('offset', params.offset);

        return this.request(`/api/logs?${query}`);
    }

    async getLog(logId) {
        return this.request(`/api/logs/${logId}`);
    }

    async getLogFibers(logId) {
        return this.request(`/api/logs/${logId}/fibers`);
    }

    // Fibers
    async listFibers(params = {}) {
        const query = new URLSearchParams();
        if (params.type) query.append('type', params.type);
        if (params.closed !== undefined) query.append('closed', params.closed);
        if (params.limit) query.append('limit', params.limit);
        if (params.offset) query.append('offset', params.offset);

        return this.request(`/api/fibers?${query}`);
    }

    async getFiber(fiberId) {
        return this.request(`/api/fibers/${fiberId}`);
    }

    async getFiberLogs(fiberId, params = {}) {
        const query = new URLSearchParams();
        if (params.limit) query.append('limit', params.limit);
        if (params.offset) query.append('offset', params.offset);

        return this.request(`/api/fibers/${fiberId}/logs?${query}`);
    }

    // Get all fiber types with metadata
    async getAllFiberTypes() {
        return this.request('/api/fiber-types');
    }

    // Get all fiber type names (for backwards compatibility)
    async getAllFiberTypeNames() {
        const metadata = await this.getAllFiberTypes();
        return metadata.map(ft => ft.name);
    }

    // Get all source IDs
    async getAllSources() {
        return this.request('/api/sources');
    }

    // Config versioning
    async getCurrentConfig() {
        return this.request('/api/config/current');
    }

    async getConfigHistory(params = {}) {
        const query = new URLSearchParams();
        if (params.limit) query.append('limit', params.limit);
        if (params.offset) query.append('offset', params.offset);
        return this.request(`/api/config/history?${query}`);
    }

    async getConfigVersion(hash) {
        return this.request(`/api/config/versions/${hash}`);
    }

    async updateConfig(yamlContent) {
        return this.request('/api/config', {
            method: 'PUT',
            body: JSON.stringify({ yaml_content: yamlContent }),
        });
    }

    async getConfigDiff(hash1, hash2) {
        return this.request(`/api/config/diff/${hash1}/${hash2}`);
    }

    // Fiber Type Management
    async getFiberType(name) {
        return this.request(`/api/fiber-types/${encodeURIComponent(name)}`);
    }

    async updateFiberType(name, yamlContent) {
        return this.request(`/api/fiber-types/${encodeURIComponent(name)}`, {
            method: 'PUT',
            body: JSON.stringify({ yaml_content: yamlContent }),
        });
    }

    async createFiberType(name, yamlContent) {
        return this.request('/api/fiber-types', {
            method: 'POST',
            body: JSON.stringify({ name, yaml_content: yamlContent }),
        });
    }

    async deleteFiberType(name) {
        return this.request(`/api/fiber-types/${encodeURIComponent(name)}`, {
            method: 'DELETE',
        });
    }

    async hotReloadFiberType(name) {
        return this.request(`/api/fiber-types/${encodeURIComponent(name)}/hot-reload`, {
            method: 'POST',
        });
    }

    // Reprocessing
    async startReprocessing(options = {}) {
        return this.request('/api/reprocess', {
            method: 'POST',
            body: JSON.stringify(options),
        });
    }

    async getReprocessStatus() {
        return this.request('/api/reprocess/status');
    }

    async cancelReprocessing() {
        return this.request('/api/reprocess/cancel', {
            method: 'POST',
        });
    }

    // Working Set Testing
    async testWorkingSet(fiberTypeName, logIds, yamlContent) {
        return this.request(`/api/fiber-types/${encodeURIComponent(fiberTypeName)}/test-working-set`, {
            method: 'POST',
            body: JSON.stringify({
                log_ids: logIds,
                yaml_content: yamlContent,
                include_margin: true
            }),
        });
    }
}

// Export singleton instance
const api = new NoilAPI();
