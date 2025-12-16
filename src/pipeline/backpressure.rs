use crate::config::types::BackpressureStrategy;

/// Handler for backpressure based on configuration strategy.
///
/// Currently implements the "block" strategy via bounded channels.
/// Future implementations may add drop and buffer_in_memory strategies.
pub struct BackpressureHandler {
    strategy: BackpressureStrategy,
    buffer_limit: usize,
}

impl BackpressureHandler {
    pub fn new(strategy: BackpressureStrategy, buffer_limit: usize) -> Self {
        Self {
            strategy,
            buffer_limit,
        }
    }

    /// Get the buffer size to use for channels based on strategy
    pub fn channel_buffer_size(&self) -> usize {
        match self.strategy {
            BackpressureStrategy::Block => self.buffer_limit,
            BackpressureStrategy::Drop => self.buffer_limit,
            BackpressureStrategy::BufferInMemory => self.buffer_limit,
        }
    }

    /// Get the backpressure strategy
    pub fn strategy(&self) -> BackpressureStrategy {
        self.strategy
    }
}
