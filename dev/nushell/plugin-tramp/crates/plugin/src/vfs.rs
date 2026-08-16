#![allow(dead_code)]

//! Virtual filesystem layer.
//!
//! The VFS sits between the Nushell plugin commands and the transport
//! backends.  Its responsibilities are:
//!
//! - Resolve a [`TrampPath`] to a concrete [`Backend`] instance.
//! - Connection pooling: reuse open sessions keyed by the full hop chain.
//! - Connection health-checking: verify pooled connections are alive
//!   before reusing them; reconnect on failure.
//! - Stat cache: cache metadata results with a configurable TTL to
//!   avoid redundant remote calls.
//! - Path chaining: compose backends so that e.g. Docker commands run
//!   through an SSH session (`/ssh:host|docker:ctr:/path`).
//! - Provide a **synchronous** API that the Nushell plugin commands can call
//!   (internally it owns a tokio runtime and blocks on async operations).

use bytes::Bytes;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::backend::deploy::{self, DeployResult};
use crate::backend::deploy_exec::{self, ContainerKind, ContainerTarget, ExecDeployResult};
use crate::backend::exec::ExecBackend;
use crate::backend::rpc::RpcBackend;
use crate::backend::rpc_client::RpcClient;
use crate::backend::runner::CommandRunner;
use crate::backend::runner::{LocalRunner, RemoteRunner};
use crate::backend::socket;
use crate::backend::ssh::SshBackend;
use crate::backend::{Backend, DirEntry, ExecResult, Metadata};
use crate::errors::{TrampError, TrampResult};
use crate::protocol::{BackendKind, Hop, TrampPath};

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Default time-to-live for cached stat entries.
const DEFAULT_STAT_TTL: Duration = Duration::from_secs(5);

/// Default time-to-live for cached directory listings.
const DEFAULT_LIST_TTL: Duration = Duration::from_secs(5);

// ---------------------------------------------------------------------------
// Connection key
// ---------------------------------------------------------------------------

/// A single hop's identity, used as a component of [`ConnectionKey`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct HopKey {
    backend: BackendKind,
    user: Option<String>,
    host: String,
    port: Option<u16>,
}

impl From<&Hop> for HopKey {
    fn from(hop: &Hop) -> Self {
        Self {
            backend: hop.backend,
            user: hop.user.clone(),
            host: hop.host.clone(),
            port: hop.port,
        }
    }
}

/// Cache / pool key representing a full (possibly multi-hop) connection chain.
///
/// Two paths that differ only in their `remote_path` share the same
/// `ConnectionKey`; the key captures only the transport hops.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ConnectionKey {
    hops: Vec<HopKey>,
}

impl ConnectionKey {
    fn from_hops(hops: &[Hop]) -> Self {
        Self {
            hops: hops.iter().map(HopKey::from).collect(),
        }
    }

    /// Return `true` if any hop in this key targets `host`.
    fn contains_host(&self, host: &str) -> bool {
        self.hops.iter().any(|h| h.host == host)
    }

    /// The host of the *last* hop — the endpoint that owns the files.
    fn endpoint_host(&self) -> &str {
        self.hops.last().map(|h| h.host.as_str()).unwrap_or("")
    }
}

// ---------------------------------------------------------------------------
// Stat cache
// ---------------------------------------------------------------------------

/// A single cached entry with its insertion timestamp.
#[derive(Debug, Clone)]
struct CacheEntry<T> {
    value: T,
    inserted_at: Instant,
}

impl<T> CacheEntry<T> {
    fn new(value: T) -> Self {
        Self {
            value,
            inserted_at: Instant::now(),
        }
    }

    fn is_expired(&self, ttl: Duration) -> bool {
        self.inserted_at.elapsed() > ttl
    }
}

/// A TTL-based cache for stat (metadata) results keyed by
/// `(ConnectionKey, remote_path)`.
struct StatCache {
    entries: HashMap<(ConnectionKey, String), CacheEntry<Metadata>>,
    ttl: Duration,
}

impl StatCache {
    fn new(ttl: Duration) -> Self {
        Self {
            entries: HashMap::new(),
            ttl,
        }
    }

    /// Get a cached entry if it exists and has not expired.
    fn get(&mut self, key: &ConnectionKey, path: &str) -> Option<Metadata> {
        let cache_key = (key.clone(), path.to_string());
        if let Some(entry) = self.entries.get(&cache_key) {
            if !entry.is_expired(self.ttl) {
                return Some(entry.value.clone());
            }
            // Expired — remove lazily.
            self.entries.remove(&cache_key);
        }
        None
    }

    fn insert(&mut self, key: &ConnectionKey, path: &str, value: Metadata) {
        self.entries
            .insert((key.clone(), path.to_string()), CacheEntry::new(value));
    }

    fn invalidate(&mut self, key: &ConnectionKey, path: &str) {
        self.entries.remove(&(key.clone(), path.to_string()));
    }

    /// Invalidate all entries whose path starts with `prefix`.
    fn invalidate_prefix(&mut self, key: &ConnectionKey, prefix: &str) {
        self.entries
            .retain(|(k, p), _| !(k == key && p.starts_with(prefix)));
    }

    fn invalidate_connection(&mut self, key: &ConnectionKey) {
        self.entries.retain(|(k, _), _| k != key);
    }

    /// Remove all expired entries (garbage collection).
    fn evict_expired(&mut self) {
        self.entries.retain(|_, e| !e.is_expired(self.ttl));
    }

    fn clear(&mut self) {
        self.entries.clear();
    }

    fn len(&self) -> usize {
        self.entries.len()
    }
}

/// A TTL-based cache for directory listings.
struct ListCache {
    entries: HashMap<(ConnectionKey, String), CacheEntry<Vec<DirEntry>>>,
    ttl: Duration,
}

impl ListCache {
    fn new(ttl: Duration) -> Self {
        Self {
            entries: HashMap::new(),
            ttl,
        }
    }

    fn get(&mut self, key: &ConnectionKey, path: &str) -> Option<Vec<DirEntry>> {
        let cache_key = (key.clone(), path.to_string());
        if let Some(entry) = self.entries.get(&cache_key) {
            if !entry.is_expired(self.ttl) {
                return Some(entry.value.clone());
            }
            self.entries.remove(&cache_key);
        }
        None
    }

