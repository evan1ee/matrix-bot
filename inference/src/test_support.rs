//! Helpers shared by this crate's tests.

/// Starts a throwaway server on loopback and returns its base URL, e.g.
/// `http://127.0.0.1:54321`. Uses axum and tokio, which this crate already
/// depends on, rather than a mock-HTTP crate.
pub(crate) async fn spawn_server(app: axum::Router) -> String {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("ephemeral port should bind");
    let addr = listener
        .local_addr()
        .expect("listener should have an address");
    tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });
    format!("http://{}", addr)
}
