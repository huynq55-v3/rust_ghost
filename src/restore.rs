//! Restore logic - restores a compressed image back to a partition.
//!
//! Reads the `.gho` image file, decompresses each chunk, and writes the
//! raw cluster data back to the correct positions on the target partition.

use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom};
use std::time::Instant;

use indicatif::{ProgressBar, ProgressStyle};

use crate::image::{self, ChunkEntry, ImageHeader};
use crate::winapi::VolumeHandle;

/// Restore an image file to a target partition.
pub fn restore_image(image_path: &str, target_letter: &str) -> io::Result<()> {
    println!("=== BAT DAU RESTORE ===");
    println!("Image: {}", image_path);
    println!("Dich: \\\\.\\{}:", target_letter);
    println!();

    // 1. Open and validate image file
    println!("[1/4] Dang doc file image...");
    let image_file = File::open(image_path)
        .map_err(|e| io::Error::new(e.kind(), format!("Khong the mo file image '{}': {}", image_path, e)))?;
    let mut reader = BufReader::new(image_file);

    let header = ImageHeader::read_from(&mut reader)?;
    println!("  Format version: {}", header.version);
    println!("  Cluster size: {} bytes", header.cluster_size);
    println!("  Total clusters: {}", header.total_clusters);
    println!("  Used clusters: {}", header.used_clusters);
    println!(
        "  Partition size goc: {:.2} GB",
        header.partition_size as f64 / (1024.0 * 1024.0 * 1024.0)
    );
    println!("  Nen: {}", if header.compression == 1 { "zstd" } else { "khong" });
    println!("  So chunk: {}", header.chunk_count);

    // 2. Read chunk index
    println!("[2/4] Dang doc chunk index...");
    let chunks = image::read_chunk_index(&mut reader, &header)?;
    println!("  Da doc {} chunk entries", chunks.len());

    // 3. Open target volume for writing
    println!("[3/4] Dang mo volume dich (lock & dismount)...");
    let target_volume = VolumeHandle::open_write(target_letter)?;

    // Verify target partition size is sufficient
    let target_size = target_volume.get_partition_size()?;
    let used_size = header.used_clusters * header.cluster_size as u64;
    
    // Safety check: Find the maximum offset required by any chunk
    let max_required_cluster = chunks.iter()
        .map(|c| c.start_cluster + c.cluster_count as u64)
        .max()
        .unwrap_or(0);
    let max_required_offset = max_required_cluster * header.cluster_size as u64;

    if target_size < used_size {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "Dung luong o dia dich ({:.2} GB) khong du de chua du lieu trong image ({:.2} GB)!",
                target_size as f64 / (1024.0 * 1024.0 * 1024.0),
                used_size as f64 / (1024.0 * 1024.0 * 1024.0),
            ),
        ));
    }

    if target_size < max_required_offset {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "Loi: Partition dich qua nho ({:.2} GB) so voi vi tri cluster xa nhat ({:.2} GB).\n\
                 Vui long shrink partition goc hoac su dung relocation (chua ho tro).",
                target_size as f64 / (1024.0 * 1024.0 * 1024.0),
                max_required_offset as f64 / (1024.0 * 1024.0 * 1024.0),
            ),
        ));
    }

    if target_size < header.partition_size {
        println!("  [!] CANH BAO: Partition dich ({:.2} GB) nho hon partition goc ({:.2} GB),",
            target_size as f64 / (1024.0 * 1024.0 * 1024.0),
            header.partition_size as f64 / (1024.0 * 1024.0 * 1024.0)
        );
        println!("      nhung van du de chua du lieu. Dang tiep tuc...");
    }

    // 4. Restore each chunk
    println!("[4/4] Dang restore du lieu...");
    let total_used_clusters: u64 = chunks.iter().map(|c| c.cluster_count as u64).sum();

    let pb = ProgressBar::new(total_used_clusters);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("[{elapsed_precise}] [{wide_bar:.green/white}] {pos}/{len} clusters ({percent}%) | {per_sec} | ETA: {eta}")
            .unwrap()
            .progress_chars("█▉▊▋▌▍▎▏ "),
    );

    let start_time = Instant::now();
    let bytes_per_cluster = header.cluster_size as u64;
    let mut total_restored_bytes: u64 = 0;

    for chunk in &chunks {
        // Read compressed data from image file
        reader.seek(SeekFrom::Start(chunk.compressed_offset))?;
        let mut compressed_data = vec![0u8; chunk.compressed_size as usize];
        reader.read_exact(&mut compressed_data)?;

        // Decompress
        let decompressed = zstd::decode_all(&compressed_data[..])
            .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("Giai nen zstd that bai: {}", e)))?;

        // Write decompressed data to target volume at the correct offset
        let disk_offset = chunk.start_cluster * bytes_per_cluster;

        // Write in sector-aligned chunks
        let mut written = 0usize;
        while written < decompressed.len() {
            let remaining = decompressed.len() - written;
            // Align write size to 512 bytes (sector size)
            let write_size = if remaining >= 512 {
                (remaining / 512) * 512
            } else {
                // For the last partial sector, pad to 512 bytes
                512
            };

            let mut write_buf = vec![0u8; write_size];
            let copy_len = remaining.min(write_size);
            write_buf[..copy_len].copy_from_slice(&decompressed[written..written + copy_len]);

            target_volume.write_at(
                disk_offset + written as u64,
                &write_buf,
            )?;

            written += copy_len;
        }

        total_restored_bytes += decompressed.len() as u64;
        pb.inc(chunk.cluster_count as u64);
    }

    pb.finish_with_message("Hoan tat restore!");

    // 5. Print summary
    let duration = start_time.elapsed();
    println!();
    println!("=== RESTORE HOAN TAT ===");
    println!(
        "Du lieu da ghi: {:.2} GB",
        total_restored_bytes as f64 / (1024.0 * 1024.0 * 1024.0)
    );
    println!("Thoi gian: {:.1?}", duration);
    println!(
        "Toc do trung binh: {:.1} MB/s",
        total_restored_bytes as f64 / duration.as_secs_f64() / (1024.0 * 1024.0)
    );
    println!("Volume da duoc restore thanh cong. Vui long restart may de kiem tra.");

    Ok(())
}
