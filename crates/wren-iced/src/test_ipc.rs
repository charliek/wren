//! Private, bounded transport; only App::update handles the requests.

use std::hash::{Hash, Hasher};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iced::futures::{SinkExt, Stream};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::oneshot;

pub const MAX_FRAME_BYTES: usize = 1024 * 1024;
const MAX_CONNECTIONS: usize = 4;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

type Reply = Result<Value, String>;

#[derive(Debug, Clone)]
pub struct Endpoint {
    listener: Arc<std::os::unix::net::UnixListener>,
    guard: Arc<SocketGuard>,
}

impl Hash for Endpoint {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.guard.path.hash(state);
        self.guard.device.hash(state);
        self.guard.inode.hash(state);
    }
}

impl Endpoint {
    pub fn bind(path: &Path) -> Result<Self, String> {
        if !path.is_absolute() || path.as_os_str().as_bytes().len() > 100 {
            return Err("test socket must be absolute and at most 100 bytes".into());
        }
        let parent = path.parent().ok_or("test socket has no parent")?;
        let metadata = std::fs::symlink_metadata(parent).map_err(|error| error.to_string())?;
        if !metadata.is_dir()
            || metadata.mode() & 0o777 != 0o700
            || metadata.uid() != rustix::process::geteuid().as_raw()
        {
            return Err("test socket parent must be an owned, non-symlink 0700 directory".into());
        }
        let listener = std::os::unix::net::UnixListener::bind(path)
            .map_err(|error| format!("bind test socket: {error}"))?;
        let metadata = std::fs::symlink_metadata(path).map_err(|error| error.to_string())?;
        let guard = Arc::new(SocketGuard {
            path: path.into(),
            device: metadata.dev(),
            inode: metadata.ino(),
        });
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(|error| error.to_string())?;
        listener
            .set_nonblocking(true)
            .map_err(|error| error.to_string())?;
        Ok(Self {
            listener: Arc::new(listener),
            guard,
        })
    }

    pub fn subscription(&self) -> iced::Subscription<Incoming> {
        iced::Subscription::run_with(self.clone(), stream)
    }
}

#[derive(Debug, Clone)]
pub enum Incoming {
    Request(Arc<Request>),
    Failed(String),
}

pub struct Request {
    pub id: u64,
    pub op: String,
    pub params: Value,
    reply: Mutex<Option<oneshot::Sender<Reply>>>,
}

impl Request {
    pub fn respond(&self, result: Reply) {
        if let Some(reply) = self
            .reply
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()
        {
            let _ = reply.send(result);
        }
    }
}

impl std::fmt::Debug for Request {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Request")
            .field("id", &self.id)
            .field("op", &self.op)
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
struct SocketGuard {
    path: PathBuf,
    device: u64,
    inode: u64,
}

impl Drop for SocketGuard {
    fn drop(&mut self) {
        if let Ok(metadata) = std::fs::symlink_metadata(&self.path) {
            if metadata.file_type().is_socket()
                && metadata.dev() == self.device
                && metadata.ino() == self.inode
            {
                let _ = std::fs::remove_file(&self.path);
            }
        }
    }
}

fn stream(endpoint: &Endpoint) -> impl Stream<Item = Incoming> {
    let endpoint = endpoint.clone();
    iced::stream::channel(32, async move |mut output| {
        let listener = endpoint
            .listener
            .try_clone()
            .and_then(tokio::net::UnixListener::from_std);
        let listener = match listener {
            Ok(listener) => listener,
            Err(error) => {
                let _ = output.send(Incoming::Failed(error.to_string())).await;
                return;
            }
        };
        // Dropping the subscription aborts all connection tasks before releasing the socket guard.
        let mut tasks = tokio::task::JoinSet::new();
        loop {
            tokio::select! {
                accepted = listener.accept(), if tasks.len() < MAX_CONNECTIONS => {
                    match accepted {
                        Ok((connection, _)) => { tasks.spawn(connection_loop(connection, output.clone())); }
                        Err(error) => { let _ = output.send(Incoming::Failed(error.to_string())).await; break; }
                    }
                }
                finished = tasks.join_next(), if !tasks.is_empty() => {
                    if let Some(Err(error)) = finished {
                        let _ = output.send(Incoming::Failed(error.to_string())).await;
                        break;
                    }
                }
            }
        }
    })
}

async fn connection_loop(
    connection: UnixStream,
    mut output: iced::futures::channel::mpsc::Sender<Incoming>,
) {
    let (reader, mut writer) = connection.into_split();
    let mut reader = BufReader::new(reader);
    loop {
        let mut frame = Vec::new();
        let read = tokio::time::timeout(
            Duration::from_secs(10),
            (&mut reader)
                .take((MAX_FRAME_BYTES + 1) as u64)
                .read_until(b'\n', &mut frame),
        )
        .await;
        match read {
            Ok(Ok(0)) | Ok(Err(_)) | Err(_) => return,
            Ok(Ok(_)) => {}
        }
        if frame.len() > MAX_FRAME_BYTES || frame.last() != Some(&b'\n') {
            let _ = write_reply(
                &mut writer,
                0,
                Err("oversized or incomplete request frame".into()),
            )
            .await;
            return;
        }
        let (id, op, params) = match parse_frame(&frame) {
            Ok(request) => request,
            Err(error) => {
                let _ = write_reply(&mut writer, 0, Err(error)).await;
                return;
            }
        };
        let (reply, receive) = oneshot::channel();
        let request = Arc::new(Request {
            id,
            op,
            params,
            reply: Mutex::new(Some(reply)),
        });
        let deadline = tokio::time::Instant::now() + REQUEST_TIMEOUT;
        if !matches!(
            tokio::time::timeout_at(deadline, output.send(Incoming::Request(request))).await,
            Ok(Ok(()))
        ) {
            return;
        }
        let result = match tokio::time::timeout_at(deadline, receive).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err("application dropped the request".into()),
            Err(_) => Err("application response deadline exceeded".into()),
        };
        if write_reply(&mut writer, id, result).await.is_err() {
            return;
        }
    }
}

