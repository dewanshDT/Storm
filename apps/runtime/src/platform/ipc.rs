//! The host's local channel for its bridges and harness reports (81f, D15;
//! AM55 on Windows).
//!
//! Unix: a socket at `run/mcp.sock` in a `0700` directory, itself `0600`.
//! Windows: a named pipe derived from that same path, with a DACL of the
//! host's account and SYSTEM only, refusing remote clients, and created as
//! the first instance so no other process can stand in for it. Either way
//! only this account's processes — the bridges it launched — can connect,
//! and what crosses it is unchanged.

use std::ffi::{OsStr, OsString};
use std::io;
use std::path::Path;

/// What `STORM_MCP_SOCK` carries for the channel at `path`: the socket's path
/// on Unix, the pipe's name on Windows.
pub fn address(path: &Path) -> OsString {
    imp::address(path)
}

/// The listening end, owned by the host.
pub struct Listener(imp::Listener);

/// One accepted connection.
pub type Stream = imp::Stream;

impl Listener {
    /// Binds the channel for `path`, replacing a stale socket a previous host
    /// left behind.
    pub fn bind(path: &Path) -> io::Result<Self> {
        imp::Listener::bind(path).map(Self)
    }

    pub async fn accept(&mut self) -> io::Result<Stream> {
        self.0.accept().await
    }
}

/// The connecting end, a bridge's.
pub type Client = imp::Client;

/// Connects to the channel at `address` (a bridge, from the agent's side).
pub async fn connect(address: &OsStr) -> io::Result<Client> {
    imp::connect(address).await
}

/// Connects without a runtime, for a one-line report (the harness hook).
pub fn connect_blocking(address: &OsStr) -> io::Result<BlockingClient> {
    imp::connect_blocking(address).map(BlockingClient)
}

/// A one-line reporter's end of the channel.
pub struct BlockingClient(imp::BlockingClient);

impl io::Write for BlockingClient {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

impl BlockingClient {
    /// Waits, up to `limit`, for the host to close the connection. A host
    /// that checks who connected (decision 135) does so after reading the
    /// line, and refuses a connector that has already exited.
    pub fn wait_closed(&mut self, limit: std::time::Duration) {
        imp::wait_closed(&mut self.0, limit)
    }
}

#[cfg(unix)]
mod imp {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    pub fn address(path: &Path) -> OsString {
        path.as_os_str().to_owned()
    }

    pub type Stream = tokio::net::UnixStream;

    pub struct Listener(tokio::net::UnixListener);

    impl Listener {
        pub fn bind(path: &Path) -> io::Result<Self> {
            crate::platform::tighten_private_dir(path.parent().expect("socket has a parent"))?;
            let _ = std::fs::remove_file(path);
            let listener = tokio::net::UnixListener::bind(path)?;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
            Ok(Self(listener))
        }

        pub async fn accept(&mut self) -> io::Result<Stream> {
            Ok(self.0.accept().await?.0)
        }
    }

    pub type Client = tokio::net::UnixStream;

    pub async fn connect(address: &OsStr) -> io::Result<Client> {
        tokio::net::UnixStream::connect(address).await
    }

    pub type BlockingClient = std::os::unix::net::UnixStream;

    pub fn connect_blocking(address: &OsStr) -> io::Result<BlockingClient> {
        std::os::unix::net::UnixStream::connect(address)
    }

    pub fn wait_closed(stream: &mut BlockingClient, limit: std::time::Duration) {
        use std::io::Read;
        let deadline = std::time::Instant::now() + limit;
        let mut buf = [0u8; 256];
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() || stream.set_read_timeout(Some(left)).is_err() {
                return;
            }
            match stream.read(&mut buf) {
                Ok(0) | Err(_) => return,
                Ok(_) => {}
            }
        }
    }
}

#[cfg(windows)]
mod imp {
    use super::*;
    use tokio::net::windows::named_pipe::{
        ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions,
    };