    fn insert(&mut self, key: &ConnectionKey, path: &str, value: Vec<DirEntry>) {
        self.entries
            .insert((key.clone(), path.to_string()), CacheEntry::new(value));
    }

    fn invalidate(&mut self, key: &ConnectionKey, path: &str) {
        self.entries.remove(&(key.clone(), path.to_string()));
    }

    fn invalidate_prefix(&mut self, key: &ConnectionKey, prefix: &str) {
        self.entries
            .retain(|(k, p), _| !(k == key && p.starts_with(prefix)));
    }

    fn invalidate_connection(&mut self, key: &ConnectionKey) {
        self.entries.retain(|(k, _), _| k != key);
    }

    fn clear(&mut self) {
        self.entries.clear();
    }
}

// ---------------------------------------------------------------------------
// VFS
// ---------------------------------------------------------------------------

/// The virtual filesystem manager.
///
/// Holds a tokio runtime (created once) and a pool of open backend
/// connections.  All public methods are synchronous — they block the
/// calling thread on the internal runtime.
pub struct Vfs {
    runtime: tokio::runtime::Runtime,
    /// Connection pool keyed by the full hop chain.
    ///
    /// The `Arc<dyn Backend>` is shared so that multiple concurrent calls
    /// can reuse the same underlying session.
    pool: Mutex<HashMap<ConnectionKey, Arc<dyn Backend>>>,
    /// Stat (metadata) cache with TTL.
    stat_cache: Mutex<StatCache>,
    /// Directory listing cache with TTL.
    list_cache: Mutex<ListCache>,
}

impl Vfs {
    /// Create a new VFS with its own tokio runtime.
    pub fn new() -> TrampResult<Self> {
        Self::new_with_ttl(DEFAULT_STAT_TTL, DEFAULT_LIST_TTL)
    }