fn parse_frame(frame: &[u8]) -> Result<(u64, String, Value), String> {
    let value: Value =
        serde_json::from_slice(frame).map_err(|error| format!("invalid JSON: {error}"))?;
    let id = value
        .get("id")
        .and_then(Value::as_u64)
        .ok_or("id must be an unsigned integer")?;
    let op = value
        .get("op")
        .and_then(Value::as_str)
        .filter(|op| !op.is_empty())
        .ok_or("op must be a non-empty string")?;
    let params = value.get("params").cloned().unwrap_or_else(|| json!({}));
    if !params.is_object() {
        return Err("params must be an object".into());
    }
    Ok((id, op.into(), params))
}

async fn write_reply(
    writer: &mut tokio::net::unix::OwnedWriteHalf,
    id: u64,
    result: Reply,
) -> std::io::Result<()> {
    let value = match result {
        Ok(result) => json!({"id":id,"ok":true,"result":result}),
        Err(error) => json!({"id":id,"ok":false,"error":error}),
    };
    let mut bytes = serde_json::to_vec(&value).map_err(std::io::Error::other)?;
    if bytes.len() >= MAX_FRAME_BYTES {
        bytes = serde_json::to_vec(
            &json!({"id":id,"ok":false,"error":"response frame exceeds maximum size"}),
        )?;
    }
    bytes.push(b'\n');
    tokio::time::timeout(Duration::from_secs(5), writer.write_all(&bytes))
        .await
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "reply deadline exceeded"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn directory() -> tempfile::TempDir {
        let directory = tempfile::tempdir().unwrap();
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        directory
    }

    #[test]
    fn bind_does_not_require_a_tokio_runtime_and_cleans_its_socket() {
        let directory = directory();
        let path = directory.path().join("s.sock");
        let endpoint = Endpoint::bind(&path).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
        assert!(Endpoint::bind(&path).is_err());
        drop(endpoint);
        assert!(!path.exists());
    }

    #[test]
    fn existing_files_and_foreign_replacements_are_not_removed() {
        let directory = directory();
        let path = directory.path().join("s.sock");
        std::fs::write(&path, "sentinel").unwrap();
        assert!(Endpoint::bind(&path).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "sentinel");
        std::fs::remove_file(&path).unwrap();
        let endpoint = Endpoint::bind(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        std::fs::write(&path, "replacement").unwrap();
        drop(endpoint);
        assert_eq!(std::fs::read_to_string(path).unwrap(), "replacement");
    }

    #[test]
    fn endpoint_instances_are_independent_and_parent_must_be_private() {
        let directory = directory();
        let first = Endpoint::bind(&directory.path().join("one.sock")).unwrap();
        let second = Endpoint::bind(&directory.path().join("two.sock")).unwrap();
        drop((first, second));
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(Endpoint::bind(&directory.path().join("three.sock")).is_err());
    }

    #[test]
    fn protocol_refuses_invalid_types_and_utf8() {
        for frame in [
            b"{}".as_slice(),
            b"[]",
            b"{\"id\":true,\"op\":\"x\"}",
            b"{\"id\":1,\"op\":\"x\",\"params\":[]}",
            b"\xff",
        ] {
            assert!(parse_frame(frame).is_err());
        }
        let (id, op, params) = parse_frame(b"{\"id\":1,\"op\":\"identify\"}\n").unwrap();
        assert_eq!((id, op.as_str(), params), (1, "identify", json!({})));
    }

    #[test]
    fn subscription_identity_is_stable_per_endpoint() {
        use std::collections::hash_map::DefaultHasher;
        let directory = directory();
        let endpoint = Endpoint::bind(&directory.path().join("s.sock")).unwrap();
        let hash = |value: &Endpoint| {
            let mut state = DefaultHasher::new();
            value.hash(&mut state);
            state.finish()
        };
        assert_eq!(hash(&endpoint), hash(&endpoint.clone()));
    }
}
