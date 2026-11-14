//! Server entry point. The implementation lives in the library crate so the
//! administrative CLI can reuse it (and so integration tests can link it).

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Hold the guard until `main` returns: dropping it flushes buffered file
    // logs. Never bind it to a bare `_`, which would drop it immediately.
    let _telemetry = web_archiver::init_tracing()?;

    if let Err(error) = web_archiver::run().await {
        // The runtime prints the returned error to stderr as plain text. Log it
        // too, so a fatal startup or serve error reaches the same sinks (file,
        // aggregator) as every other event.
        tracing::error!(error = format!("{error:#}"), "fatal error");
        return Err(error);
    }
    Ok(())
}
