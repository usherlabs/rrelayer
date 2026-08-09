use axum::{body::Body, http::Request, Router};
use hyper_util::{
    rt::{TokioExecutor, TokioIo},
    server::conn::auto::Builder,
    service::TowerToHyperService,
};
use std::{
    fs::Metadata,
    future::Future,
    io,
    os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::{
    net::{UnixListener, UnixStream},
    sync::{broadcast, watch},
    task::JoinSet,
};
use tower::ServiceExt;
use tracing::warn;

#[derive(Debug)]
pub(crate) struct UnixSocketGuard {
    path: PathBuf,
    device: u64,
    inode: u64,
}

impl UnixSocketGuard {
    fn owns(&self, metadata: &Metadata) -> bool {
        metadata.file_type().is_socket()
            && metadata.dev() == self.device
            && metadata.ino() == self.inode
    }
}

impl Drop for UnixSocketGuard {
    fn drop(&mut self) {
        let Ok(metadata) = std::fs::symlink_metadata(&self.path) else {
            return;
        };

        if self.owns(&metadata) {
            if let Err(error) = std::fs::remove_file(&self.path) {
                warn!(path = %self.path.display(), %error, "failed to remove Unix socket");
            }
        }
    }
}

pub(crate) async fn bind(path: &Path) -> io::Result<(UnixListener, UnixSocketGuard)> {
    if let Ok(metadata) = std::fs::symlink_metadata(path) {
        if !metadata.file_type().is_socket() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("refusing to replace non-socket path {}", path.display()),
            ));
        }

        probe_existing_socket(path, UnixStream::connect(path), Duration::from_millis(250)).await?;
    }

    let listener = UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    let metadata = std::fs::symlink_metadata(path)?;
    let guard =
        UnixSocketGuard { path: path.to_path_buf(), device: metadata.dev(), inode: metadata.ino() };

    Ok((listener, guard))
}

async fn probe_existing_socket<F>(path: &Path, probe: F, timeout: Duration) -> io::Result<()>
where
    F: Future<Output = io::Result<UnixStream>>,
{
    match tokio::time::timeout(timeout, probe).await {
        Err(_) | Ok(Ok(_)) => Err(io::Error::new(
            io::ErrorKind::AddrInUse,
            format!("Unix socket {} may still be accepting connections", path.display()),
        )),
        Ok(Err(error))
            if matches!(
                error.kind(),
                io::ErrorKind::ConnectionRefused | io::ErrorKind::NotFound
            ) =>
        {
            std::fs::remove_file(path)
        }
        Ok(Err(error)) => Err(error),
    }
}

pub(crate) async fn serve(listener: UnixListener, app: Router) -> io::Result<()> {
    serve_with_shutdown(listener, app, crate::shutdown::subscribe_to_shutdown()).await
}

async fn serve_with_shutdown(
    listener: UnixListener,
    app: Router,
    mut shutdown: broadcast::Receiver<()>,
) -> io::Result<()> {
    let mut connections = JoinSet::new();
    let (connection_shutdown, _) = watch::channel(false);

    loop {
        let stream = tokio::select! {
            result = listener.accept() => result?.0,
            _ = shutdown.recv() => break,
            completed = connections.join_next(), if !connections.is_empty() => {
                if let Some(Err(error)) = completed {
                    warn!(%error, "Unix socket connection task failed");
                }
                continue;
            }
        };
        let service = app
            .clone()
            .map_request(|request: Request<hyper_v1::body::Incoming>| request.map(Body::new));
        let mut shutdown_connection = connection_shutdown.subscribe();

        connections.spawn(async move {
            let io = TokioIo::new(stream);
            let service = TowerToHyperService::new(service);
            let builder = Builder::new(TokioExecutor::new());
            let connection = builder.serve_connection_with_upgrades(io, service);
            tokio::pin!(connection);
            let shutdown_requested = async {
                let result = shutdown_connection.wait_for(|requested| *requested).await;
                drop(result);
            };

            tokio::select! {
                result = &mut connection => {
                    if let Err(error) = result {
                        warn!(%error, "failed to serve Unix socket connection");
                    }
                }
                _ = shutdown_requested => {
                    connection.as_mut().graceful_shutdown();
                    if let Err(error) = connection.await {
                        warn!(%error, "failed to gracefully close Unix socket connection");
                    }
                }
            }
        });
    }

    let _ = connection_shutdown.send(true);
    while let Some(result) = connections.join_next().await {
        if let Err(error) = result {
            warn!(%error, "Unix socket connection task failed during shutdown");
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::routing::get;
    use std::os::unix::fs::{FileTypeExt, PermissionsExt};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use uuid::Uuid;

    fn socket_path(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("rrelayer-{label}-{}.sock", Uuid::new_v4()))
    }

    #[tokio::test]
    async fn refuses_to_replace_a_regular_file() {
        let path = socket_path("regular-file");
        std::fs::write(&path, "keep me").unwrap();

        let error = bind(&path).await.unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "keep me");
        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn refuses_to_replace_an_active_socket() {
        let path = socket_path("active");
        let active = tokio::net::UnixListener::bind(&path).unwrap();

        let error = bind(&path).await.unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::AddrInUse);
        assert!(std::fs::symlink_metadata(&path).unwrap().file_type().is_socket());
        drop(active);
        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn replaces_a_stale_socket_and_applies_owner_only_permissions() {
        let path = socket_path("stale");
        drop(std::os::unix::net::UnixListener::bind(&path).unwrap());

        let (_listener, guard) = bind(&path).await.unwrap();
        let metadata = std::fs::symlink_metadata(&path).unwrap();

        assert!(metadata.file_type().is_socket());
        assert_eq!(metadata.permissions().mode() & 0o777, 0o600);

        drop(guard);
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn serves_http_over_the_unix_socket() {
        let path = socket_path("http");
        let (listener, guard) = bind(&path).await.unwrap();
        let app = Router::new().route("/health", get(|| async { "healthy" }));
        let server = tokio::spawn(serve(listener, app));
        let mut stream = UnixStream::connect(&path).await.unwrap();

        stream
            .write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).await.unwrap();

        assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
        assert!(response.ends_with("healthy"), "{response}");

        server.abort();
        drop(guard);
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn a_timed_out_activity_probe_preserves_the_existing_socket() {
        let path = socket_path("probe-timeout");
        let active = tokio::net::UnixListener::bind(&path).unwrap();

        let error = probe_existing_socket(
            &path,
            std::future::pending(),
            std::time::Duration::from_millis(1),
        )
        .await
        .unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::AddrInUse);
        assert!(std::fs::symlink_metadata(&path).unwrap().file_type().is_socket());
        drop(active);
        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn shutdown_stops_accepting_and_drains_connection_tasks() {
        let path = socket_path("shutdown");
        let (listener, guard) = bind(&path).await.unwrap();
        let coordinator = std::sync::Arc::new(crate::shutdown::ShutdownCoordinator::new_for_test());
        let shutdown = coordinator.subscribe();
        let app = Router::new().route("/health", get(|| async { "healthy" }));
        let server = tokio::spawn(serve_with_shutdown(listener, app, shutdown));
        let stream = UnixStream::connect(&path).await.unwrap();

        assert!(coordinator.request_shutdown(std::time::Duration::from_millis(20)).await);
        tokio::time::timeout(std::time::Duration::from_millis(100), server)
            .await
            .expect("Unix server must stop after shutdown")
            .unwrap()
            .unwrap();

        drop(stream);
        drop(guard);
    }
}
