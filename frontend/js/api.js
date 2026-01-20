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

    // Get all fiber types
    async getAllFiberTypes() {
        return this.request('/api/fiber-types');
    }

    // Get all source IDs
    async getAllSources() {
        return this.request('/api/sources');
    }
}

// Export singleton instance
const api = new NoilAPI();
