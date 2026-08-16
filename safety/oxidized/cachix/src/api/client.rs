use anyhow::{Context, Result, bail};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use uuid::Uuid;

use super::types::*;

/// Cachix API client.
#[derive(Debug, Clone)]
pub struct ApiClient {
    client: reqwest::Client,
    base_url: String,
    auth_token: Option<String>,
}

impl ApiClient {
    pub fn new(base_url: &str, auth_token: Option<&str>) -> Result<Self> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        if let Some(token) = auth_token {
            headers.insert(
                AUTHORIZATION,
                HeaderValue::from_str(&format!("Bearer {token}")).context("invalid auth token")?,
            );
        }

        let client = reqwest::Client::builder()
            .default_headers(headers)
            .build()
            .context("failed to build HTTP client")?;

        Ok(Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            auth_token: auth_token.map(String::from),
        })
    }

    pub fn auth_token(&self) -> Option<&str> {
        self.auth_token.as_deref()
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    fn api_url(&self, path: &str) -> String {
        format!("{}/api/v1{}", self.base_url, path)
    }

    /// Get binary cache metadata.
    pub async fn get_cache(&self, name: &str) -> Result<BinaryCache> {
        let url = self.api_url(&format!("/cache/{name}"));
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .context("failed to get cache info")?;
        if !resp.status().is_success() {
            bail!(
                "failed to get cache {name}: {} {}",
                resp.status(),
                resp.text().await.unwrap_or_default()
            );
        }
        resp.json().await.context("failed to parse cache response")
    }

    /// Get nix-cache-info for a cache.
    pub async fn get_nix_cache_info(&self, name: &str) -> Result<NixCacheInfo> {
        let url = self.api_url(&format!("/cache/{name}/nix-cache-info"));
        let resp = self.client.get(&url).send().await?;
        if !resp.status().is_success() {
            bail!("failed to get nix-cache-info: {}", resp.status());
        }
        resp.json().await.context("failed to parse nix-cache-info")
    }

    /// Check if a narinfo exists (HEAD request).
    pub async fn head_narinfo(&self, cache: &str, store_hash: &str) -> Result<bool> {
        let url = self.api_url(&format!("/cache/{cache}/{store_hash}.narinfo"));
        let resp = self.client.head(&url).send().await?;
        Ok(resp.status().is_success())
    }

    /// Bulk query: given store hashes, return which are missing.
    pub async fn narinfo_bulk(&self, cache: &str, store_hashes: &[String]) -> Result<Vec<String>> {
        let url = self.api_url(&format!("/cache/{cache}/narinfo"));
        let resp = self
            .client
            .post(&url)
            .json(store_hashes)
            .send()
            .await
            .context("failed to bulk query narinfo")?;
        if !resp.status().is_success() {
            bail!("bulk narinfo query failed: {}", resp.status());
        }
        resp.json()
            .await
            .context("failed to parse bulk narinfo response")
    }

    /// Create a multipart NAR upload.
    pub async fn create_multipart_upload(
        &self,
        cache: &str,
        compression: &CompressionMethod,
    ) -> Result<CreateMultipartUploadResponse> {
        let url = self.api_url(&format!(
            "/cache/{cache}/multipart-nar?compression={}",
            compression
        ));
        let resp = self.client.post(&url).send().await?;
        if !resp.status().is_success() {
            bail!(
                "failed to create multipart upload: {} {}",
                resp.status(),
                resp.text().await.unwrap_or_default()
            );
        }
        resp.json()
            .await
            .context("failed to parse multipart upload response")
    }

    /// Get a presigned URL for uploading a part.
    pub async fn get_upload_part_url(
        &self,
        cache: &str,
        nar_id: &Uuid,
        upload_id: &str,
        part_number: u32,
        content_md5: &str,
    ) -> Result<UploadPartResponse> {
        let url = self.api_url(&format!(
            "/cache/{cache}/multipart-nar/{nar_id}?uploadId={upload_id}&partNumber={part_number}"
        ));
        let body = SigningData {
            content_md5: content_md5.to_string(),
        };
        let resp = self.client.post(&url).json(&body).send().await?;
        if !resp.status().is_success() {
            bail!(
                "failed to get upload part URL: {} {}",
                resp.status(),
                resp.text().await.unwrap_or_default()
            );
        }
        resp.json()
            .await
            .context("failed to parse upload part response")
    }

    /// Upload a part to the presigned URL.
    /// Uses a plain HTTP client without default auth headers since S3
    /// presigned URLs include their own authentication.
    pub async fn upload_part(
        &self,
        upload_url: &str,
        data: Vec<u8>,
        content_md5: &str,
    ) -> Result<String> {
        let s3_client = reqwest::Client::new();
        let resp = s3_client
            .put(upload_url)
            .header(CONTENT_TYPE, "application/octet-stream")
            .header("Content-MD5", content_md5)
            .body(data)
            .send()
            .await
            .context("failed to upload part")?;
        if !resp.status().is_success() {
            bail!(
                "part upload failed: {} {}",
                resp.status(),
                resp.text().await.unwrap_or_default()
            );
        }
        let etag = resp
            .headers()
            .get("ETag")
            .or_else(|| resp.headers().get("etag"))
            .context("missing ETag in upload response")?
            .to_str()
            .context("invalid ETag header")?
            .to_string();
        Ok(etag)
    }

    /// Complete a multipart upload.
    pub async fn complete_multipart_upload(
        &self,
        cache: &str,
        nar_id: &Uuid,
        upload_id: &str,
        body: &CompletedMultipartUpload,
    ) -> Result<()> {
        let url = self.api_url(&format!(
            "/cache/{cache}/multipart-nar/{nar_id}/complete?uploadId={upload_id}"
        ));
        let resp = self.client.post(&url).json(body).send().await?;
        if !resp.status().is_success() {
            bail!(
                "failed to complete multipart upload: {} {}",
                resp.status(),
                resp.text().await.unwrap_or_default()
            );
        }
        Ok(())
    }

    /// Abort a multipart upload.
    pub async fn abort_multipart_upload(
        &self,
        cache: &str,
        nar_id: &Uuid,
        upload_id: &str,
    ) -> Result<()> {
        let url = self.api_url(&format!(
            "/cache/{cache}/multipart-nar/{nar_id}/abort?uploadId={upload_id}"
        ));
        let resp = self.client.post(&url).send().await?;
        if !resp.status().is_success() {
            tracing::warn!("failed to abort multipart upload: {}", resp.status());
        }
        Ok(())
    }

    /// Upload a signing public key.
    pub async fn upload_signing_key(&self, cache: &str, public_key: &str) -> Result<()> {
        let url = self.api_url(&format!("/cache/{cache}/key"));
        let body = serde_json::json!({ "publicSigningKey": public_key });
        let resp = self.client.post(&url).json(&body).send().await?;
        if !resp.status().is_success() {
            bail!(
                "failed to upload signing key: {} {}",
                resp.status(),
                resp.text().await.unwrap_or_default()
            );
        }
        Ok(())
    }

    /// Create a pin.
    pub async fn create_pin(&self, cache: &str, pin: &PinCreate) -> Result<()> {
        let url = self.api_url(&format!("/cache/{cache}/pin"));
        let resp = self.client.post(&url).json(pin).send().await?;
        if !resp.status().is_success() {
            bail!(
                "failed to create pin: {} {}",
                resp.status(),
                resp.text().await.unwrap_or_default()
            );
        }
        Ok(())
    }

    /// Activate a deployment (V2).
    pub async fn deploy_activate(&self, spec: &DeploySpec) -> Result<DeployActivateResponse> {
        let url = format!("{}/api/v2/deploy/activate", self.base_url);
        let resp = self.client.post(&url).json(spec).send().await?;
        if !resp.status().is_success() {
            bail!(
                "failed to activate deployment: {} {}",
                resp.status(),
                resp.text().await.unwrap_or_default()
            );
        }
        resp.json()
            .await
            .context("failed to parse deploy activate response")
    }

    /// Get deployment status.
    pub async fn get_deployment(&self, deployment_id: &Uuid) -> Result<Deployment> {
        let url = self.api_url(&format!("/deploy/deployment/{deployment_id}"));
        let resp = self.client.get(&url).send().await?;
        if !resp.status().is_success() {
            bail!("failed to get deployment: {}", resp.status());
        }
        resp.json()
            .await
            .context("failed to parse deployment response")
    }
}
