//! Helpers shared by the `services/*` client tests.

/// Starts a throwaway server on an ephemeral port and returns its base URL.
/// Uses axum and tokio, both already backend dependencies, rather than adding
/// a mock-HTTP crate.
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
