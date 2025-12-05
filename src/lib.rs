//! # ADZDB: Append-only Deterministic Zero-copy Database
//!
//! A specialized storage engine for blockchain data, inspired by:
//! - **NuDB** (XRPL): Append-only data file, linear hashing, O(1) reads
//! - **TigerBeetle**: Deterministic operations, zero-copy structs, protocol-aware recovery
//!
//! ## Design Principles
//!
//! 1. **Append-only**: Data is never overwritten, only appended
//! 2. **Deterministic**: All operations produce identical results
//! 3. **Zero-copy**: Fixed-size headers for direct memory mapping
//!
//! ## File Structure
//!
//! ```text
//! adzdb/
//! ├── adzdb.idx     # Hash index (hash → offset)
//! ├── adzdb.dat     # Data file (append-only block storage)
//! ├── adzdb.hgt     # Height index (height → hash)
//! └── adzdb.meta    # Metadata (chain state)
//! ```
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use adzdb::{Database, Config};
//!
//! # fn main() -> adzdb::Result<()> {
//! // Create or open database
//! let config = Config::new("./my-blockchain");
//! let mut db = Database::open_or_create(config)?;
//!
//! // Store a block
//! let hash = [42u8; 32];
//! db.put(&hash, 0, b"genesis")?;
//!
//! // Retrieve by hash (O(1))
//! let data = db.get(&hash)?;
//!
//! // Retrieve by height (O(1))
//! let data = db.get_by_height(0)?;
//! # Ok(())
//! # }
//! ```
//!
//! ## Performance
//!
//! ADZDB achieves O(1) lookups for both hash and height queries by maintaining
//! in-memory indices that are persisted to disk. This makes it ideal for
//! blockchain verification where fast block lookups are critical.
//!
//! | Operation | Complexity |
//! |-----------|------------|
//! | Get by hash | O(1) |
//! | Get by height | O(1) |
//! | Put block | O(1) amortized |
//! | Contains | O(1) |

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write, Seek, SeekFrom, BufReader};
use std::path::{Path, PathBuf};
use std::collections::HashMap;

/// Magic bytes for ADZDB files
pub const MAGIC: &[u8; 4] = b"ADZB";

/// Current file format version
pub const VERSION: u32 = 1;

/// Maximum value size (1 GB)
pub const MAX_VALUE_SIZE: u64 = 1 << 30;

/// Maximum reasonable block height (corruption detection)
pub const MAX_REASONABLE_HEIGHT: u64 = 10_000_000;

/// 256-bit hash type
pub type Hash = [u8; 32];

/// Zero hash constant
pub const ZERO_HASH: Hash = [0u8; 32];

/// Configuration for ADZDB
///
/// # Example
///
/// ```rust
/// use adzdb::Config;
///
/// let config = Config::new("./blockchain");
/// assert!(config.sync_on_write); // Default is true
/// ```
#[derive(Debug, Clone)]
pub struct Config {
    /// Base path for database files
    pub path: PathBuf,
    /// Sync data to disk after each write (default: true)
    pub sync_on_write: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            path: PathBuf::from("./adzdb"),
            sync_on_write: true,
        }
    }
}

impl Config {
    /// Create a new configuration with the specified path
    ///
    /// # Example
    ///
    /// ```rust
    /// use adzdb::Config;
    ///
    /// let config = Config::new("/var/lib/blockchain");
    /// ```
    pub fn new<P: AsRef<Path>>(path: P) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
            ..Default::default()
        }
    }

    /// Set whether to sync to disk after each write
    ///
    /// Disabling sync improves performance but risks data loss on crash.
    pub fn with_sync_on_write(mut self, sync: bool) -> Self {
        self.sync_on_write = sync;
        self
    }
}