    /// Create a new VFS with custom cache TTLs.
    pub fn new_with_ttl(stat_ttl: Duration, list_ttl: Duration) -> TrampResult<Self> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|e| TrampError::Internal(format!("failed to create tokio runtime: {e}")))?;

        Ok(Self {
            runtime,
            pool: Mutex::new(HashMap::new()),
            stat_cache: Mutex::new(StatCache::new(stat_ttl)),
            list_cache: Mutex::new(ListCache::new(list_ttl)),
        })
    }

    /// Update the TTL for both stat and list caches.
    ///
    /// Existing cached entries retain their original insertion time, so
    /// they will be re-evaluated against the new TTL on next access.
    pub fn set_cache_ttl(&self, ttl: Duration) {
        if let Ok(mut cache) = self.stat_cache.lock() {
            cache.ttl = ttl;
        }
        if let Ok(mut cache) = self.list_cache.lock() {
            cache.ttl = ttl;
        }
    }

    // -----------------------------------------------------------------------
    // Chain resolution
    // -----------------------------------------------------------------------

    /// Build a backend for a single hop, optionally chained through a
    /// `parent` backend from a previous hop.
    async fn connect_hop(
        hop: &Hop,
        parent: Option<&Arc<dyn Backend>>,
    ) -> TrampResult<Arc<dyn Backend>> {
        match hop.backend {
            BackendKind::Ssh => {
                if parent.is_some() {
                    return Err(TrampError::Internal(
                        "SSH-through-SSH chaining is not supported directly. \
                         Use ProxyJump in ~/.ssh/config for jump hosts instead."
                            .into(),
                    ));
                }
                let ssh = SshBackend::connect(&hop.host, hop.user.as_deref(), hop.port).await?;
                let host = hop.host.clone();

                // Attempt to deploy and start the RPC agent for lower-latency
                // native operations.  On failure, fall back to the shell-parsing
                // SshBackend transparently.
                let session = Arc::clone(ssh.session());
                let sftp = ssh.sftp();
                match deploy::deploy_and_start(session, sftp).await {
                    DeployResult::Ready(mut agent) => {
                        let stdin = agent.take_stdin();
                        let stdout = agent.take_stdout();
                        if let (Some(stdin), Some(stdout)) = (stdin, stdout) {
                            let client = RpcClient::new(stdout, stdin);
                            // Verify the agent is alive before committing.
                            match client.ping().await {
                                Ok(()) => {
                                    return Ok(Arc::new(RpcBackend::new(client, host)));
                                }
                                Err(e) => {
                                    eprintln!(
                                        "tramp: agent ping failed for {}, falling back to SSH: {e}",
                                        host
                                    );
                                }
                            }
                        } else {
                            eprintln!(
                                "tramp: agent started but stdin/stdout not available for {}, falling back to SSH",
                                host
                            );
                        }
                    }
                    DeployResult::Fallback(reason) => {
                        eprintln!("tramp: agent deployment skipped for {}: {reason}", host);
                    }
                }

                // Fallback: use the shell-parsing SSH backend.
                Ok(Arc::new(ssh))
            }
            BackendKind::Docker => {
                let is_standalone = parent.is_none();
                let runner: Arc<dyn CommandRunner> = match parent {
                    None => Arc::new(LocalRunner),
                    Some(p) => Arc::new(RemoteRunner::new(Arc::clone(p))),
                };

                // For standalone Docker containers (no parent / local runner),
                // attempt to deploy the RPC agent inside the container for
                // lower-latency native operations.
                if is_standalone {
                    let target = ContainerTarget {
                        kind: ContainerKind::Docker,
                        name: hop.host.clone(),
                        user: hop.user.clone(),
                        k8s_container: None,
                    };
                    match deploy_exec::deploy_and_start_in_container(runner.as_ref(), &target).await
                    {
                        ExecDeployResult::Ready(backend) => {
                            // Agent is deployed — try upgrading to a direct
                            // TCP socket connection for lower overhead (avoids
                            // the `docker exec -i` process in the middle).
                            if let Some(tcp_backend) = socket::start_docker_tcp_agent(
                                &hop.host,
                                deploy_exec::CONTAINER_AGENT_PATH,
                                socket::DEFAULT_AGENT_PORT,
                            )
                            .await
                            {
                                return Ok(tcp_backend);
                            }
                            // TCP upgrade failed — use the pipe-based RPC
                            // backend (still much faster than shell parsing).
                            return Ok(backend);
                        }
                        ExecDeployResult::Fallback(reason) => {
                            eprintln!(
                                "tramp: agent deployment skipped for docker:{}: {reason}",
                                hop.host
                            );
                        }
                    }
                }

                Ok(Arc::new(ExecBackend::docker(
                    runner,
                    &hop.host,
                    hop.user.as_deref(),
                )))
            }
            BackendKind::Kubernetes => {
                let is_standalone = parent.is_none();
                let runner: Arc<dyn CommandRunner> = match parent {
                    None => Arc::new(LocalRunner),
                    Some(p) => Arc::new(RemoteRunner::new(Arc::clone(p))),
                };

                // For standalone Kubernetes pods (no parent / local runner),
                // attempt to deploy the RPC agent inside the pod.
                if is_standalone {
                    let target = ContainerTarget {
                        kind: ContainerKind::Kubernetes,
                        name: hop.host.clone(),
                        user: None,
                        // In our URI format, the user field doubles as the
                        // container name for K8s multi-container pods.
                        k8s_container: hop.user.clone(),
                    };
                    match deploy_exec::deploy_and_start_in_container(runner.as_ref(), &target).await
                    {
                        ExecDeployResult::Ready(backend) => {
                            // Agent is deployed — try upgrading to a direct
                            // TCP socket via kubectl port-forward for lower
                            // overhead.
                            if let Some(tcp_backend) = socket::start_k8s_tcp_agent(
                                &hop.host,
                                hop.user.as_deref(),
                                deploy_exec::CONTAINER_AGENT_PATH,
                                socket::DEFAULT_AGENT_PORT,
                            )
                            .await
                            {
                                return Ok(tcp_backend);
                            }
                            // TCP upgrade failed — use the pipe-based RPC
                            // backend (still much faster than shell parsing).
                            return Ok(backend);
                        }
                        ExecDeployResult::Fallback(reason) => {
                            eprintln!(
                                "tramp: agent deployment skipped for k8s:{}: {reason}",
                                hop.host
                            );
                        }
                    }
                }

                Ok(Arc::new(ExecBackend::kubernetes(
                    runner,
                    &hop.host,
                    hop.user.as_deref(),
                )))
            }
            BackendKind::Sudo => {
                let runner: Arc<dyn CommandRunner> = match parent {
                    None => Arc::new(LocalRunner),
                    Some(p) => Arc::new(RemoteRunner::new(Arc::clone(p))),
                };
                // For sudo, the "host" field is the target user.
                Ok(Arc::new(ExecBackend::sudo(runner, &hop.host)))
            }
        }
    }

    /// Build a (possibly chained) backend from a sequence of hops.
    ///
    /// Each hop is layered on top of the previous one:
    ///
    /// ```text
    /// /ssh:myvm|docker:ctr:/path
    ///   hop 0: SSH to myvm          → SshBackend
    ///   hop 1: docker exec ctr …    → ExecBackend(RemoteRunner(SshBackend))
    /// ```
    async fn build_chain(hops: &[Hop]) -> TrampResult<Arc<dyn Backend>> {
        if hops.is_empty() {
            return Err(TrampError::Internal("no hops in path".into()));
        }

        let mut current: Option<Arc<dyn Backend>> = None;

        for hop in hops {
            current = Some(Self::connect_hop(hop, current.as_ref()).await?);
        }

        // Safety: we checked that hops is non-empty above.
        Ok(current.unwrap())
    }

    /// Get or create a backend connection for the given hop chain.
    ///
    /// If a pooled connection exists it is health-checked first.  Stale
    /// connections are dropped and a fresh chain is built transparently.
    async fn get_or_connect(
        pool: &Mutex<HashMap<ConnectionKey, Arc<dyn Backend>>>,
        hops: &[Hop],
    ) -> TrampResult<(ConnectionKey, Arc<dyn Backend>)> {
        let key = ConnectionKey::from_hops(hops);

        // Fast path: check if we already have a connection.
        let existing = {
            let pool_guard = pool
                .lock()
                .map_err(|e| TrampError::Internal(format!("pool lock poisoned: {e}")))?;
            pool_guard.get(&key).cloned()
        };

        if let Some(backend) = existing {
            // Health-check the pooled connection.
            match backend.check().await {
                Ok(()) => return Ok((key, backend)),
                Err(_) => {
                    // Connection is stale — remove and reconnect.
                    if let Ok(mut pool_guard) = pool.lock() {
                        pool_guard.remove(&key);
                    }
                }
            }
        }

        // Slow path: build a new chain.
        let backend = Self::build_chain(hops).await?;

        // Store in pool.
        {
            let mut pool_guard = pool
                .lock()
                .map_err(|e| TrampError::Internal(format!("pool lock poisoned: {e}")))?;
            pool_guard.insert(key.clone(), Arc::clone(&backend));
        }

        Ok((key, backend))
    }

    /// Resolve a [`TrampPath`] to a `(connection_key, backend, remote_path)` triple.
    fn resolve(&self, path: &TrampPath) -> TrampResult<(ConnectionKey, Arc<dyn Backend>, String)> {
        let pool = &self.pool;
        let (key, backend) = self
            .runtime
            .block_on(Self::get_or_connect(pool, &path.hops))?;
        Ok((key, backend, path.remote_path.clone()))
    }

    // -----------------------------------------------------------------------
    // Cache helpers
    // -----------------------------------------------------------------------

    /// Invalidate caches that may be affected by a write to `path` under
    /// the given connection.
    fn invalidate_for_write(&self, key: &ConnectionKey, path: &str) {
        // Invalidate the exact stat entry.
        if let Ok(mut cache) = self.stat_cache.lock() {
            cache.invalidate(key, path);
        }
        // Invalidate the parent directory listing.
        if let Some(parent) = parent_dir(path)
            && let Ok(mut cache) = self.list_cache.lock()
        {
            cache.invalidate(key, parent);
        }
    }

    /// Invalidate caches that may be affected by a delete of `path` under
    /// the given connection.
    fn invalidate_for_delete(&self, key: &ConnectionKey, path: &str) {
        if let Ok(mut cache) = self.stat_cache.lock() {
            cache.invalidate(key, path);
            // Also invalidate anything under path (in case it was a directory).
            cache.invalidate_prefix(key, path);
        }
        if let Ok(mut cache) = self.list_cache.lock() {
            cache.invalidate(key, path);
            cache.invalidate_prefix(key, path);
            if let Some(parent) = parent_dir(path) {
                cache.invalidate(key, parent);
            }
        }
    }

    // -----------------------------------------------------------------------
    // Public synchronous API
    // -----------------------------------------------------------------------

    /// Read the contents of a remote file.
    pub fn read(&self, path: &TrampPath) -> TrampResult<Bytes> {
        let (_key, backend, remote_path) = self.resolve(path)?;
        self.runtime.block_on(backend.read(&remote_path))
    }

    /// Write data to a remote file, creating or truncating it.
    pub fn write(&self, path: &TrampPath, data: Bytes) -> TrampResult<()> {
        let (key, backend, remote_path) = self.resolve(path)?;
        self.runtime.block_on(backend.write(&remote_path, data))?;
        self.invalidate_for_write(&key, &remote_path);
        Ok(())
    }

    /// List entries in a remote directory.
    pub fn list(&self, path: &TrampPath) -> TrampResult<Vec<DirEntry>> {
        let (key, backend, remote_path) = self.resolve(path)?;

        // Check list cache first.
        if let Ok(mut cache) = self.list_cache.lock()
            && let Some(entries) = cache.get(&key, &remote_path)
        {
            return Ok(entries);
        }

        let entries = self.runtime.block_on(backend.list(&remote_path))?;

        // Populate the list cache.
        if let Ok(mut cache) = self.list_cache.lock() {
            cache.insert(&key, &remote_path, entries.clone());
        }

        Ok(entries)
    }

    /// Get metadata for a remote path.
    pub fn stat(&self, path: &TrampPath) -> TrampResult<Metadata> {
        let (key, backend, remote_path) = self.resolve(path)?;

        // Check stat cache first.
        if let Ok(mut cache) = self.stat_cache.lock()
            && let Some(meta) = cache.get(&key, &remote_path)
        {
            return Ok(meta);
        }

        let meta = self.runtime.block_on(backend.stat(&remote_path))?;

        // Populate the stat cache.
        if let Ok(mut cache) = self.stat_cache.lock() {
            cache.insert(&key, &remote_path, meta.clone());
        }

        Ok(meta)
    }

    /// Execute a command on the remote host described by `path`.
    pub fn exec(&self, path: &TrampPath, cmd: &str, args: &[&str]) -> TrampResult<ExecResult> {
        let (_key, backend, _) = self.resolve(path)?;
        self.runtime.block_on(backend.exec(cmd, args))
    }

    /// Delete a remote file.
    pub fn delete(&self, path: &TrampPath) -> TrampResult<()> {
        let (key, backend, remote_path) = self.resolve(path)?;
        self.runtime.block_on(backend.delete(&remote_path))?;
        self.invalidate_for_delete(&key, &remote_path);
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Chunked / streaming I/O
    // -----------------------------------------------------------------------

    /// Get the size of a remote file in bytes.
    ///
    /// Uses the agent's `file.size` RPC when available, otherwise falls
    /// back to a full `stat`.
    pub fn file_size(&self, path: &TrampPath) -> TrampResult<u64> {
        let (_key, backend, remote_path) = self.resolve(path)?;
        self.runtime.block_on(backend.file_size(&remote_path))
    }

    /// Read a byte range from a remote file.
    ///
    /// Returns `(data, eof)` where `data` contains up to `length` bytes
    /// starting at `offset`, and `eof` is `true` when the end of file has
    /// been reached.
    pub fn read_range(
        &self,
        path: &TrampPath,
        offset: u64,
        length: u64,
    ) -> TrampResult<(Vec<u8>, bool)> {
        let (_key, backend, remote_path) = self.resolve(path)?;
        self.runtime
            .block_on(backend.read_range(&remote_path, offset, length))
    }

    /// Write data at a specific byte offset in a remote file.
    ///
    /// When `truncate` is `true` the file is truncated before writing
    /// (use for the first chunk).  Returns the number of bytes written.
    pub fn write_range(
        &self,
        path: &TrampPath,
        offset: u64,
        data: Bytes,
        truncate: bool,
    ) -> TrampResult<u64> {
        let (key, backend, remote_path) = self.resolve(path)?;
        let written =
            self.runtime
                .block_on(backend.write_range(&remote_path, offset, data, truncate))?;
        // Invalidate caches on the first chunk (truncate) since the file
        // is being rewritten.
        if truncate {
            self.invalidate_for_write(&key, &remote_path);
        }
        Ok(written)
    }

    /// Whether the backend for this path supports efficient chunked I/O.
    pub fn supports_streaming(&self, path: &TrampPath) -> bool {
        match self.resolve(path) {
            Ok((_key, backend, _)) => backend.supports_streaming(),
            Err(_) => false,
        }
    }

    /// Check that the remote is reachable (used by `tramp ping`).
    pub fn ping(&self, path: &TrampPath) -> TrampResult<()> {
        let (_key, backend, _) = self.resolve(path)?;
        self.runtime.block_on(backend.check())
    }

    // -----------------------------------------------------------------------
    // Watch operations
    // -----------------------------------------------------------------------

    /// Whether the backend for the given path supports filesystem watching.
    pub fn supports_watch(&self, path: &TrampPath) -> TrampResult<bool> {
        let (_key, backend, _) = self.resolve(path)?;
        Ok(backend.supports_watch())
    }

    /// Start watching a remote path for filesystem changes.
    pub fn watch_add(&self, path: &TrampPath, recursive: bool) -> TrampResult<()> {
        let (_key, backend, remote_path) = self.resolve(path)?;
        self.runtime
            .block_on(backend.watch_add(&remote_path, recursive))
    }

    /// Stop watching a remote path.
    pub fn watch_remove(&self, path: &TrampPath) -> TrampResult<()> {
        let (_key, backend, remote_path) = self.resolve(path)?;
        self.runtime.block_on(backend.watch_remove(&remote_path))
    }

    /// List all active watches on the backend for the given path.
    pub fn watch_list(&self, path: &TrampPath) -> TrampResult<Vec<crate::backend::WatchInfo>> {
        let (_key, backend, _) = self.resolve(path)?;
        self.runtime.block_on(backend.watch_list())
    }

    /// Drain pending filesystem change notifications from the backend.
    pub fn watch_poll(
        &self,
        path: &TrampPath,
    ) -> TrampResult<Vec<crate::backend::WatchNotification>> {
        let (_key, backend, _) = self.resolve(path)?;
        self.runtime.block_on(backend.watch_poll())
    }

    // -----------------------------------------------------------------------
    // PTY operations
    // -----------------------------------------------------------------------

    /// Whether the backend for the given path supports PTY (pseudo-terminal)
    /// operations.
    pub fn supports_pty(&self, path: &TrampPath) -> bool {
        match self.resolve(path) {
            Ok((_key, backend, _)) => backend.supports_pty(),
            Err(_) => false,
        }
    }

    /// Start a process with a pseudo-terminal on the remote host.
    ///
    /// Returns a [`PtyHandle`](crate::backend::PtyHandle) containing the
    /// opaque handle ID and the remote PID.
    pub fn pty_start(
        &self,
        path: &TrampPath,
        program: &str,
        args: &[&str],
        rows: u16,
        cols: u16,
    ) -> TrampResult<crate::backend::PtyHandle> {
        let (_key, backend, _) = self.resolve(path)?;
        self.runtime
            .block_on(backend.pty_start(program, args, rows, cols))
    }

    /// Read available output from a PTY process.
    pub fn pty_read(
        &self,
        path: &TrampPath,
        handle: u64,
    ) -> TrampResult<crate::backend::PtyReadResult> {
        let (_key, backend, _) = self.resolve(path)?;
        self.runtime.block_on(backend.pty_read(handle))
    }

    /// Write data to a PTY process (send to its stdin via the PTY master).
    pub fn pty_write(&self, path: &TrampPath, handle: u64, data: bytes::Bytes) -> TrampResult<()> {
        let (_key, backend, _) = self.resolve(path)?;
        self.runtime.block_on(backend.pty_write(handle, data))
    }

    /// Resize a PTY process's terminal window.
    pub fn pty_resize(
        &self,
        path: &TrampPath,
        handle: u64,
        rows: u16,
        cols: u16,
    ) -> TrampResult<()> {
        let (_key, backend, _) = self.resolve(path)?;
        self.runtime
            .block_on(backend.pty_resize(handle, rows, cols))
    }

    /// Kill a PTY process by handle.
    pub fn pty_kill(&self, path: &TrampPath, handle: u64) -> TrampResult<()> {
        let (_key, backend, _) = self.resolve(path)?;
        self.runtime.block_on(backend.pty_kill(handle))
    }

    // -----------------------------------------------------------------------
    // Connection management
    // -----------------------------------------------------------------------

    /// Return the number of currently pooled connections.
    pub fn connection_count(&self) -> usize {
        self.pool.lock().map(|p| p.len()).unwrap_or(0)
    }

    /// Drop all pooled connections and clear all caches.
    pub fn disconnect_all(&self) {
        if let Ok(mut pool) = self.pool.lock() {
            pool.clear();
        }
        if let Ok(mut cache) = self.stat_cache.lock() {
            cache.clear();
        }
        if let Ok(mut cache) = self.list_cache.lock() {
            cache.clear();
        }
    }

    /// Drop pooled connections for a specific host.
    ///
    /// Any connection whose hop chain mentions `host` (in any position)
    /// is removed, along with its cached data.
    pub fn disconnect_host(&self, host: &str) {
        if let Ok(mut pool) = self.pool.lock() {
            pool.retain(|key, _| !key.contains_host(host));
        }

        // Invalidate caches for any connection key mentioning this host.
        if let Ok(mut cache) = self.stat_cache.lock() {
            cache.entries.retain(|(k, _), _| !k.contains_host(host));
        }
        if let Ok(mut cache) = self.list_cache.lock() {
            cache.entries.retain(|(k, _), _| !k.contains_host(host));
        }
    }

    /// Return a snapshot of the currently active connection descriptions
    /// (for display in `tramp connections`).
    pub fn active_connections(&self) -> Vec<String> {
        let pool = match self.pool.lock() {
            Ok(p) => p,
            Err(_) => return Vec::new(),
        };

        pool.iter()
            .map(|(key, backend)| {
                let chain: Vec<String> = key
                    .hops
                    .iter()
                    .map(|h| {
                        let mut s = format!("{}", h.backend);
                        s.push(':');
                        if let Some(ref user) = h.user {
                            s.push_str(user);
                            s.push('@');
                        }
                        s.push_str(&h.host);
                        if let Some(port) = h.port {
                            s.push('#');
                            s.push_str(&port.to_string());
                        }
                        s
                    })
                    .collect();
                let chain_str = chain.join("|");

                // Append the backend's own description for richer output.
                let desc = backend.description();
                if !desc.is_empty() {
                    format!("{chain_str} ({desc})")
                } else {
                    chain_str
                }
            })
            .collect()
    }

    /// Return structured connection info for table display.
    pub fn active_connections_detailed(&self) -> Vec<ConnectionInfo> {
        let pool = match self.pool.lock() {
            Ok(p) => p,
            Err(_) => return Vec::new(),
        };

        pool.keys()
            .map(|key| {
                // Format the chain as "backend:host|backend:host|…"
                let chain: Vec<String> = key
                    .hops
                    .iter()
                    .map(|h| format!("{}:{}", h.backend, h.host))
                    .collect();

                // Use the last hop for the primary display fields.
                let last = key.hops.last();
                ConnectionInfo {
                    backend: last.map(|h| h.backend.to_string()).unwrap_or_default(),
                    user: last.and_then(|h| h.user.clone()),
                    host: last.map(|h| h.host.clone()).unwrap_or_default(),
                    port: last.and_then(|h| h.port),
                    chain: if key.hops.len() > 1 {
                        Some(chain.join("|"))
                    } else {
                        None
                    },
                }
            })
            .collect()
    }

    /// Evict expired entries from all caches (garbage collection).
    /// This can be called periodically or before operations to keep
    /// memory usage bounded.
    pub fn evict_expired_caches(&self) {
        if let Ok(mut cache) = self.stat_cache.lock() {
            cache.evict_expired();
        }
    }

    /// Return current stat cache size (for diagnostics).
    pub fn stat_cache_size(&self) -> usize {
        self.stat_cache.lock().map(|c| c.len()).unwrap_or(0)
    }
}

