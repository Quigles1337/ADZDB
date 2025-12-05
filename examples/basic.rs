//! Basic usage example for ADZDB
//!
//! Run with: cargo run --example basic

use adzdb::{Database, Config};

fn main() -> adzdb::Result<()> {
    // Create a temporary database
    let temp_dir = std::env::temp_dir().join("adzdb-example");
    let _ = std::fs::remove_dir_all(&temp_dir);

    println!("🗄️  Creating ADZDB at {:?}", temp_dir);

    // Create database with default config (sync on every write)
    let config = Config::new(&temp_dir);
    let mut db = Database::open_or_create(config)?;

    // Simulate storing blockchain blocks
    println!("\n📦 Storing blocks...");

    // Genesis block (height 0)
    let genesis_hash = [0u8; 32];
    let genesis_data = br#"{"height":0,"data":"Genesis block","timestamp":1700000000}"#;
    db.put(&genesis_hash, 0, genesis_data)?;
    println!("   Block 0: Genesis stored");

    // Block 1
    let block1_hash = [1u8; 32];
    let block1_data = br#"{"height":1,"data":"First block","prev_hash":"00...00"}"#;
    db.put(&block1_hash, 1, block1_data)?;
    println!("   Block 1: Stored");

    // Block 2
    let block2_hash = [2u8; 32];
    let block2_data = br#"{"height":2,"data":"Second block","prev_hash":"01...01"}"#;
    db.put(&block2_hash, 2, block2_data)?;
    println!("   Block 2: Stored");

    // Retrieve by hash (O(1))
    println!("\n🔍 Retrieving by hash...");
    let retrieved = db.get(&genesis_hash)?;
    println!("   Genesis: {}", String::from_utf8_lossy(&retrieved));

    // Retrieve by height (O(1))
    println!("\n📊 Retrieving by height...");
    for height in 0..=db.latest_height() {
        let data = db.get_by_height(height)?;
        let hash = db.get_hash_by_height(height)?;
        println!(
            "   Height {}: {} bytes, hash: {:02x}{:02x}...",
            height,
            data.len(),
            hash[0],
            hash[1]
        );
    }

    // Check existence
    println!("\n✅ Checking existence...");
    println!("   Hash [0u8; 32] exists: {}", db.contains(&genesis_hash));
    println!("   Hash [99u8; 32] exists: {}", db.contains(&[99u8; 32]));
    println!("   Height 0 exists: {}", db.contains_height(0));
    println!("   Height 999 exists: {}", db.contains_height(999));

    // Show statistics
    let stats = db.stats();
    println!("\n📈 Database Statistics:");
    println!("   Entry count: {}", stats.entry_count);
    println!("   Data size: {} bytes", stats.data_size);
    println!("   Latest height: {}", stats.latest_height);
    println!(
        "   Genesis hash: {:02x}{:02x}...",
        stats.genesis_hash[0], stats.genesis_hash[1]
    );

    // Demonstrate deduplication
    println!("\n🔄 Testing deduplication...");
    let count_before = db.entry_count();
    db.put(&genesis_hash, 0, b"different data")?;
    let count_after = db.entry_count();
    println!(
        "   Entries before: {}, after: {} (should be same)",
        count_before, count_after
    );

    // Clean up
    println!("\n🧹 Cleaning up...");
    drop(db);
    std::fs::remove_dir_all(&temp_dir)?;
    println!("   Done!");

    Ok(())
}