/// Error types for ADZDB operations
#[derive(Debug)]
pub enum Error {
    /// I/O error
    Io(io::Error),
    /// Key not found
    NotFound,
    /// Corrupt data detected
    Corruption(String),
    /// Value too large
    ValueTooLarge(u64),
    /// Database already exists
    AlreadyExists,
    /// Invalid configuration
    InvalidConfig(String),
    /// Hash mismatch (content-addressable violation)
    HashMismatch { expected: Hash, actual: Hash },
    /// Height too large (corruption detection)
    HeightTooLarge(u64),
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(e) => write!(f, "I/O error: {}", e),
            Error::NotFound => write!(f, "Key not found"),
            Error::Corruption(msg) => write!(f, "Data corruption: {}", msg),
            Error::ValueTooLarge(size) => write!(f, "Value too large: {} bytes", size),
            Error::AlreadyExists => write!(f, "Database already exists"),
            Error::InvalidConfig(msg) => write!(f, "Invalid config: {}", msg),
            Error::HashMismatch { expected, actual } => {
                write!(f, "Hash mismatch: expected {:?}, got {:?}", expected, actual)
            }
            Error::HeightTooLarge(h) => write!(f, "Height {} exceeds maximum {}", h, MAX_REASONABLE_HEIGHT),
        }
    }
}

impl std::error::Error for Error {}

/// Result type for ADZDB operations
pub type Result<T> = std::result::Result<T, Error>;

/// Index entry - maps hash to data file offset (56 bytes)
///
/// This is a fixed-size structure that can be directly memory-mapped
/// for zero-copy access.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct IndexEntry {
    /// Full key hash (32 bytes)
    pub key: Hash,
    /// Offset in data file (8 bytes)
    pub offset: u64,
    /// Size of value in data file (4 bytes)
    pub size: u32,
    /// Block height for quick filtering (8 bytes)
    pub height: u64,
    /// Flags reserved for future use (4 bytes)
    pub flags: u32,
}

impl IndexEntry {
    /// Size of index entry in bytes
    pub const SIZE: usize = 56;

    /// Serialize to bytes
    pub fn to_bytes(&self) -> [u8; Self::SIZE] {
        let mut buf = [0u8; Self::SIZE];
        buf[0..32].copy_from_slice(&self.key);
        buf[32..40].copy_from_slice(&self.offset.to_le_bytes());
        buf[40..44].copy_from_slice(&self.size.to_le_bytes());
        buf[44..52].copy_from_slice(&self.height.to_le_bytes());
        buf[52..56].copy_from_slice(&self.flags.to_le_bytes());
        buf
    }

    /// Deserialize from bytes
    pub fn from_bytes(bytes: &[u8; Self::SIZE]) -> Self {
        Self {
            key: bytes[0..32].try_into().unwrap(),
            offset: u64::from_le_bytes(bytes[32..40].try_into().unwrap()),
            size: u32::from_le_bytes(bytes[40..44].try_into().unwrap()),
            height: u64::from_le_bytes(bytes[44..52].try_into().unwrap()),
            flags: u32::from_le_bytes(bytes[52..56].try_into().unwrap()),
        }
    }
}

/// Height index entry - maps height to hash (40 bytes)
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct HeightEntry {
    /// Block height (8 bytes)
    pub height: u64,
    /// Block hash at this height (32 bytes)
    pub hash: Hash,
}

impl HeightEntry {
    /// Size of height entry in bytes
    pub const SIZE: usize = 40;

    /// Serialize to bytes
    pub fn to_bytes(&self) -> [u8; Self::SIZE] {
        let mut buf = [0u8; Self::SIZE];
        buf[0..8].copy_from_slice(&self.height.to_le_bytes());
        buf[8..40].copy_from_slice(&self.hash);
        buf
    }

    /// Deserialize from bytes
    pub fn from_bytes(bytes: &[u8; Self::SIZE]) -> Self {
        Self {
            height: u64::from_le_bytes(bytes[0..8].try_into().unwrap()),
            hash: bytes[8..40].try_into().unwrap(),
        }
    }
}

