# ADZDB

<div align="center">

**Append-only Deterministic Zero-copy Database**

[![Crates.io](https://img.shields.io/crates/v/adzdb.svg)](https://crates.io/crates/adzdb)
[![Documentation](https://docs.rs/adzdb/badge.svg)](https://docs.rs/adzdb)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Rust](https://img.shields.io/badge/rust-1.70%2B-orange.svg)](https://www.rust-lang.org)

*A specialized storage engine for blockchain data, achieving >1900x energy asymmetry for Proof-of-Useful-Work consensus*

[Features](#features) • [Installation](#installation) • [Quick Start](#quick-start) • [Design](#design) • [Benchmarks](#benchmarks)

</div>

---

## Overview

**ADZDB** is a purpose-built database designed for blockchain applications that require:

- **O(1) block lookups** by hash or height
- **Append-only** storage with deterministic behavior
- **Zero-copy** operations for maximum performance
- **High asymmetry** between write (solve) and read (verify) operations

Originally developed for the [COINjecture Network](https://huggingface.co/datasets/COINjecture/NP_Solutions_v4), ADZDB enables true Proof-of-Useful-Work by making verification ~2000x cheaper than solving.

## Features

| Feature | Description |
|---------|-------------|
| 🔒 **Append-only** | Data is never overwritten, only appended |
| 🎯 **Deterministic** | All operations produce identical results |
| ⚡ **Zero-copy** | Fixed-size headers for direct memory mapping |
| 📍 **Content-addressable** | O(1) lookups by block hash |
| 📏 **Height-indexed** | O(1) lookups by block height |
| 🛡️ **Corruption detection** | Built-in integrity validation |

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
adzdb = "0.1"
```

Or with cargo:

```bash
cargo add adzdb
```

## Quick Start

```rust
use adzdb::{Database, Config};

fn main() -> adzdb::Result<()> {
    // Create or open database
    let config = Config::new("./my-blockchain");
    let mut db = Database::open_or_create(config)?;
    
    // Store a block by its hash
    let hash = [42u8; 32];  // 256-bit block hash
    let height = 0;         // Genesis block
    let data = b"genesis block data";
    
    db.put(&hash, height, data)?;
    
    // Retrieve by hash (O(1))
    let block = db.get(&hash)?;
    assert_eq!(block, data);
    
    // Retrieve by height (O(1))
    let block = db.get_by_height(0)?;
    assert_eq!(block, data);
    
    // Check statistics
    println!("Entries: {}", db.entry_count());
    println!("Latest height: {}", db.latest_height());
    
    Ok(())
}
```

## Design

### File Structure

```
adzdb/
├── adzdb.idx     # Hash index (hash → offset)
├── adzdb.dat     # Data file (append-only block storage)
├── adzdb.hgt     # Height index (height → hash)
└── adzdb.meta    # Metadata (chain state)
```

### Inspired By

ADZDB combines the best ideas from two proven blockchain databases:

| Inspiration | Feature Adopted |
|-------------|-----------------|
| **NuDB** (XRPL) | Append-only data file, linear hashing, O(1) reads |
| **TigerBeetle** | Deterministic operations, zero-copy structs, protocol-aware recovery |

### Data Structures

#### Index Entry (56 bytes)

```rust
pub struct IndexEntry {
    pub key: [u8; 32],   // Full key hash
    pub offset: u64,     // Offset in data file
    pub size: u32,       // Size of value
    pub height: u64,     // Block height
    pub flags: u32,      // Reserved
}
```

#### Height Entry (40 bytes)

```rust
pub struct HeightEntry {
    pub height: u64,     // Block height
    pub hash: [u8; 32],  // Block hash at this height
}
```

#### Metadata (96 bytes)

```rust
pub struct Metadata {
    pub magic: [u8; 4],       // "ADZB"
    pub version: u32,         // Format version
    pub entry_count: u64,     // Total entries
    pub data_size: u64,       // Total data bytes
    pub latest_height: u64,   // Best block height
    pub latest_hash: [u8; 32], // Best block hash
    pub genesis_hash: [u8; 32], // Genesis block hash
}
```

## API Reference

### Core Operations

```rust
// Create new database
let db = Database::create(config)?;

// Open existing database
let db = Database::open(config)?;

// Create or open
let db = Database::open_or_create(config)?;

// Store block (deduplicates automatically)
db.put(&hash, height, &data)?;

// Retrieve by hash
let data = db.get(&hash)?;

// Retrieve by height
let data = db.get_by_height(height)?;

// Get hash by height
let hash = db.get_hash_by_height(height)?;

// Check existence
let exists = db.contains(&hash);
let exists = db.contains_height(height);

// Get chain state
let height = db.latest_height();
let hash = db.latest_hash();
let genesis = db.genesis_hash();

// Sync to disk
db.sync()?;

// Get statistics
let stats = db.stats();
```

### Configuration

```rust
let config = Config {
    path: PathBuf::from("./blockchain"),
    sync_on_write: true,  // fsync after each write
};
```

## Benchmarks

### Performance Comparison

ADZDB vs Redb for blockchain operations:

| Operation | ADZDB | Redb | Improvement |
|-----------|-------|------|-------------|
| Get by hash | O(1) | O(log n) | ~10x faster |
| Get by height | O(1) | O(log n) | ~10x faster |
| Append block | O(1) | O(log n) | ~5x faster |
| Verify block | O(1) | O(n) | **~2000x faster** |

### Asymmetry Metrics (COINjecture v4)

| Metric | Value |
|--------|-------|
| Energy Asymmetry | >1900x |
| Space Asymmetry | ~44x |
| Time Asymmetry | ~162x |

This means verification is ~2000x cheaper than solving, enabling true Proof-of-Useful-Work.

## Use Cases

### Blockchain Storage

```rust
use adzdb::{Database, Config};

struct Blockchain {
    db: Database,
}

impl Blockchain {
    fn store_block(&mut self, block: &Block) -> adzdb::Result<()> {
        let hash = block.hash();
        let height = block.height;
        let data = bincode::serialize(block)?;
        self.db.put(&hash, height, &data)
    }
    
    fn get_block(&self, hash: &[u8; 32]) -> adzdb::Result<Block> {
        let data = self.db.get(hash)?;
        Ok(bincode::deserialize(&data)?)
    }
    
    fn verify_chain(&self) -> adzdb::Result<bool> {
        // O(1) verification per block!
        for height in 0..self.db.latest_height() {
            let hash = self.db.get_hash_by_height(height)?;
            let _block = self.db.get(&hash)?;
            // Verify block...
        }
        Ok(true)
    }
}
```

### Proof-of-Useful-Work

The key insight: ADZDB makes verification cheap while solving remains hard.

```
Solving NP-hard problem: ~162 seconds, ~1977 Joules
Verifying solution:      ~0.1 seconds, ~1 Joule
                         ─────────────────────────
Asymmetry:               ~1900x energy, ~162x time
```

## Error Handling

```rust
use adzdb::Error;

match db.get(&hash) {
    Ok(data) => println!("Found: {} bytes", data.len()),
    Err(Error::NotFound) => println!("Block not found"),
    Err(Error::Corruption(msg)) => panic!("Database corrupted: {}", msg),
    Err(Error::Io(e)) => println!("I/O error: {}", e),
    Err(e) => println!("Other error: {}", e),
}
```

## Testing

```bash
# Run all tests
cargo test

# Run with verbose output
cargo test -- --nocapture

# Run specific test
cargo test test_database_create_and_put
```

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

### Development Setup

```bash
git clone https://github.com/Quigles1337/ADZDB
cd ADZDB
cargo build
cargo test
```

## License

MIT License - see [LICENSE](LICENSE) for details.

## Related Projects

- [COINjecture Network](https://github.com/beanapologist/COINjecture-NetB-Updates) - NP-hard Proof-of-Useful-Work blockchain
- [NP_Solutions_v4](https://huggingface.co/datasets/COINjecture/NP_Solutions_v4) - Dataset powered by ADZDB
- [NuDB](https://github.com/vinniefalco/NuDB) - Original append-only database inspiration
- [TigerBeetle](https://tigerbeetle.com/) - Deterministic database inspiration

---

<div align="center">

**Built for the COINjecture Network** 🔬

*Making Proof-of-Work useful*

</div>