impl Default for Vfs {
    fn default() -> Self {
        Self::new().expect("failed to create VFS")
    }
}

// ---------------------------------------------------------------------------
// Connection info (for structured output)
// ---------------------------------------------------------------------------

/// Structured info about an active connection, suitable for rendering as
/// a Nushell table row.
pub struct ConnectionInfo {
    pub backend: String,
    pub user: Option<String>,
    pub host: String,
    pub port: Option<u16>,
    /// For chained connections, the full chain description (e.g. `ssh:myvm|docker:ctr`).
    /// `None` for single-hop connections.
    pub chain: Option<String>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Return the parent directory of a remote path, or `None` for `/`.
fn parent_dir(path: &str) -> Option<&str> {
    if path == "/" {
        return None;
    }
    let trimmed = path.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(0) => Some("/"),
        Some(idx) => Some(&trimmed[..idx]),
        None => None,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::BackendKind;

    fn make_path(host: &str, remote: &str) -> TrampPath {
        TrampPath {
            hops: vec![Hop {
                backend: BackendKind::Ssh,
                host: host.to_string(),
                user: None,
                port: None,
            }],
            remote_path: remote.to_string(),
        }
    }

    fn make_key(host: &str) -> ConnectionKey {
        ConnectionKey {
            hops: vec![HopKey {
                backend: BackendKind::Ssh,
                user: None,
                host: host.to_string(),
                port: None,
            }],
        }
    }

    fn make_chained_key(hosts: &[(&str, BackendKind)]) -> ConnectionKey {
        ConnectionKey {
            hops: hosts
                .iter()
                .map(|(h, b)| HopKey {
                    backend: *b,
                    user: None,
                    host: h.to_string(),
                    port: None,
                })
                .collect(),
        }
    }

    // -- ConnectionKey -------------------------------------------------------

    #[test]
    fn connection_key_from_single_hop() {
        let hop = Hop {
            backend: BackendKind::Ssh,
            host: "myvm".into(),
            user: Some("admin".into()),
            port: Some(2222),
        };
        let key = ConnectionKey::from_hops(&[hop]);
        assert_eq!(key.hops.len(), 1);
        assert_eq!(key.hops[0].host, "myvm");
        assert_eq!(key.hops[0].user.as_deref(), Some("admin"));
        assert_eq!(key.hops[0].port, Some(2222));
    }

    #[test]
    fn connection_key_from_multi_hop() {
        let hops = vec![
            Hop {
                backend: BackendKind::Ssh,
                host: "myvm".into(),
                user: None,
                port: None,
            },
            Hop {
                backend: BackendKind::Docker,
                host: "container".into(),
                user: None,
                port: None,
            },
        ];
        let key = ConnectionKey::from_hops(&hops);
        assert_eq!(key.hops.len(), 2);
        assert_eq!(key.endpoint_host(), "container");
    }

    #[test]
    fn connection_key_contains_host() {
        let key = make_chained_key(&[
            ("myvm", BackendKind::Ssh),
            ("container", BackendKind::Docker),
        ]);
        assert!(key.contains_host("myvm"));
        assert!(key.contains_host("container"));
        assert!(!key.contains_host("other"));
    }

    #[test]
    fn connection_key_equality() {
        let key1 = make_key("myvm");
        let key2 = make_key("myvm");
        let key3 = make_key("other");
        assert_eq!(key1, key2);
        assert_ne!(key1, key3);
    }

    // -- Chain validation (standalone Docker/K8s/Sudo) -----------------------

    #[test]
    fn accepts_single_ssh_hop() {
        let path = make_path("myvm", "/etc/config");
        let key = ConnectionKey::from_hops(&path.hops);
        assert_eq!(key.hops.len(), 1);
        assert_eq!(key.hops[0].backend, BackendKind::Ssh);
    }

    #[test]
    fn accepts_single_docker_hop() {
        let path = TrampPath {
            hops: vec![Hop {
                backend: BackendKind::Docker,
                host: "mycontainer".into(),
                user: None,
                port: None,
            }],
            remote_path: "/app".into(),
        };
        let key = ConnectionKey::from_hops(&path.hops);
        assert_eq!(key.hops[0].backend, BackendKind::Docker);
    }

    #[test]
    fn accepts_single_kubernetes_hop() {
        let path = TrampPath {
            hops: vec![Hop {
                backend: BackendKind::Kubernetes,
                host: "mypod".into(),
                user: None,
                port: None,
            }],
            remote_path: "/tmp".into(),
        };
        let key = ConnectionKey::from_hops(&path.hops);
        assert_eq!(key.hops[0].backend, BackendKind::Kubernetes);
    }

    #[test]
    fn accepts_single_sudo_hop() {
        let path = TrampPath {
            hops: vec![Hop {
                backend: BackendKind::Sudo,
                host: "root".into(),
                user: None,
                port: None,
            }],
            remote_path: "/etc/shadow".into(),
        };
        let key = ConnectionKey::from_hops(&path.hops);
        assert_eq!(key.hops[0].backend, BackendKind::Sudo);
    }

    #[test]
    fn accepts_chained_ssh_docker() {
        let path = TrampPath {
            hops: vec![
                Hop {
                    backend: BackendKind::Ssh,
                    host: "myvm".into(),
                    user: None,
                    port: None,
                },
                Hop {
                    backend: BackendKind::Docker,
                    host: "container".into(),
                    user: None,
                    port: None,
                },
            ],
            remote_path: "/app/config.toml".into(),
        };
        let key = ConnectionKey::from_hops(&path.hops);
        assert_eq!(key.hops.len(), 2);
        assert_eq!(key.endpoint_host(), "container");
    }

    #[test]
    fn accepts_triple_chain() {
        let path = TrampPath {
            hops: vec![
                Hop {
                    backend: BackendKind::Ssh,
                    host: "myvm".into(),
                    user: None,
                    port: None,
                },
                Hop {
                    backend: BackendKind::Docker,
                    host: "container".into(),
                    user: None,
                    port: None,
                },
                Hop {
                    backend: BackendKind::Sudo,
                    host: "root".into(),
                    user: None,
                    port: None,
                },
            ],
            remote_path: "/etc/shadow".into(),
        };
        let key = ConnectionKey::from_hops(&path.hops);
        assert_eq!(key.hops.len(), 3);
        assert_eq!(key.endpoint_host(), "root");
    }

    // -- VFS creation --------------------------------------------------------

    #[test]
    fn vfs_creates_successfully() {
        let vfs = Vfs::new().unwrap();
        assert_eq!(vfs.connection_count(), 0);
    }

    #[test]
    fn disconnect_all_clears_pool() {
        let vfs = Vfs::new().unwrap();
        assert_eq!(vfs.connection_count(), 0);
        vfs.disconnect_all();
        assert_eq!(vfs.connection_count(), 0);
    }

    #[test]
    fn active_connections_empty_initially() {
        let vfs = Vfs::new().unwrap();
        assert!(vfs.active_connections().is_empty());
    }

    // -- Stat cache -----------------------------------------------------------

    #[test]
    fn stat_cache_insert_and_get() {
        let mut cache = StatCache::new(Duration::from_secs(60));
        let key = make_key("myvm");
        let meta = Metadata {
            kind: crate::backend::EntryKind::File,
            size: 1024,
            modified: None,
            permissions: Some(644),
            ..Default::default()
        };

        cache.insert(&key, "/etc/config", meta.clone());
        let result = cache.get(&key, "/etc/config");
        assert!(result.is_some());
        assert_eq!(result.unwrap().size, 1024);
    }

    #[test]
    fn stat_cache_miss_for_unknown_path() {
        let mut cache = StatCache::new(Duration::from_secs(60));
        let key = make_key("myvm");
        assert!(cache.get(&key, "/etc/missing").is_none());
    }

    #[test]
    fn stat_cache_expires() {
        let mut cache = StatCache::new(Duration::from_millis(1));
        let key = make_key("myvm");
        let meta = Metadata {
            kind: crate::backend::EntryKind::File,
            size: 100,
            modified: None,
            permissions: None,
            ..Default::default()
        };

        cache.insert(&key, "/tmp/file", meta);
        // Sleep just long enough for the TTL to expire.
        std::thread::sleep(Duration::from_millis(5));
        assert!(cache.get(&key, "/tmp/file").is_none());
    }

    #[test]
    fn stat_cache_invalidate_single() {
        let mut cache = StatCache::new(Duration::from_secs(60));
        let key = make_key("myvm");
        let meta = Metadata {
            kind: crate::backend::EntryKind::File,
            size: 100,
            modified: None,
            permissions: None,
            ..Default::default()
        };

        cache.insert(&key, "/etc/config", meta);
        assert!(cache.get(&key, "/etc/config").is_some());

        cache.invalidate(&key, "/etc/config");
        assert!(cache.get(&key, "/etc/config").is_none());
    }

    #[test]
    fn stat_cache_invalidate_prefix() {
        let mut cache = StatCache::new(Duration::from_secs(60));
        let key = make_key("myvm");
        let meta = Metadata {
            kind: crate::backend::EntryKind::File,
            size: 100,
            modified: None,
            permissions: None,
            ..Default::default()
        };

        cache.insert(&key, "/app/config", meta.clone());
        cache.insert(&key, "/app/data", meta.clone());
        cache.insert(&key, "/etc/other", meta);

        cache.invalidate_prefix(&key, "/app");
        assert!(cache.get(&key, "/app/config").is_none());
        assert!(cache.get(&key, "/app/data").is_none());
        assert!(cache.get(&key, "/etc/other").is_some());
    }

    #[test]
    fn stat_cache_invalidate_connection() {
        let mut cache = StatCache::new(Duration::from_secs(60));
        let key1 = make_key("vm1");
        let key2 = make_key("vm2");
        let meta = Metadata {
            kind: crate::backend::EntryKind::File,
            size: 100,
            modified: None,
            permissions: None,
            ..Default::default()
        };

        cache.insert(&key1, "/etc/config", meta.clone());
        cache.insert(&key2, "/etc/config", meta);

        cache.invalidate_connection(&key1);
        assert!(cache.get(&key1, "/etc/config").is_none());
        assert!(cache.get(&key2, "/etc/config").is_some());
    }

    #[test]
    fn stat_cache_evict_expired() {
        let mut cache = StatCache::new(Duration::from_millis(1));
        let key = make_key("myvm");
        let meta = Metadata {
            kind: crate::backend::EntryKind::File,
            size: 100,
            modified: None,
            permissions: None,
            ..Default::default()
        };

        cache.insert(&key, "/a", meta.clone());
        cache.insert(&key, "/b", meta);
        assert_eq!(cache.len(), 2);

        std::thread::sleep(Duration::from_millis(5));
        cache.evict_expired();
        assert_eq!(cache.len(), 0);
    }

    // -- List cache -----------------------------------------------------------

    #[test]
    fn list_cache_insert_and_get() {
        let mut cache = ListCache::new(Duration::from_secs(60));
        let key = make_key("myvm");
        let entries = vec![DirEntry {
            name: "file.txt".to_string(),
            kind: crate::backend::EntryKind::File,
            size: Some(42),
            modified: None,
            permissions: Some(644),
            ..Default::default()
        }];

        cache.insert(&key, "/app", entries.clone());
        let result = cache.get(&key, "/app");
        assert!(result.is_some());
        assert_eq!(result.unwrap().len(), 1);
    }

    #[test]
    fn list_cache_expires() {
        let mut cache = ListCache::new(Duration::from_millis(1));
        let key = make_key("myvm");
        let entries = vec![];
        cache.insert(&key, "/app", entries);

        std::thread::sleep(Duration::from_millis(5));
        assert!(cache.get(&key, "/app").is_none());
    }

    // -- Chained key cache isolation -----------------------------------------

    #[test]
    fn stat_cache_isolates_by_chain() {
        let mut cache = StatCache::new(Duration::from_secs(60));

        // Same container name but different SSH hosts → different keys.
        let key_vm1 = make_chained_key(&[("vm1", BackendKind::Ssh), ("ctr", BackendKind::Docker)]);
        let key_vm2 = make_chained_key(&[("vm2", BackendKind::Ssh), ("ctr", BackendKind::Docker)]);

        let meta1 = Metadata {
            kind: crate::backend::EntryKind::File,
            size: 100,
            modified: None,
            permissions: None,
            ..Default::default()
        };
        let meta2 = Metadata {
            kind: crate::backend::EntryKind::File,
            size: 200,
            modified: None,
            permissions: None,
            ..Default::default()
        };

        cache.insert(&key_vm1, "/app/config", meta1);
        cache.insert(&key_vm2, "/app/config", meta2);

        assert_eq!(cache.get(&key_vm1, "/app/config").unwrap().size, 100);
        assert_eq!(cache.get(&key_vm2, "/app/config").unwrap().size, 200);
    }

    // -- parent_dir helper ---------------------------------------------------

    #[test]
    fn parent_dir_root() {
        assert_eq!(parent_dir("/"), None);
    }

    #[test]
    fn parent_dir_top_level() {
        assert_eq!(parent_dir("/etc"), Some("/"));
    }

    #[test]
    fn parent_dir_nested() {
        assert_eq!(parent_dir("/etc/config"), Some("/etc"));
    }

    #[test]
    fn parent_dir_deep() {
        assert_eq!(parent_dir("/a/b/c/d"), Some("/a/b/c"));
    }

    #[test]
    fn parent_dir_trailing_slash() {
        assert_eq!(parent_dir("/etc/config/"), Some("/etc"));
    }

    // -- VFS stat cache integration ------------------------------------------

    #[test]
    fn vfs_stat_cache_starts_empty() {
        let vfs = Vfs::new().unwrap();
        assert_eq!(vfs.stat_cache_size(), 0);
    }

    #[test]
    fn vfs_disconnect_all_clears_caches() {
        let vfs = Vfs::new().unwrap();
        // Manually insert something into the stat cache.
        {
            let key = make_key("myvm");
            let meta = Metadata {
                kind: crate::backend::EntryKind::File,
                size: 100,
                modified: None,
                permissions: None,
                ..Default::default()
            };
            let mut cache = vfs.stat_cache.lock().unwrap();
            cache.insert(&key, "/test", meta);
        }
        assert_eq!(vfs.stat_cache_size(), 1);

        vfs.disconnect_all();
        assert_eq!(vfs.stat_cache_size(), 0);
    }

    #[test]
    fn vfs_disconnect_host_clears_host_caches() {
        let vfs = Vfs::new().unwrap();
        let key1 = make_key("vm1");
        let key2 = make_key("vm2");
        let meta = Metadata {
            kind: crate::backend::EntryKind::File,
            size: 100,
            modified: None,
            permissions: None,
            ..Default::default()
        };

        {
            let mut cache = vfs.stat_cache.lock().unwrap();
            cache.insert(&key1, "/test", meta.clone());
            cache.insert(&key2, "/test", meta);
        }
        assert_eq!(vfs.stat_cache_size(), 2);

        vfs.disconnect_host("vm1");
        assert_eq!(vfs.stat_cache_size(), 1);

        // Verify vm2's entry is still present.
        let mut cache = vfs.stat_cache.lock().unwrap();
        assert!(cache.get(&key2, "/test").is_some());
    }

    #[test]
    fn vfs_disconnect_host_clears_chained_caches() {
        let vfs = Vfs::new().unwrap();
        let chained_key =
            make_chained_key(&[("myvm", BackendKind::Ssh), ("ctr", BackendKind::Docker)]);
        let other_key = make_key("othervm");
        let meta = Metadata {
            kind: crate::backend::EntryKind::File,
            size: 100,
            modified: None,
            permissions: None,
            ..Default::default()
        };

        {
            let mut cache = vfs.stat_cache.lock().unwrap();
            cache.insert(&chained_key, "/test", meta.clone());
            cache.insert(&other_key, "/test", meta);
        }
        assert_eq!(vfs.stat_cache_size(), 2);

        // Disconnecting "myvm" should clear the chained connection too.
        vfs.disconnect_host("myvm");
        assert_eq!(vfs.stat_cache_size(), 1);

        let mut cache = vfs.stat_cache.lock().unwrap();
        assert!(cache.get(&other_key, "/test").is_some());
        assert!(cache.get(&chained_key, "/test").is_none());
    }

    #[test]
    fn vfs_new_with_ttl() {
        let vfs = Vfs::new_with_ttl(Duration::from_secs(10), Duration::from_secs(20)).unwrap();
        let key = make_key("myvm");
        let meta = Metadata {
            kind: crate::backend::EntryKind::File,
            size: 42,
            modified: None,
            permissions: None,
            ..Default::default()
        };

        // Insert and verify it's retrievable (TTL is 10s, well within range).
        {
            let mut cache = vfs.stat_cache.lock().unwrap();
            cache.insert(&key, "/test", meta.clone());
            assert!(cache.get(&key, "/test").is_some());
        }
    }

    #[test]
    fn vfs_set_cache_ttl_updates_both_caches() {
        let vfs = Vfs::new().unwrap();
        let key = make_key("myvm");
        let meta = Metadata {
            kind: crate::backend::EntryKind::File,
            size: 100,
            modified: None,
            permissions: None,
            ..Default::default()
        };
        let entries = vec![DirEntry {
            name: "file.txt".to_string(),
            kind: crate::backend::EntryKind::File,
            size: Some(100),
            ..Default::default()
        }];

        // Insert entries with the default TTL (5s).
        {
            let mut stat = vfs.stat_cache.lock().unwrap();
            stat.insert(&key, "/test", meta);
        }
        {
            let mut list = vfs.list_cache.lock().unwrap();
            list.insert(&key, "/dir", entries);
        }

        // Set TTL to zero — all entries should now be expired.
        vfs.set_cache_ttl(Duration::from_secs(0));

        {
            let mut stat = vfs.stat_cache.lock().unwrap();
            assert!(
                stat.get(&key, "/test").is_none(),
                "stat entry should be expired after TTL=0"
            );
        }
        {
            let mut list = vfs.list_cache.lock().unwrap();
            assert!(
                list.get(&key, "/dir").is_none(),
                "list entry should be expired after TTL=0"
            );
        }
    }

    #[test]
    fn vfs_set_cache_ttl_extends_lifetime() {
        let vfs = Vfs::new().unwrap();
        let key = make_key("myvm");
        let meta = Metadata {
            kind: crate::backend::EntryKind::File,
            size: 50,
            modified: None,
            permissions: None,
            ..Default::default()
        };

        {
            let mut cache = vfs.stat_cache.lock().unwrap();
            cache.insert(&key, "/test", meta);
        }

        // Extend TTL to a very large value — entry should still be valid.
        vfs.set_cache_ttl(Duration::from_secs(3600));

        {
            let mut cache = vfs.stat_cache.lock().unwrap();
            assert!(
                cache.get(&key, "/test").is_some(),
                "entry should still be valid with extended TTL"
            );
        }
    }
}
