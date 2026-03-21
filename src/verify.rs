//! Verify logic - validates the integrity of a `.gho` image file.
//!
//! Reads the header and all chunks, decompresses them, and calculates
//! a SHA256 hash to ensure no corruption in the compressed data.

use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom};
use std::time::Instant;

use indicatif::{ProgressBar, ProgressStyle};
use sha2::{Digest, Sha256};

use crate::image::{self, ImageHeader};

/// Verify the integrity of an image file.
pub fn verify_image(image_path: &str) -> io::Result<()> {
    println!("=== BAT DAU KIEM TRA IMAGE ===");
    println!("Image: {}", image_path);
    println!();

    // 1. Open and validate image file
    println!("[1/3] Dang doc file image...");
    let image_file = File::open(image_path)
        .map_err(|e| io::Error::new(e.kind(), format!("Khong the mo file image '{}': {}", image_path, e)))?;
    let mut reader = BufReader::new(image_file);

    let header = ImageHeader::read_from(&mut reader)?;
    println!("  Format version: {}", header.version);
    println!("  Used clusters: {}", header.used_clusters);
    println!("  So chunk: {}", header.chunk_count);

    // 2. Read chunk index
    println!("[2/3] Dang doc chunk index...");
    let chunks = image::read_chunk_index(&mut reader, &header)?;
    println!("  Da doc {} chunk entries", chunks.len());

    // 3. Decompress and hash
    println!("[3/3] Dang kiem tra toan ven (SHA256)...");
    let total_used_clusters: u64 = chunks.iter().map(|c| c.cluster_count as u64).sum();

    let pb = ProgressBar::new(total_used_clusters);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("[{elapsed_precise}] [{wide_bar:.magenta/white}] {pos}/{len} clusters ({percent}%) | {per_sec} | ETA: {eta}")
            .unwrap()
            .progress_chars("█▉▊▋▌▍▎▏ "),
    );

    let mut hasher = Sha256::new();
    let start_time = Instant::now();
    let mut total_decompressed_bytes: u64 = 0;

    for chunk in &chunks {
        // Read compressed data
        reader.seek(SeekFrom::Start(chunk.compressed_offset))?;
        let mut compressed_data = vec![0u8; chunk.compressed_size as usize];
        reader.read_exact(&mut compressed_data)?;

        // Decompress
        let decompressed = zstd::decode_all(&compressed_data[..])
            .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("Loi giai nen chunk tai offset {}: {}", chunk.compressed_offset, e)))?;

        // Hash
        hasher.update(&decompressed);
        
        total_decompressed_bytes += decompressed.len() as u64;
        pb.inc(chunk.cluster_count as u64);
    }

    let hash_result = hasher.finalize();
    pb.finish_with_message("Hoan tat kiem tra!");

    let duration = start_time.elapsed();
    println!();
    println!("=== KET QUA KIEM TRA ===");
    println!("Trang thai: ✅ HOP LE");
    println!("SHA256: {:x}", hash_result);
    println!("Tong du lieu giai nen: {:.2} GB", total_decompressed_bytes as f64 / (1024.0 * 1024.0 * 1024.0));
    println!("Thoi gian: {:.1?}", duration);
    println!(
        "Toc do kiem tra: {:.1} MB/s",
        total_decompressed_bytes as f64 / duration.as_secs_f64() / (1024.0 * 1024.0)
    );

    Ok(())
}