/// Database metadata (stored in adzdb.meta)
#[derive(Debug, Clone)]
pub struct Metadata {
    /// Magic bytes ("ADZB")
    pub magic: [u8; 4],
    /// Version number
    pub version: u32,
    /// Number of entries
    pub entry_count: u64,
    /// Total data size in bytes
    pub data_size: u64,
    /// Latest block height
    pub latest_height: u64,
    /// Latest block hash
    pub latest_hash: Hash,
    /// Genesis hash
    pub genesis_hash: Hash,
}

impl Default for Metadata {
    fn default() -> Self {
        Self {
            magic: *MAGIC,
            version: VERSION,
            entry_count: 0,
            data_size: 0,
            latest_height: 0,
            latest_hash: ZERO_HASH,
            genesis_hash: ZERO_HASH,
        }
    }
}

impl Metadata {
    /// Size of metadata in bytes
    pub const SIZE: usize = 96;

    /// Serialize to bytes
    pub fn to_bytes(&self) -> [u8; Self::SIZE] {
        let mut buf = [0u8; Self::SIZE];
        buf[0..4].copy_from_slice(&self.magic);
        buf[4..8].copy_from_slice(&self.version.to_le_bytes());
        buf[8..16].copy_from_slice(&self.entry_count.to_le_bytes());
        buf[16..24].copy_from_slice(&self.data_size.to_le_bytes());
        buf[24..32].copy_from_slice(&self.latest_height.to_le_bytes());
        buf[32..64].copy_from_slice(&self.latest_hash);
        buf[64..96].copy_from_slice(&self.genesis_hash);
        buf
    }

    /// Deserialize from bytes with validation
    pub fn from_bytes(bytes: &[u8; Self::SIZE]) -> Result<Self> {
        let magic: [u8; 4] = bytes[0..4].try_into().unwrap();
        if &magic != MAGIC {
            return Err(Error::Corruption("Invalid magic bytes".to_string()));
        }

        let meta = Self {
            magic,
            version: u32::from_le_bytes(bytes[4..8].try_into().unwrap()),
            entry_count: u64::from_le_bytes(bytes[8..16].try_into().unwrap()),
            data_size: u64::from_le_bytes(bytes[16..24].try_into().unwrap()),
            latest_height: u64::from_le_bytes(bytes[24..32].try_into().unwrap()),
            latest_hash: bytes[32..64].try_into().unwrap(),
            genesis_hash: bytes[64..96].try_into().unwrap(),
        };

        // Corruption detection
        if meta.latest_height > MAX_REASONABLE_HEIGHT {
            return Err(Error::HeightTooLarge(meta.latest_height));
        }

        Ok(meta)
    }
}

/// The main ADZDB database handle
///
/// # Example
///
/// ```rust,no_run
/// use adzdb::{Database, Config};
///
/// # fn main() -> adzdb::Result<()> {
/// let config = Config::new("./blockchain");
/// let mut db = Database::open_or_create(config)?;
///
/// // Store genesis block
/// let hash = [0u8; 32];
/// db.put(&hash, 0, b"genesis block")?;
///
/// // Retrieve it
/// let data = db.get(&hash)?;
/// assert_eq!(data, b"genesis block");
/// # Ok(())
/// # }
/// ```
pub struct Database {
    config: Config,
    /// Hash index file
    index_file: File,
    /// Data file (append-only)
    data_file: File,
    /// Height index file
    height_file: File,
    /// Metadata file
    meta_file: File,
    /// In-memory hash index (loaded on open)
    hash_index: HashMap<Hash, IndexEntry>,
    /// In-memory height index
    height_index: HashMap<u64, Hash>,
    /// Current metadata
    metadata: Metadata,
}

impl Database {
    /// Create a new database at the specified path
    ///
    /// # Errors
    ///
    /// Returns `Error::AlreadyExists` if the database already exists.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use adzdb::{Database, Config};
    ///
    /// # fn main() -> adzdb::Result<()> {
    /// let config = Config::new("./new-blockchain");
    /// let db = Database::create(config)?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn create(config: Config) -> Result<Self> {
        std::fs::create_dir_all(&config.path)?;

