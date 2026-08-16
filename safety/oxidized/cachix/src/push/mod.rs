use std::io::Write;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;

use crate::api::client::ApiClient;
use crate::api::types::*;
use crate::config::PushCredential;
use crate::nix::sign;

/// Options for pushing store paths.
#[derive(Debug, Clone)]
pub struct PushOptions {
    pub compression_method: CompressionMethod,
    pub compression_level: u32,
    pub chunk_size: usize,
    pub num_concurrent_chunks: usize,
    pub jobs: usize,
    pub omit_deriver: bool,
}

impl Default for PushOptions {
    fn default() -> Self {
        Self {
            compression_method: CompressionMethod::Zstd,
            compression_level: 2,
            chunk_size: 32 * 1024 * 1024, // 32 MiB
            num_concurrent_chunks: 4,
            jobs: 8,
            omit_deriver: false,
        }
    }
}

/// Push a single store path to the cache.
pub async fn push_store_path(
    api: &ApiClient,
    cache_name: &str,
    store_path: &str,
    credential: &PushCredential,
    opts: &PushOptions,
) -> Result<()> {
    tracing::info!("pushing {store_path}");

    // Get path info
    let path_info = crate::nix::store::query_path_info(store_path)
        .await
        .with_context(|| format!("failed to get path info for {store_path}"))?;

    // Dump NAR
    let mut child = crate::nix::store::dump_nar(store_path).await?;
    let stdout = child
        .stdout
        .take()
        .context("no stdout from nix-store --dump")?;

    // Read the entire NAR into memory, computing hashes as we go
    let mut nar_reader = tokio::io::BufReader::new(stdout);
    let mut nar_data = Vec::new();
    let mut nar_hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];

    loop {
        let n = nar_reader.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        nar_hasher.update(&buf[..n]);
        nar_data.extend_from_slice(&buf[..n]);
    }

    let status = child.wait().await?;
    if !status.success() {
        anyhow::bail!("nix-store --dump failed for {store_path}");
    }

    let nar_hash_bytes = nar_hasher.finalize();
    let nar_hash = format!("sha256:{}", sign::nix_base32_encode(&nar_hash_bytes));
    let nar_size = nar_data.len() as u64;

    // Compress
    let compressed = compress_nar(&nar_data, &opts.compression_method, opts.compression_level)?;
    let file_size = compressed.len() as u64;

    // Hash compressed data (hex SHA256)
    let file_hash = {
        let mut hasher = Sha256::new();
        hasher.update(&compressed);
        format!("sha256:{:x}", hasher.finalize())
    };

    // Build references as basenames
    let references: Vec<String> = path_info
        .references
        .iter()
        .filter_map(|r| r.strip_prefix("/nix/store/").map(|s| s.to_string()))
        .collect();

    // Get deriver
    let deriver = if opts.omit_deriver {
        String::new()
    } else {
        path_info
            .deriver
            .as_deref()
            .and_then(|d| d.strip_prefix("/nix/store/").map(String::from))
            .unwrap_or_default()
    };

    let store_hash = sign::store_path_hash(store_path)?;
    let store_suffix = sign::store_path_suffix(store_path)?;

    // Sign if we have a signing key
    let sig = if let Some(sk) = credential.signing_key() {
        let fingerprint =
            sign::narinfo_fingerprint(store_path, &nar_hash, nar_size, &path_info.references);
        Some(sign::sign_narinfo(sk, &fingerprint)?)
    } else {
        None
    };

    let narinfo_create = NarInfoCreate {
        c_store_hash: store_hash,
        c_store_suffix: store_suffix,
        c_nar_hash: nar_hash,
        c_nar_size: nar_size,
        c_file_hash: file_hash,
        c_file_size: file_size,
        c_references: references,
        c_deriver: deriver,
        c_sig: sig,
    };

    // Multipart upload
    let upload_resp = api
        .create_multipart_upload(cache_name, &opts.compression_method)
        .await?;

    let nar_id = upload_resp.nar_id;
    let upload_id = upload_resp.upload_id;

    match do_multipart_upload(
        api,
        cache_name,
        &nar_id,
        &upload_id,
        &compressed,
        opts,
        narinfo_create,
    )
    .await
    {
        Ok(()) => Ok(()),
        Err(e) => {
            tracing::warn!("upload failed, aborting: {e}");
            let _ = api
                .abort_multipart_upload(cache_name, &nar_id, &upload_id)
                .await;
            Err(e)
        }
    }
}

