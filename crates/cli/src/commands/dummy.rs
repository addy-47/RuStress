use rustress_dummy::server::DummyServer;

/// Start the built-in test HTTP server.
pub async fn run(port: u16) -> anyhow::Result<()> {
    let server = DummyServer::new(port);
    server.run().await
}
