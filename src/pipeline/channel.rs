use tokio::sync::mpsc;

pub type Sender<T> = mpsc::Sender<T>;
pub type Receiver<T> = mpsc::Receiver<T>;

pub fn create_channel<T>(_buffer_size: usize) -> (Sender<T>, Receiver<T>) {
    // TODO: Create configured channel
    todo!("implement channel creation")
}