    /// `\\.\pipe\storm-runtime-<digest>`: one pipe per state directory, so
    /// two hosts on one machine (a test's and the service's) never meet.
    pub fn address(path: &Path) -> OsString {
        let digest = blake3::hash(path.as_os_str().as_encoded_bytes()).to_hex();
        format!(r"\\.\pipe\storm-runtime-{}", &digest[..24]).into()
    }

    pub type Stream = NamedPipeServer;

    pub struct Listener {
        name: OsString,
        next: NamedPipeServer,
    }

    fn create(name: &OsString, first: bool) -> io::Result<NamedPipeServer> {
        let security = crate::platform::OwnerOnly::for_pipe()?;
        let mut options = ServerOptions::new();
        options
            .first_pipe_instance(first)
            .reject_remote_clients(true);
        // SAFETY: `security` holds a valid SECURITY_ATTRIBUTES whose
        // descriptor lives until it is dropped, after this call returns.
        unsafe { options.create_with_security_attributes_raw(name, security.attributes()) }
    }

    impl Listener {
        pub fn bind(path: &Path) -> io::Result<Self> {
            let name = address(path);
            let next = create(&name, true)?;
            Ok(Self { name, next })
        }

        pub async fn accept(&mut self) -> io::Result<Stream> {
            self.next.connect().await?;
            let fresh = create(&self.name, false)?;
            Ok(std::mem::replace(&mut self.next, fresh))
        }
    }

    const ERROR_PIPE_BUSY: i32 = 231;

    pub type Client = NamedPipeClient;

    pub async fn connect(address: &OsStr) -> io::Result<Client> {
        for _ in 0..50 {
            match ClientOptions::new().open(address) {
                Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) => {
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
                other => return other,
            }
        }
        ClientOptions::new().open(address)
    }

    pub type BlockingClient = std::fs::File;

    /// Nothing to wait for: Windows hosts do not check who connected (AM80).
    pub fn wait_closed(_stream: &mut BlockingClient, _limit: std::time::Duration) {}

    pub fn connect_blocking(address: &OsStr) -> io::Result<BlockingClient> {
        for _ in 0..50 {
            match std::fs::OpenOptions::new().write(true).open(address) {
                Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) => {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                other => return other,
            }
        }
        std::fs::OpenOptions::new().write(true).open(address)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    /// A line from a bridge reaches the host, and the answer comes back, the
    /// same on a socket and on a pipe.
    #[tokio::test]
    async fn a_line_crosses_the_channel_both_ways() {
        let state = tempfile::tempdir().unwrap();
        let path = state.path().join("run").join("mcp.sock");
        let mut listener = Listener::bind(&path).unwrap();
        let address = address(&path);
        let server = tokio::spawn(async move {
            let stream = listener.accept().await.unwrap();
            let (read, mut write) = tokio::io::split(stream);
            let mut line = String::new();
            BufReader::new(read).read_line(&mut line).await.unwrap();
            write
                .write_all(format!("echo {line}").as_bytes())
                .await
                .unwrap();
            listener
        });
        let client = connect(&address).await.unwrap();
        let (read, mut write) = tokio::io::split(client);
        write.write_all(b"hello\n").await.unwrap();
        let mut answer = String::new();
        BufReader::new(read).read_line(&mut answer).await.unwrap();
        assert_eq!(answer, "echo hello\n");
        drop(server.await.unwrap());
    }

    /// AM55: the pipe is the first instance, so nothing else can create it
    /// first and stand in for the host.
    #[cfg(windows)]
    #[tokio::test]
    async fn no_one_else_can_stand_in_for_the_pipe() {
        let state = tempfile::tempdir().unwrap();
        let path = state.path().join("run").join("mcp.sock");
        let _host = Listener::bind(&path).unwrap();
        assert!(Listener::bind(&path).is_err());
        let name = address(&path).to_string_lossy().into_owned();
        assert!(name.starts_with(r"\\.\pipe\storm-runtime-"), "{name}");
    }
}