async fn do_multipart_upload(
    api: &ApiClient,
    cache_name: &str,
    nar_id: &uuid::Uuid,
    upload_id: &str,
    data: &[u8],
    opts: &PushOptions,
    narinfo_create: NarInfoCreate,
) -> Result<()> {
    let chunks: Vec<&[u8]> = data.chunks(opts.chunk_size).collect();
    let mut completed_parts = Vec::with_capacity(chunks.len());

    // Upload chunks with limited concurrency
    let semaphore = Arc::new(tokio::sync::Semaphore::new(opts.num_concurrent_chunks));

    let mut tasks = Vec::new();
    for (i, chunk) in chunks.iter().enumerate() {
        let part_number = (i + 1) as u32;
        let chunk_data = chunk.to_vec();
        let api = api.clone();
        let cache_name = cache_name.to_string();
        let nar_id = *nar_id;
        let upload_id = upload_id.to_string();
        let sem = semaphore.clone();

        tasks.push(tokio::spawn(async move {
            let _permit = sem.acquire().await.unwrap();

            let md5_hash = {
                use md5::Digest;
                let mut hasher = md5::Md5::new();
                hasher.update(&chunk_data);
                hasher.finalize()
            };
            let content_md5 = BASE64.encode(md5_hash.as_slice());

            let part_url = api
                .get_upload_part_url(&cache_name, &nar_id, &upload_id, part_number, &content_md5)
                .await?;

            let etag = api
                .upload_part(&part_url.upload_url, chunk_data, &content_md5)
                .await?;

            Ok::<CompletedPart, anyhow::Error>(CompletedPart {
                part_number,
                e_tag: etag,
            })
        }));
    }

    for task in tasks {
        completed_parts.push(task.await??);
    }

    completed_parts.sort_by_key(|p| p.part_number);

    let completion = CompletedMultipartUpload {
        parts: if completed_parts.is_empty() {
            None
        } else {
            Some(completed_parts)
        },
        nar_info_create: narinfo_create,
    };

    api.complete_multipart_upload(cache_name, nar_id, upload_id, &completion)
        .await?;

    Ok(())
}

/// Compress NAR data.
fn compress_nar(data: &[u8], method: &CompressionMethod, level: u32) -> Result<Vec<u8>> {
    match method {
        CompressionMethod::Zstd => {
            let compressed = zstd::encode_all(std::io::Cursor::new(data), level as i32)
                .context("zstd compression failed")?;
            Ok(compressed)
        }
        CompressionMethod::Xz => {
            let mut compressed = Vec::new();
            let mut encoder = xz2::write::XzEncoder::new(&mut compressed, level);
            encoder.write_all(data).context("xz compression failed")?;
            encoder.finish().context("xz finish failed")?;
            Ok(compressed)
        }
    }
}

/// Push multiple store paths, filtering out those already in the cache.
pub async fn push_paths(
    api: &ApiClient,
    cache_name: &str,
    store_paths: &[String],
    credential: &PushCredential,
    opts: &PushOptions,
) -> Result<()> {
    // Compute closure
    tracing::info!("computing closure for {} paths", store_paths.len());
    let closure = crate::nix::store::query_closure(store_paths).await?;
    tracing::info!("closure contains {} paths", closure.len());

    // Get store hashes
    let store_hashes: Vec<String> = closure
        .iter()
        .filter_map(|p| sign::store_path_hash(p).ok())
        .collect();

    // Find missing paths
    let missing_hashes = api.narinfo_bulk(cache_name, &store_hashes).await?;
    let missing_set: std::collections::HashSet<&str> =
        missing_hashes.iter().map(|s| s.as_str()).collect();

    let paths_to_push: Vec<&String> = closure
        .iter()
        .filter(|p| {
            sign::store_path_hash(p)
                .map(|h| missing_set.contains(h.as_str()))
                .unwrap_or(false)
        })
        .collect();

    tracing::info!(
        "{} paths to push ({} already cached)",
        paths_to_push.len(),
        closure.len() - paths_to_push.len()
    );

    // Push with concurrency limit
    let semaphore = Arc::new(tokio::sync::Semaphore::new(opts.jobs));
    let failure_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut tasks = Vec::new();

    for path in paths_to_push {
        let api = api.clone();
        let cache_name = cache_name.to_string();
        let path = path.clone();
        let credential = credential.clone();
        let opts = opts.clone();
        let sem = semaphore.clone();
        let failures = failure_count.clone();

        tasks.push(tokio::spawn(async move {
            let _permit = sem.acquire().await.unwrap();
            if let Err(e) = push_store_path(&api, &cache_name, &path, &credential, &opts).await {
                tracing::error!("failed to push {path}: {e:#}");
                failures.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }));
    }

    for task in tasks {
        task.await?;
    }

    let failures = failure_count.load(std::sync::atomic::Ordering::Relaxed);
    if failures > 0 {
        bail!("{failures} path(s) failed to push");
    }

    Ok(())
}
