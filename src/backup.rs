//! Backup logic - creates a compressed image from an NTFS partition.
//!
//! Reads only used clusters (based on NTFS bitmap), compresses them with zstd,
//! and writes to a custom `.gho` image file.

use std::fs::File;
use std::io::{self, BufWriter, Seek, SeekFrom, Write};
use std::time::Instant;

use indicatif::{ProgressBar, ProgressStyle};

use crate::image::{ChunkEntry, ImageHeader, HEADER_SIZE};
use crate::ntfs_bitmap::NtfsBitmapReader;
use crate::winapi::VolumeHandle;

/// Maximum clusters to read in one chunk before compressing.
/// This controls memory usage: chunk_clusters * bytes_per_cluster = memory per chunk.
/// With 4096 bytes/cluster and 1024 clusters = ~4MB per chunk.
const MAX_CLUSTERS_PER_CHUNK: u64 = 1024;

/// Create a backup image of the given volume.
pub fn create_backup(source_letter: &str, dest_path: &str, zstd_level: i32) -> io::Result<()> {
    println!("=== BAT DAU BACKUP ===");
    println!("Nguon: \\\\.\\{}:", source_letter);
    println!("Dich: {}", dest_path);
    println!();

    // 1. Open source volume
    println!("[1/5] Dang mo volume nguon...");
    let volume = VolumeHandle::open_read(source_letter)?;

    // 2. Get partition info
    println!("[2/5] Dang lay thong tin partition...");
    let partition_size = volume.get_partition_size()?;
    let vol_info = volume.get_ntfs_volume_data()?;
    println!(
        "  Partition size: {:.2} GB",
        partition_size as f64 / (1024.0 * 1024.0 * 1024.0)
    );
    println!("  Bytes per cluster: {}", vol_info.bytes_per_cluster);
    println!("  Total clusters: {}", vol_info.total_clusters);

    // 3. Read NTFS bitmap
    println!("[3/5] Dang doc NTFS Bitmap (ban do cluster)...");
    let bitmap = NtfsBitmapReader::read_bitmap(&volume)?;
    let stats = bitmap.stats();
    println!("  {}", stats);
    println!(
        "  Chi can backup: {:.2} GB (tiet kiem {:.1}%)",
        stats.used_bytes as f64 / (1024.0 * 1024.0 * 1024.0),
        (1.0 - stats.used_bytes as f64 / stats.total_bytes as f64) * 100.0
    );

    // 4. Get used cluster ranges
    let ranges = bitmap.used_cluster_ranges();
    println!(
        "  So vung cluster lien tuc: {} ranges",
        ranges.len()
    );

    // 5. Create image file and write header
    println!("[4/5] Dang tao file image...");
    let image_file = File::create(dest_path)?;
    let mut writer = BufWriter::new(image_file);

    let mut header = ImageHeader::new(
        vol_info.bytes_per_cluster,
        vol_info.total_clusters,
        stats.used_clusters,
        partition_size,
        zstd_level as u8,
    );

    // Write preliminary header (will be updated later with chunk count and index offset)
    header.write_to(&mut writer)?;

    // Move past header to start writing chunk data
    writer.seek(SeekFrom::Start(HEADER_SIZE))?;

    // 6. Read, compress, and write chunks
    println!("[5/5] Dang backup & nen du lieu...");
    let pb = ProgressBar::new(stats.used_clusters);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("[{elapsed_precise}] [{wide_bar:.cyan/blue}] {pos}/{len} clusters ({percent}%) | {per_sec} | ETA: {eta}")
            .unwrap()
            .progress_chars("█▉▊▋▌▍▎▏ "),
    );

    let bytes_per_cluster = vol_info.bytes_per_cluster as u64;
    let mut chunk_entries: Vec<ChunkEntry> = Vec::new();
    let mut current_offset = HEADER_SIZE;
    let start_time = Instant::now();
    let mut total_compressed_bytes: u64 = 0;

    // Read buffer (sector-aligned)
    let read_buf_size = MAX_CLUSTERS_PER_CHUNK as usize * bytes_per_cluster as usize;

    for range in &ranges {
        // Process this range in sub-chunks of MAX_CLUSTERS_PER_CHUNK
        let mut remaining = range.count;
        let mut range_offset = 0u64;

        while remaining > 0 {
            let chunk_clusters = remaining.min(MAX_CLUSTERS_PER_CHUNK);
            let chunk_start_cluster = range.start + range_offset;
            let chunk_bytes = chunk_clusters * bytes_per_cluster;

            // Read raw clusters from volume
            let mut raw_data = vec![0u8; chunk_bytes as usize];
            let disk_offset = chunk_start_cluster * bytes_per_cluster;

            let mut total_read = 0usize;
            while total_read < chunk_bytes as usize {
                let to_read = (chunk_bytes as usize - total_read)
                    .min(read_buf_size);
                // Ensure read size is sector-aligned (512 bytes)
                let aligned_read = ((to_read + 511) / 512) * 512;
                let mut aligned_buf = vec![0u8; aligned_read];

                let bytes_read = volume.read_at(
                    disk_offset + total_read as u64,
                    &mut aligned_buf,
                )?;

                if bytes_read == 0 {
                    break;
                }

                let copy_len = (bytes_read as usize).min(chunk_bytes as usize - total_read);
                raw_data[total_read..total_read + copy_len]
                    .copy_from_slice(&aligned_buf[..copy_len]);
                total_read += copy_len;
            }

            // Compress with zstd
            let compressed = zstd::encode_all(&raw_data[..total_read], zstd_level)
                .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("Nen zstd that bai: {}", e)))?;

            // Write compressed data to image file
            writer.write_all(&compressed)?;

            // Record chunk entry
            chunk_entries.push(ChunkEntry {
                start_cluster: chunk_start_cluster,
                cluster_count: chunk_clusters as u32,
                compressed_offset: current_offset,
                compressed_size: compressed.len() as u64,
                original_size: total_read as u64,
            });

            current_offset += compressed.len() as u64;
            total_compressed_bytes += compressed.len() as u64;

            pb.inc(chunk_clusters);

            remaining -= chunk_clusters;
            range_offset += chunk_clusters;
        }
    }

    pb.finish_with_message("Hoan tat nen du lieu!");

    // 7. Write chunk index table at the end
    let index_offset = current_offset;
    for entry in &chunk_entries {
        entry.write_to(&mut writer)?;
    }

    // 8. Update header with final chunk count and index offset
    header.chunk_count = chunk_entries.len() as u32;
    header.index_offset = index_offset;
    header.write_to(&mut writer)?;

    writer.flush()?;

    // 9. Print summary
    let duration = start_time.elapsed();
    let original_bytes = stats.used_bytes;

    println!();
    println!("=== BACKUP HOAN TAT ===");
    println!("Du lieu goc (chi cluster da dung): {:.2} GB", original_bytes as f64 / (1024.0 * 1024.0 * 1024.0));
    println!("Du lieu sau nen: {:.2} GB", total_compressed_bytes as f64 / (1024.0 * 1024.0 * 1024.0));
    println!(
        "Ti le nen: {:.1}%",
        (1.0 - total_compressed_bytes as f64 / original_bytes as f64) * 100.0
    );
    println!("So chunk: {}", chunk_entries.len());
    println!("Thoi gian: {:.1?}", duration);
    println!(
        "Toc do trung binh: {:.1} MB/s",
        original_bytes as f64 / duration.as_secs_f64() / (1024.0 * 1024.0)
    );

    Ok(())
}
