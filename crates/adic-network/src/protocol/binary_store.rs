use anyhow::{anyhow, Result};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info};

/// Size of each chunk in bytes (1MB)
pub const CHUNK_SIZE: usize = 1024 * 1024;

/// Type alias for chunk cache key (version, chunk_index)
type ChunkCacheKey = (String, u32);

/// Type alias for chunk cache storage
type ChunkCache = Arc<RwLock<HashMap<ChunkCacheKey, Vec<u8>>>>;

/// Type alias for version metadata storage
type VersionMetadataStore = Arc<RwLock<HashMap<String, BinaryMetadata>>>;

/// Manages binary storage and chunking for P2P distribution
pub struct BinaryStore {
    /// Base directory for storing binaries and chunks
    base_dir: PathBuf,

    /// Cache of loaded chunks
    chunk_cache: ChunkCache,

    /// Metadata about available versions
    version_metadata: VersionMetadataStore,
}

#[derive(Debug, Clone)]
pub struct BinaryMetadata {
    pub version: String,
    pub binary_path: PathBuf,
    pub binary_hash: String,
    pub total_chunks: u32,
    pub total_size: u64,
    pub chunk_hashes: Vec<String>,
}

impl BinaryStore {
    /// Create a new binary store
    pub fn new(base_dir: PathBuf) -> Result<Self> {
        // Ensure directories exist
        let binaries_dir = base_dir.join("binaries");
        let chunks_dir = base_dir.join("chunks");

        fs::create_dir_all(&binaries_dir)?;
        fs::create_dir_all(&chunks_dir)?;

        Ok(Self {
            base_dir,
            chunk_cache: Arc::new(RwLock::new(HashMap::new())),
            version_metadata: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    /// Add a binary to the store and create chunks
    pub async fn add_binary(&self, version: String, binary_path: &Path) -> Result<BinaryMetadata> {
        if !binary_path.exists() {
            return Err(anyhow!("Binary file does not exist: {:?}", binary_path));
        }

        info!("Adding binary version {} to store", version);

        // Read the binary
        let binary_data = fs::read(binary_path)?;
        let total_size = binary_data.len() as u64;

        // Calculate overall hash
        let mut hasher = Sha256::new();
        hasher.update(&binary_data);
        let binary_hash = format!("{:x}", hasher.finalize());

        // Calculate number of chunks
        let total_chunks = ((total_size + CHUNK_SIZE as u64 - 1) / CHUNK_SIZE as u64) as u32;

        // Create chunks and calculate their hashes
        let mut chunk_hashes = Vec::new();
        let chunks_dir = self.base_dir.join("chunks").join(&version);
        fs::create_dir_all(&chunks_dir)?;

        for i in 0..total_chunks {
            let start = (i as usize) * CHUNK_SIZE;
            let end = std::cmp::min(start + CHUNK_SIZE, binary_data.len());
            let chunk_data = &binary_data[start..end];

            // Calculate chunk hash
            let mut chunk_hasher = Sha256::new();
            chunk_hasher.update(chunk_data);
            let chunk_hash = format!("{:x}", chunk_hasher.finalize());
            chunk_hashes.push(chunk_hash.clone());

            // Save chunk to disk
            let chunk_path = chunks_dir.join(format!("chunk_{:04}.bin", i));
            fs::write(&chunk_path, chunk_data)?;

            debug!(
                "Created chunk {} for version {}: {} bytes, hash: {}",
                i,
                version,
                chunk_data.len(),
                &chunk_hash[..8]
            );
        }

        // Copy binary to store
        let stored_binary_path = self
            .base_dir
            .join("binaries")
            .join(format!("{}.bin", version));
        fs::copy(binary_path, &stored_binary_path)?;

        // Create metadata
        let metadata = BinaryMetadata {
            version: version.clone(),
            binary_path: stored_binary_path,
            binary_hash,
            total_chunks,
            total_size,
            chunk_hashes,
        };

        // Store metadata
        let mut meta_lock = self.version_metadata.write().await;
        meta_lock.insert(version.clone(), metadata.clone());

        info!(
            "Successfully added binary version {}: {} chunks, {} bytes",
            version, total_chunks, total_size
        );

        Ok(metadata)
    }

    /// Get a specific chunk for a version
    pub async fn get_chunk(&self, version: &str, chunk_index: u32) -> Result<Vec<u8>> {
        // Check cache first
        let cache_key = (version.to_string(), chunk_index);
        let cache = self.chunk_cache.read().await;
        if let Some(cached) = cache.get(&cache_key) {
            debug!("Serving chunk {}/{} from cache", version, chunk_index);
            return Ok(cached.clone());
        }
        drop(cache);

        // Load from disk
        let chunk_path = self
            .base_dir
            .join("chunks")
            .join(version)
            .join(format!("chunk_{:04}.bin", chunk_index));

        if !chunk_path.exists() {
            return Err(anyhow!("Chunk not found: {}/{}", version, chunk_index));
        }

        let chunk_data = fs::read(&chunk_path)?;

        // Add to cache
        let mut cache = self.chunk_cache.write().await;
        cache.insert(cache_key, chunk_data.clone());

        debug!(
            "Loaded chunk {}/{} from disk: {} bytes",
            version,
            chunk_index,
            chunk_data.len()
        );

        Ok(chunk_data)
    }

    /// Store a received chunk
    pub async fn store_chunk(&self, version: &str, chunk_index: u32, data: Vec<u8>) -> Result<()> {
        // Create version directory if needed
        let chunks_dir = self.base_dir.join("chunks").join(version);
        fs::create_dir_all(&chunks_dir)?;

        // Calculate hash for verification
        let mut hasher = Sha256::new();
        hasher.update(&data);
        let chunk_hash = format!("{:x}", hasher.finalize());

        // Save to disk
        let chunk_path = chunks_dir.join(format!("chunk_{:04}.bin", chunk_index));
        fs::write(&chunk_path, &data)?;

        // Add to cache
        let mut cache = self.chunk_cache.write().await;
        cache.insert((version.to_string(), chunk_index), data.clone());

        debug!(
            "Stored chunk {}/{}: {} bytes, hash: {}",
            version,
            chunk_index,
            data.len(),
            &chunk_hash[..8]
        );

        Ok(())
    }

    /// Check if we have a complete binary
    pub async fn has_complete_binary(&self, version: &str, total_chunks: u32) -> bool {
        let chunks_dir = self.base_dir.join("chunks").join(version);
        if !chunks_dir.exists() {
            return false;
        }

        for i in 0..total_chunks {
            let chunk_path = chunks_dir.join(format!("chunk_{:04}.bin", i));
            if !chunk_path.exists() {
                return false;
            }
        }

        true
    }

    /// Assemble a complete binary from chunks
    pub async fn assemble_binary(&self, version: &str, total_chunks: u32) -> Result<PathBuf> {
        if !self.has_complete_binary(version, total_chunks).await {
            return Err(anyhow!("Not all chunks available for version {}", version));
        }

        info!(
            "Assembling binary for version {} from {} chunks",
            version, total_chunks
        );

        let chunks_dir = self.base_dir.join("chunks").join(version);
        let output_path = self
            .base_dir
            .join("binaries")
            .join(format!("{}.bin", version));

        let mut output = fs::File::create(&output_path)?;
        let mut total_size = 0;

        for i in 0..total_chunks {
            let chunk_path = chunks_dir.join(format!("chunk_{:04}.bin", i));
            let chunk_data = fs::read(&chunk_path)?;
            total_size += chunk_data.len();
            output.write_all(&chunk_data)?;
        }

        info!(
            "Successfully assembled binary for version {}: {} bytes",
            version, total_size
        );

        Ok(output_path)
    }

    /// Get metadata for a version
    pub async fn get_metadata(&self, version: &str) -> Option<BinaryMetadata> {
        let meta = self.version_metadata.read().await;
        meta.get(version).cloned()
    }

    /// List all available versions
    pub async fn list_versions(&self) -> Vec<String> {
        let meta = self.version_metadata.read().await;
        meta.keys().cloned().collect()
    }

    /// Clear cache to free memory
    pub async fn clear_cache(&self) {
        let mut cache = self.chunk_cache.write().await;
        let size = cache.len();
        cache.clear();
        debug!("Cleared chunk cache: {} entries freed", size);
    }
}
