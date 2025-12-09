use std::net::SocketAddr;

pub struct WebServer {
    // TODO: Define web server
}

impl WebServer {
    pub fn new(_listen_addr: SocketAddr) -> Self {
        todo!("implement WebServer")
    }

    pub async fn serve(&self) -> Result<(), Box<dyn std::error::Error>> {
        todo!("implement serve")
    }
}
