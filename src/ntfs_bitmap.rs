//! NTFS Bitmap reader - determines which clusters are used on an NTFS volume.
//!
//! Reads the NTFS `$Bitmap` file (MFT entry 6) to build a map of used/free
//! clusters. This allows the backup to skip empty clusters, producing smaller images.

use std::io::{self, Read, Seek, SeekFrom};

use crate::winapi::VolumeHandle;

/// Reads the NTFS Volume Bitmap directly using Windows API.
///
/// Instead of using the `ntfs` crate to parse MFT entry 6, we use
/// `FSCTL_GET_VOLUME_BITMAP` which is the proper Windows API to retrieve
/// the volume's cluster allocation bitmap.
pub struct NtfsBitmapReader {
    /// One bit per cluster: true = used, false = free
    bitmap: Vec<u8>,
    /// Total number of clusters on the volume
    total_clusters: u64,
    /// Bytes per cluster
    bytes_per_cluster: u32,
}

/// A contiguous range of used clusters.
#[derive(Debug, Clone, Copy)]
pub struct ClusterRange {
    /// Starting cluster number (LCN)
    pub start: u64,
    /// Number of clusters in this range
    pub count: u64,
}

impl NtfsBitmapReader {
    /// Read the volume bitmap using FSCTL_GET_VOLUME_BITMAP.
    pub fn read_bitmap(volume: &VolumeHandle) -> io::Result<Self> {
        let vol_info = volume.get_ntfs_volume_data()?;
        let total_clusters = vol_info.total_clusters;
        let bytes_per_cluster = vol_info.bytes_per_cluster;

        // We'll read the bitmap using FSCTL_GET_VOLUME_BITMAP
        // Input: starting LCN (8 bytes, i64)
        // Output: VOLUME_BITMAP_BUFFER { StartingLcn: i64, BitmapSize: i64, Buffer: [u8] }
        let bitmap_byte_count = ((total_clusters + 7) / 8) as usize;

        // Output buffer: 16 bytes header + bitmap data
        let output_size = 16 + bitmap_byte_count;
        let mut output_buffer = vec![0u8; output_size];

        // Input: starting LCN = 0
        let starting_lcn: i64 = 0;
        let mut bytes_returned: u32 = 0;

        // FSCTL_GET_VOLUME_BITMAP = 0x0009006F
        const FSCTL_GET_VOLUME_BITMAP: u32 = 0x0009006F;

        let result = unsafe {
            windows::Win32::System::IO::DeviceIoControl(
                volume.raw_handle(),
                FSCTL_GET_VOLUME_BITMAP,
                Some(&starting_lcn as *const i64 as *const _),
                std::mem::size_of::<i64>() as u32,
                Some(output_buffer.as_mut_ptr() as *mut _),
                output_size as u32,
                Some(&mut bytes_returned),
                None,
            )
        };

        if result.is_err() {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                format!(
                    "FSCTL_GET_VOLUME_BITMAP that bai: {:?}. Can quyen Administrator!",
                    result
                ),
            ));
        }

        // Parse output: first 8 bytes = StartingLcn (i64), next 8 bytes = BitmapSize (i64)
        // Rest = bitmap data
        let bitmap_data = output_buffer[16..].to_vec();

        Ok(NtfsBitmapReader {
            bitmap: bitmap_data,
            total_clusters,
            bytes_per_cluster,
        })
    }

    /// Check if a cluster is used (allocated).
    pub fn is_cluster_used(&self, cluster: u64) -> bool {
        if cluster >= self.total_clusters {
            return false;
        }
        let byte_index = (cluster / 8) as usize;
        let bit_index = (cluster % 8) as u8;
        if byte_index >= self.bitmap.len() {
            return false;
        }
        (self.bitmap[byte_index] >> bit_index) & 1 == 1
    }

    /// Get contiguous ranges of used clusters for efficient sequential I/O.
    /// This merges adjacent used clusters into ranges.
    pub fn used_cluster_ranges(&self) -> Vec<ClusterRange> {
        let mut ranges = Vec::new();
        let mut range_start: Option<u64> = None;
        let mut range_count: u64 = 0;

        for cluster in 0..self.total_clusters {
            if self.is_cluster_used(cluster) {
                if range_start.is_none() {
                    range_start = Some(cluster);
                    range_count = 1;
                } else {
                    range_count += 1;
                }
            } else if let Some(start) = range_start {
                ranges.push(ClusterRange {
                    start,
                    count: range_count,
                });
                range_start = None;
                range_count = 0;
            }
        }

        // Don't forget the last range
        if let Some(start) = range_start {
            ranges.push(ClusterRange {
                start,
                count: range_count,
            });
        }

        ranges
    }

    /// Get statistics about the bitmap.
    pub fn stats(&self) -> BitmapStats {
        let mut used = 0u64;
        for cluster in 0..self.total_clusters {
            if self.is_cluster_used(cluster) {
                used += 1;
            }
        }

        BitmapStats {
            total_clusters: self.total_clusters,
            used_clusters: used,
            free_clusters: self.total_clusters - used,
            bytes_per_cluster: self.bytes_per_cluster,
            used_bytes: used * self.bytes_per_cluster as u64,
            total_bytes: self.total_clusters * self.bytes_per_cluster as u64,
        }
    }

    /// Total clusters on the volume.
    pub fn total_clusters(&self) -> u64 {
        self.total_clusters
    }

    /// Bytes per cluster.
    pub fn bytes_per_cluster(&self) -> u32 {
        self.bytes_per_cluster
    }
}

/// Statistics about the volume bitmap.
#[derive(Debug, Clone)]
pub struct BitmapStats {
    pub total_clusters: u64,
    pub used_clusters: u64,
    pub free_clusters: u64,
    pub bytes_per_cluster: u32,
    pub used_bytes: u64,
    pub total_bytes: u64,
}

impl std::fmt::Display for BitmapStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Tong cluster: {} | Da dung: {} ({:.1}%) | Trong: {} | Cluster size: {} bytes",
            self.total_clusters,
            self.used_clusters,
            (self.used_clusters as f64 / self.total_clusters as f64) * 100.0,
            self.free_clusters,
            self.bytes_per_cluster,
        )
    }
}
