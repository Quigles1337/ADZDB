//! Benchmarks for ADZDB operations

use adzdb::{Config, Database};
use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};
use std::fs;

fn create_test_db(name: &str) -> Database {
    let temp_dir = std::env::temp_dir().join(format!("adzdb-bench-{}", name));
    let _ = fs::remove_dir_all(&temp_dir);
    
    let config = Config::new(&temp_dir).with_sync_on_write(false);
    Database::create(config).unwrap()
}

fn cleanup_test_db(name: &str) {
    let temp_dir = std::env::temp_dir().join(format!("adzdb-bench-{}", name));
    let _ = fs::remove_dir_all(&temp_dir);
}

fn bench_put(c: &mut Criterion) {
    let mut group = c.benchmark_group("put");
    
    for size in [100, 1000, 10000].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let mut db = create_test_db(&format!("put-{}", size));
            
            // Pre-populate
            for i in 0..size {
                let mut hash = [0u8; 32];
                hash[0..8].copy_from_slice(&(i as u64).to_le_bytes());
                let data = format!("block data {}", i);
                db.put(&hash, i as u64, data.as_bytes()).unwrap();
            }
            
            let mut counter = size;
            b.iter(|| {
                let mut hash = [0u8; 32];
                hash[0..8].copy_from_slice(&(counter as u64).to_le_bytes());
                let data = format!("new block {}", counter);
                db.put(black_box(&hash), black_box(counter as u64), black_box(data.as_bytes())).unwrap();
                counter += 1;
            });
            
            cleanup_test_db(&format!("put-{}", size));
        });
    }
    
    group.finish();
}

fn bench_get_by_hash(c: &mut Criterion) {
    let mut group = c.benchmark_group("get_by_hash");
    
    for size in [100, 1000, 10000].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let mut db = create_test_db(&format!("get-hash-{}", size));
            let mut hashes = Vec::new();
            
            // Pre-populate
            for i in 0..size {
                let mut hash = [0u8; 32];
                hash[0..8].copy_from_slice(&(i as u64).to_le_bytes());
                let data = format!("block data {}", i);
                db.put(&hash, i as u64, data.as_bytes()).unwrap();
                hashes.push(hash);
            }
            
            let mut counter = 0;
            b.iter(|| {
                let hash = &hashes[counter % size];
                let _data = db.get(black_box(hash)).unwrap();
                counter += 1;
            });
            
            cleanup_test_db(&format!("get-hash-{}", size));
        });
    }
    
    group.finish();
}

fn bench_get_by_height(c: &mut Criterion) {
    let mut group = c.benchmark_group("get_by_height");
    
    for size in [100, 1000, 10000].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let mut db = create_test_db(&format!("get-height-{}", size));
            
            // Pre-populate
            for i in 0..size {
                let mut hash = [0u8; 32];
                hash[0..8].copy_from_slice(&(i as u64).to_le_bytes());
                let data = format!("block data {}", i);
                db.put(&hash, i as u64, data.as_bytes()).unwrap();
            }
            
            let mut counter: u64 = 0;
            b.iter(|| {
                let height = counter % (size as u64);
                let _data = db.get_by_height(black_box(height)).unwrap();
                counter += 1;
            });
            
            cleanup_test_db(&format!("get-height-{}", size));
        });
    }
    
    group.finish();
}

fn bench_contains(c: &mut Criterion) {
    let mut group = c.benchmark_group("contains");
    
    let size = 10000;
    let mut db = create_test_db("contains");
    let mut hashes = Vec::new();
    
    // Pre-populate
    for i in 0..size {
        let mut hash = [0u8; 32];
        hash[0..8].copy_from_slice(&(i as u64).to_le_bytes());
        let data = format!("block data {}", i);
        db.put(&hash, i as u64, data.as_bytes()).unwrap();
        hashes.push(hash);
    }
    
    group.bench_function("existing", |b| {
        let mut counter = 0;
        b.iter(|| {
            let hash = &hashes[counter % size];
            let _exists = db.contains(black_box(hash));
            counter += 1;
        });
    });
    
    group.bench_function("non_existing", |b| {
        b.iter(|| {
            let hash = [255u8; 32];
            let _exists = db.contains(black_box(&hash));
        });
    });
    
    group.finish();
    cleanup_test_db("contains");
}

criterion_group!(benches, bench_put, bench_get_by_hash, bench_get_by_height, bench_contains);
criterion_main!(benches);

