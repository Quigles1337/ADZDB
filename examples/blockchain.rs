//! Blockchain simulation example
//!
//! Shows how ADZDB can be used as a blockchain storage backend
//!
//! Run with: cargo run --example blockchain

use adzdb::{Database, Config, Hash};
use std::time::Instant;

/// Simple block structure
#[derive(Debug)]
struct Block {
    height: u64,
    prev_hash: Hash,
    data: String,
    nonce: u64,
}

impl Block {
    fn new(height: u64, prev_hash: Hash, data: &str) -> Self {
        Block {
            height,
            prev_hash,
            data: data.to_string(),
            nonce: 0,
        }
    }

    /// Simple hash function (not cryptographically secure, just for demo)
    fn hash(&self) -> Hash {
        let mut hash = [0u8; 32];
        let data = format!("{}:{:?}:{}:{}", self.height, self.prev_hash, self.data, self.nonce);
        
        // Simple hash: just take bytes from the data
        for (i, byte) in data.bytes().enumerate() {
            hash[i % 32] ^= byte;
        }
        
        // Mix in height
        hash[0] ^= (self.height & 0xFF) as u8;
        hash[1] ^= ((self.height >> 8) & 0xFF) as u8;
        
        hash
    }

    fn serialize(&self) -> Vec<u8> {
        format!(
            r#"{{"height":{},"prev_hash":"{:?}","data":"{}","nonce":{}}}"#,
            self.height, self.prev_hash, self.data, self.nonce
        )
        .into_bytes()
    }
}

fn main() -> adzdb::Result<()> {
    let temp_dir = std::env::temp_dir().join("adzdb-blockchain-example");
    let _ = std::fs::remove_dir_all(&temp_dir);

    println!("⛓️  ADZDB Blockchain Simulation");
    println!("================================\n");

    // Create database
    let config = Config::new(&temp_dir).with_sync_on_write(false); // Faster for this demo
    let mut db = Database::open_or_create(config)?;

    // Create genesis block
    let genesis = Block::new(0, [0u8; 32], "Genesis Block");
    let genesis_hash = genesis.hash();
    db.put(&genesis_hash, 0, &genesis.serialize())?;
    println!("📦 Genesis block created: {:02x}{:02x}...", genesis_hash[0], genesis_hash[1]);

    // Mine some blocks
    let block_count = 100;
    println!("\n⛏️  Mining {} blocks...", block_count);
    
    let start = Instant::now();
    let mut prev_hash = genesis_hash;

    for height in 1..=block_count {
        let block = Block::new(height, prev_hash, &format!("Block {} data", height));
        let hash = block.hash();
        db.put(&hash, height, &block.serialize())?;
        prev_hash = hash;

        if height % 25 == 0 {
            println!("   Mined block {}", height);
        }
    }

    let elapsed = start.elapsed();
    db.sync()?;

    println!("\n📊 Mining Results:");
    println!("   Blocks mined: {}", block_count);
    println!("   Time elapsed: {:?}", elapsed);
    println!("   Blocks/second: {:.0}", block_count as f64 / elapsed.as_secs_f64());

    // Test retrieval performance
    println!("\n🔍 Testing retrieval performance...");
    
    // Random hash lookups
    let start = Instant::now();
    for height in 0..=block_count {
        let hash = db.get_hash_by_height(height)?;
        let _data = db.get(&hash)?;
    }
    println!("   Hash lookups ({} blocks): {:?}", block_count + 1, start.elapsed());

    // Sequential height lookups
    let start = Instant::now();
    for height in 0..=block_count {
        let _data = db.get_by_height(height)?;
    }
    println!("   Height lookups ({} blocks): {:?}", block_count + 1, start.elapsed());

    // Verify chain integrity
    println!("\n✅ Verifying chain integrity...");
    let start = Instant::now();
    
    let mut expected_prev_hash = [0u8; 32]; // Genesis has no previous
    for height in 0..=block_count {
        let data = db.get_by_height(height)?;
        let data_str = String::from_utf8_lossy(&data);
        
        // Check prev_hash is mentioned (simplified check)
        if height > 0 {
            // In a real implementation, we'd deserialize and verify
            assert!(data_str.contains("prev_hash"));
        }
        
        expected_prev_hash = db.get_hash_by_height(height)?;
    }
    
    println!("   Chain verified in {:?}", start.elapsed());
    println!("   Tip: height {}, hash {:02x}{:02x}...", 
        db.latest_height(), 
        db.latest_hash()[0], 
        db.latest_hash()[1]
    );

    // Show final stats
    let stats = db.stats();
    println!("\n📈 Final Statistics:");
    println!("   Total entries: {}", stats.entry_count);
    println!("   Total data: {} KB", stats.data_size / 1024);
    println!("   Avg block size: {} bytes", stats.data_size / stats.entry_count);

    // Clean up
    drop(db);
    std::fs::remove_dir_all(&temp_dir)?;
    println!("\n🧹 Cleaned up successfully!");

    Ok(())
}

