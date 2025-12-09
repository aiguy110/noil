pub enum BackpressureStrategy {
    Block,
    Drop,
    BufferInMemory,
}

pub struct BackpressureHandler {
    // TODO: Define backpressure handling
}

impl BackpressureHandler {
    pub fn new(_strategy: BackpressureStrategy) -> Self {
        todo!("implement BackpressureHandler")
    }
}
