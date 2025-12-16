use tokio::sync::mpsc;

pub type Sender<T> = mpsc::Sender<T>;
pub type Receiver<T> = mpsc::Receiver<T>;

/// Create a bounded channel with the specified buffer size
pub fn create_channel<T>(buffer_size: usize) -> (Sender<T>, Receiver<T>) {
    mpsc::channel(buffer_size)
}
