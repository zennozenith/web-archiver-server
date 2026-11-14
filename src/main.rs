//! Server entry point. The implementation lives in the library crate so the
//! administrative CLI can reuse it (and so integration tests can link it).

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    web_archiver::init_tracing();
    web_archiver::run().await
}