        let index_path = config.path.join("adzdb.idx");
        let data_path = config.path.join("adzdb.dat");
        let height_path = config.path.join("adzdb.hgt");
        let meta_path = config.path.join("adzdb.meta");

        // Check if already exists
        if index_path.exists() || data_path.exists() {
            return Err(Error::AlreadyExists);
        }

        // Create files
        let index_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&index_path)?;

        let data_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .append(true)
            .open(&data_path)?;

        let height_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&height_path)?;

        let mut meta_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&meta_path)?;

        // Write initial metadata
        let metadata = Metadata::default();
        meta_file.write_all(&metadata.to_bytes())?;
        meta_file.sync_all()?;

        #[cfg(feature = "tracing")]
        tracing::info!("🗄️  ADZDB created at {:?}", config.path);

        Ok(Self {
            config,
            index_file,
            data_file,
            height_file,
            meta_file,
            hash_index: HashMap::new(),
            height_index: HashMap::new(),
            metadata,
        })
    }

    /// Open an existing database
    ///
    /// # Errors
    ///
    /// Returns an I/O error if the database doesn't exist or is corrupted.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use adzdb::{Database, Config};
    ///
    /// # fn main() -> adzdb::Result<()> {
    /// let config = Config::new("./existing-blockchain");
    /// let db = Database::open(config)?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn open(config: Config) -> Result<Self> {
        let index_path = config.path.join("adzdb.idx");
        let data_path = config.path.join("adzdb.dat");
        let height_path = config.path.join("adzdb.hgt");
        let meta_path = config.path.join("adzdb.meta");

        // Open files
        let index_file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&index_path)?;

        let data_file = OpenOptions::new()
            .read(true)
            .write(true)
            .append(true)
            .open(&data_path)?;

        let height_file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&height_path)?;

        let meta_file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&meta_path)?;

        // Load metadata
        let metadata = Self::load_metadata(&meta_file)?;

        // Load hash index into memory
        let hash_index = Self::load_hash_index(&index_file)?;

        // Load height index into memory
        let height_index = Self::load_height_index(&height_file)?;

        #[cfg(feature = "tracing")]
        tracing::info!(
            "🗄️  ADZDB opened: {} entries, height {}",
            metadata.entry_count,
            metadata.latest_height
        );

        Ok(Self {
            config,
            index_file,
            data_file,
            height_file,
            meta_file,
            hash_index,
            height_index,
            metadata,
        })
    }

    /// Open existing database or create new one
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use adzdb::{Database, Config};
    ///
    /// # fn main() -> adzdb::Result<()> {
    /// let config = Config::new("./blockchain");
    /// let mut db = Database::open_or_create(config)?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn open_or_create(config: Config) -> Result<Self> {
        let meta_path = config.path.join("adzdb.meta");
        if meta_path.exists() {
            Self::open(config)
        } else {
            Self::create(config)
        }
    }

    fn load_metadata(file: &File) -> Result<Metadata> {
        let mut reader = BufReader::new(file);
        let mut buf = [0u8; Metadata::SIZE];

        reader.read_exact(&mut buf).map_err(|e| {
            if e.kind() == io::ErrorKind::UnexpectedEof {
                Error::Corruption("Metadata file too small".to_string())
            } else {
                Error::Io(e)
            }
        })?;

        Metadata::from_bytes(&buf)
    }

    fn load_hash_index(file: &File) -> Result<HashMap<Hash, IndexEntry>> {
        let mut index = HashMap::new();
        let mut reader = BufReader::new(file);
        let mut buf = [0u8; IndexEntry::SIZE];

        loop {
            match reader.read_exact(&mut buf) {
                Ok(()) => {
                    let entry = IndexEntry::from_bytes(&buf);
                    if entry.key != ZERO_HASH {
                        index.insert(entry.key, entry);
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
                Err(e) => return Err(Error::Io(e)),
            }
        }

        Ok(index)
    }

    fn load_height_index(file: &File) -> Result<HashMap<u64, Hash>> {
        let mut index = HashMap::new();
        let mut reader = BufReader::new(file);
        let mut buf = [0u8; HeightEntry::SIZE];

        loop {
            match reader.read_exact(&mut buf) {
                Ok(()) => {
                    let entry = HeightEntry::from_bytes(&buf);
                    if entry.hash != ZERO_HASH {
                        index.insert(entry.height, entry.hash);
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
                Err(e) => return Err(Error::Io(e)),
            }
        }

        Ok(index)
    }

    /// Store a value by hash (content-addressable)
    ///
    /// Automatically deduplicates: if the hash already exists, this is a no-op.
    ///
    /// # Arguments
    ///
    /// * `hash` - The 256-bit hash key (typically the block hash)
    /// * `height` - The block height for indexing
    /// * `data` - The data to store
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use adzdb::{Database, Config};
    ///
    /// # fn main() -> adzdb::Result<()> {
    /// let config = Config::new("./blockchain");
    /// let mut db = Database::open_or_create(config)?;
    ///
    /// let hash = [42u8; 32];
    /// db.put(&hash, 0, b"block data")?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn put(&mut self, hash: &Hash, height: u64, data: &[u8]) -> Result<()> {
        // Corruption detection
        if height > MAX_REASONABLE_HEIGHT {
            return Err(Error::HeightTooLarge(height));
        }

        // Check if already exists (deduplication)
        if self.hash_index.contains_key(hash) {
            return Ok(());
        }

        // Get current data file position
        let offset = self.data_file.seek(SeekFrom::End(0))?;

        // Write data
        self.data_file.write_all(data)?;

        // Create index entry
        let entry = IndexEntry {
            key: *hash,
            offset,
            size: data.len() as u32,
            height,
            flags: 0,
        };

        // Write to index file
        self.index_file.seek(SeekFrom::End(0))?;
        self.index_file.write_all(&entry.to_bytes())?;

        // Write to height index file
        let height_entry = HeightEntry {
            height,
            hash: *hash,
        };
        self.height_file.seek(SeekFrom::End(0))?;
        self.height_file.write_all(&height_entry.to_bytes())?;

        // Update in-memory indices
        self.hash_index.insert(*hash, entry);
        self.height_index.insert(height, *hash);

        // Update metadata
        self.metadata.entry_count += 1;
        self.metadata.data_size += data.len() as u64;

        if height > self.metadata.latest_height {
            self.metadata.latest_height = height;
            self.metadata.latest_hash = *hash;
        }

        if height == 0 {
            self.metadata.genesis_hash = *hash;
        }

        // Sync if configured
        if self.config.sync_on_write {
            self.sync()?;
        }

        Ok(())
    }

    /// Get value by hash (O(1) lookup)
    ///
    /// # Errors
    ///
    /// Returns `Error::NotFound` if the hash doesn't exist.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use adzdb::{Database, Config};
    ///
    /// # fn main() -> adzdb::Result<()> {
    /// let config = Config::new("./blockchain");
    /// let db = Database::open(config)?;
    ///
    /// let hash = [42u8; 32];
    /// let data = db.get(&hash)?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn get(&self, hash: &Hash) -> Result<Vec<u8>> {
        let entry = self.hash_index.get(hash).ok_or(Error::NotFound)?;

        let file = &self.data_file;
        let mut reader = BufReader::new(file);
        reader.seek(SeekFrom::Start(entry.offset))?;

        let mut data = vec![0u8; entry.size as usize];
        reader.read_exact(&mut data)?;

        Ok(data)
    }

    /// Get value by height (O(1) with height index)
    ///
    /// # Errors
    ///
    /// Returns `Error::NotFound` if no block exists at the given height.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use adzdb::{Database, Config};
    ///
    /// # fn main() -> adzdb::Result<()> {
    /// let config = Config::new("./blockchain");
    /// let db = Database::open(config)?;
    ///
    /// let genesis = db.get_by_height(0)?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn get_by_height(&self, height: u64) -> Result<Vec<u8>> {
        let hash = self.height_index.get(&height).ok_or(Error::NotFound)?;
        self.get(hash)
    }

    /// Get hash by height
    ///
    /// # Errors
    ///
    /// Returns `Error::NotFound` if no block exists at the given height.
    pub fn get_hash_by_height(&self, height: u64) -> Result<Hash> {
        self.height_index.get(&height).copied().ok_or(Error::NotFound)
    }

    /// Check if hash exists
    pub fn contains(&self, hash: &Hash) -> bool {
        self.hash_index.contains_key(hash)
    }

    /// Check if height exists
    pub fn contains_height(&self, height: u64) -> bool {
        self.height_index.contains_key(&height)
    }

    /// Get latest block height
    pub fn latest_height(&self) -> u64 {
        self.metadata.latest_height
    }

    /// Get latest block hash
    pub fn latest_hash(&self) -> Hash {
        self.metadata.latest_hash
    }

    /// Get genesis block hash
    pub fn genesis_hash(&self) -> Hash {
        self.metadata.genesis_hash
    }

    /// Get total entry count
    pub fn entry_count(&self) -> u64 {
        self.metadata.entry_count
    }

    /// Sync all files to disk
    pub fn sync(&mut self) -> Result<()> {
        // Update metadata file
        self.meta_file.seek(SeekFrom::Start(0))?;
        self.meta_file.write_all(&self.metadata.to_bytes())?;

        // Sync all files
        self.data_file.sync_all()?;
        self.index_file.sync_all()?;
        self.height_file.sync_all()?;
        self.meta_file.sync_all()?;

        Ok(())
    }

    /// Get database statistics
    pub fn stats(&self) -> DatabaseStats {
        DatabaseStats {
            entry_count: self.metadata.entry_count,
            data_size: self.metadata.data_size,
            latest_height: self.metadata.latest_height,
            latest_hash: self.metadata.latest_hash,
            genesis_hash: self.metadata.genesis_hash,
        }
    }

    /// Get the database path
    pub fn path(&self) -> &Path {
        &self.config.path
    }

    /// Iterate over all entries by height (ascending)
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use adzdb::{Database, Config};
    ///
    /// # fn main() -> adzdb::Result<()> {
    /// let config = Config::new("./blockchain");
    /// let db = Database::open(config)?;
    ///
    /// for height in 0..=db.latest_height() {
    ///     if let Ok(data) = db.get_by_height(height) {
    ///         println!("Block {}: {} bytes", height, data.len());
    ///     }
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub fn iter_heights(&self) -> impl Iterator<Item = u64> + '_ {
        let mut heights: Vec<_> = self.height_index.keys().copied().collect();
        heights.sort();
        heights.into_iter()
    }
}

/// Database statistics
#[derive(Debug, Clone)]
pub struct DatabaseStats {
    /// Total number of entries
    pub entry_count: u64,
    /// Total data size in bytes
    pub data_size: u64,
    /// Latest block height
    pub latest_height: u64,
    /// Latest block hash
    pub latest_hash: Hash,
    /// Genesis block hash
    pub genesis_hash: Hash,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_index_entry_roundtrip() {
        let entry = IndexEntry {
            key: [1u8; 32],
            offset: 12345,
            size: 1000,
            height: 42,
            flags: 0,
        };

        let bytes = entry.to_bytes();
        let recovered = IndexEntry::from_bytes(&bytes);

        assert_eq!(entry.key, recovered.key);
        assert_eq!(entry.offset, recovered.offset);
        assert_eq!(entry.size, recovered.size);
        assert_eq!(entry.height, recovered.height);
    }

    #[test]
    fn test_metadata_roundtrip() {
        let meta = Metadata {
            magic: *MAGIC,
            version: VERSION,
            entry_count: 100,
            data_size: 50000,
            latest_height: 42,
            latest_hash: [1u8; 32],
            genesis_hash: [2u8; 32],
        };

        let bytes = meta.to_bytes();
        let recovered = Metadata::from_bytes(&bytes).unwrap();

        assert_eq!(meta.entry_count, recovered.entry_count);
        assert_eq!(meta.latest_height, recovered.latest_height);
    }

    #[test]
    fn test_database_create_and_put() {
        let temp_dir = std::env::temp_dir().join("adzdb-test-create");
        let _ = fs::remove_dir_all(&temp_dir);

        let config = Config::new(&temp_dir);
        let mut db = Database::create(config).unwrap();

        let hash = [42u8; 32];
        let data = b"test block data";

        db.put(&hash, 0, data).unwrap();

        assert!(db.contains(&hash));
        assert_eq!(db.entry_count(), 1);

        let retrieved = db.get(&hash).unwrap();
        assert_eq!(retrieved, data);

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_database_height_index() {
        let temp_dir = std::env::temp_dir().join("adzdb-test-height");
        let _ = fs::remove_dir_all(&temp_dir);

        let config = Config::new(&temp_dir);
        let mut db = Database::create(config).unwrap();

        // Add blocks at different heights
        let hash0 = [0u8; 32];
        let hash1 = [1u8; 32];
        let hash2 = [2u8; 32];

        db.put(&hash0, 0, b"genesis").unwrap();
        db.put(&hash1, 1, b"block 1").unwrap();
        db.put(&hash2, 2, b"block 2").unwrap();

        // Retrieve by height
        assert_eq!(db.get_by_height(0).unwrap(), b"genesis");
        assert_eq!(db.get_by_height(1).unwrap(), b"block 1");
        assert_eq!(db.get_by_height(2).unwrap(), b"block 2");

        // Get hash by height
        assert_eq!(db.get_hash_by_height(0).unwrap(), hash0);
        assert_eq!(db.get_hash_by_height(1).unwrap(), hash1);
        assert_eq!(db.get_hash_by_height(2).unwrap(), hash2);

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_corruption_detection() {
        let temp_dir = std::env::temp_dir().join("adzdb-test-corrupt");
        let _ = fs::remove_dir_all(&temp_dir);

        let config = Config::new(&temp_dir);
        let mut db = Database::create(config).unwrap();

        let hash = [42u8; 32];

        // Try to insert with impossibly high height
        let result = db.put(&hash, MAX_REASONABLE_HEIGHT + 1, b"corrupt");
        assert!(matches!(result, Err(Error::HeightTooLarge(_))));

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_database_reopen() {
        let temp_dir = std::env::temp_dir().join("adzdb-test-reopen");
        let _ = fs::remove_dir_all(&temp_dir);

        let config = Config::new(&temp_dir);

        // Create and populate
        {
            let mut db = Database::create(config.clone()).unwrap();
            db.put(&[1u8; 32], 0, b"genesis").unwrap();
            db.put(&[2u8; 32], 1, b"block 1").unwrap();
            db.sync().unwrap();
        }

        // Reopen and verify
        {
            let db = Database::open(config).unwrap();
            assert_eq!(db.entry_count(), 2);
            assert_eq!(db.latest_height(), 1);
            assert_eq!(db.get_by_height(0).unwrap(), b"genesis");
            assert_eq!(db.get_by_height(1).unwrap(), b"block 1");
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_deduplication() {
        let temp_dir = std::env::temp_dir().join("adzdb-test-dedup");
        let _ = fs::remove_dir_all(&temp_dir);

        let config = Config::new(&temp_dir);
        let mut db = Database::create(config).unwrap();

        let hash = [42u8; 32];

        // Insert same hash twice
        db.put(&hash, 0, b"first").unwrap();
        db.put(&hash, 0, b"second").unwrap(); // Should be no-op

        // Should still have only one entry with original data
        assert_eq!(db.entry_count(), 1);
        assert_eq!(db.get(&hash).unwrap(), b"first");

        let _ = fs::remove_dir_all(&temp_dir);
    }
}

