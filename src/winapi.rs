//! Windows API wrappers for raw partition access.
//!
//! Provides safe wrappers around CreateFileW, DeviceIoControl, and related
//! Win32 functions needed for direct partition read/write.

use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::ptr;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_NO_BUFFERING, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::IO::DeviceIoControl;
use windows::Win32::System::Ioctl::{
    FSCTL_DISMOUNT_VOLUME, FSCTL_LOCK_VOLUME, FSCTL_UNLOCK_VOLUME,
    IOCTL_DISK_GET_LENGTH_INFO,
};

/// Represents an open volume handle with associated metadata.
pub struct VolumeHandle {
    handle: HANDLE,
    locked: bool,
}

impl VolumeHandle {
    /// Open a volume for reading (e.g., letter = "D").
    /// Opens `\\.\D:` with GENERIC_READ, FILE_FLAG_NO_BUFFERING.
    pub fn open_read(letter: &str) -> io::Result<Self> {
        let path = format!("\\\\.\\{}:", letter.trim_end_matches(':'));
        let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();

        let handle = unsafe {
            CreateFileW(
                PCWSTR(wide.as_ptr()),
                0x80000000, // GENERIC_READ
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_NO_BUFFERING,
                None,
            )
        }
        .map_err(|e| io::Error::new(io::ErrorKind::PermissionDenied, format!(
            "Khong the mo volume {}: {}. Can quyen Administrator!", path, e
        )))?;

        Ok(VolumeHandle {
            handle,
            locked: false,
        })
    }

    /// Open a volume for writing (restore mode).
    /// Opens with GENERIC_READ | GENERIC_WRITE, then locks and dismounts.
    pub fn open_write(letter: &str) -> io::Result<Self> {
        let path = format!("\\\\.\\{}:", letter.trim_end_matches(':'));
        let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();

        let handle = unsafe {
            CreateFileW(
                PCWSTR(wide.as_ptr()),
                0x80000000 | 0x40000000, // GENERIC_READ | GENERIC_WRITE
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_NO_BUFFERING,
                None,
            )
        }
        .map_err(|e| io::Error::new(io::ErrorKind::PermissionDenied, format!(
            "Khong the mo volume {} de ghi: {}. Can quyen Administrator!", path, e
        )))?;

        let mut vol = VolumeHandle {
            handle,
            locked: false,
        };

        // Lock the volume so no other process can access it
        vol.lock_volume()?;

        // Dismount the filesystem so Windows doesn't interfere
        vol.dismount_volume()?;

        Ok(vol)
    }

    /// Get the exact partition size in bytes using IOCTL_DISK_GET_LENGTH_INFO.
    pub fn get_partition_size(&self) -> io::Result<u64> {
        let mut length_info: u64 = 0;
        let mut bytes_returned: u32 = 0;

        let ok = unsafe {
            DeviceIoControl(
                self.handle,
                IOCTL_DISK_GET_LENGTH_INFO,
                None,        // no input buffer
                0,           // input buffer size
                Some(&mut length_info as *mut u64 as *mut _),
                std::mem::size_of::<u64>() as u32,
                Some(&mut bytes_returned),
                None,
            )
        };

        if ok.is_err() {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                format!("DeviceIoControl IOCTL_DISK_GET_LENGTH_INFO that bai: {:?}", ok),
            ));
        }

        Ok(length_info)
    }

    /// Get NTFS volume data including cluster size.
    /// Returns (bytes_per_cluster, total_clusters, mft_start_lcn).
    pub fn get_ntfs_volume_data(&self) -> io::Result<NtfsVolumeInfo> {
        // NTFS_VOLUME_DATA_BUFFER is 96 bytes
        #[repr(C)]
        #[derive(Default)]
        struct NtfsVolumeDataBuffer {
            volume_serial_number: i64,
            number_sectors: i64,
            total_clusters: i64,
            free_clusters: i64,
            total_reserved: i64,
            bytes_per_sector: u32,
            bytes_per_cluster: u32,
            bytes_per_file_record_segment: u32,
            clusters_per_file_record_segment: u32,
            mft_valid_data_length: i64,
            mft_start_lcn: i64,
            mft2_start_lcn: i64,
            mft_zone_start: i64,
            mft_zone_end: i64,
        }

        let mut data = NtfsVolumeDataBuffer::default();
        let mut bytes_returned: u32 = 0;

        // FSCTL_GET_NTFS_VOLUME_DATA = 0x00090064
        const FSCTL_GET_NTFS_VOLUME_DATA: u32 = 0x00090064;

        let ok = unsafe {
            DeviceIoControl(
                self.handle,
                FSCTL_GET_NTFS_VOLUME_DATA,
                None,
                0,
                Some(&mut data as *mut NtfsVolumeDataBuffer as *mut _),
                std::mem::size_of::<NtfsVolumeDataBuffer>() as u32,
                Some(&mut bytes_returned),
                None,
            )
        };

        if ok.is_err() {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                format!("FSCTL_GET_NTFS_VOLUME_DATA that bai: {:?}", ok),
            ));
        }

        Ok(NtfsVolumeInfo {
            bytes_per_sector: data.bytes_per_sector,
            bytes_per_cluster: data.bytes_per_cluster,
            total_clusters: data.total_clusters as u64,
            free_clusters: data.free_clusters as u64,
            mft_start_lcn: data.mft_start_lcn as u64,
        })
    }

    /// Get the raw Windows HANDLE.
    pub fn raw_handle(&self) -> HANDLE {
        self.handle
    }

    /// Lock the volume (prevents other processes from accessing it).
    fn lock_volume(&mut self) -> io::Result<()> {
        let mut bytes_returned: u32 = 0;
        let ok = unsafe {
            DeviceIoControl(
                self.handle,
                FSCTL_LOCK_VOLUME,
                None,
                0,
                None,
                0,
                Some(&mut bytes_returned),
                None,
            )
        };

        if ok.is_err() {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "Khong the lock volume. Co chuong trinh khac dang su dung?",
            ));
        }

        self.locked = true;
        Ok(())
    }

    /// Dismount the volume's filesystem.
    fn dismount_volume(&self) -> io::Result<()> {
        let mut bytes_returned: u32 = 0;
        let ok = unsafe {
            DeviceIoControl(
                self.handle,
                FSCTL_DISMOUNT_VOLUME,
                None,
                0,
                None,
                0,
                Some(&mut bytes_returned),
                None,
            )
        };

        if ok.is_err() {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "Khong the dismount volume.",
            ));
        }

        Ok(())
    }

    /// Unlock the volume.
    fn unlock_volume(&mut self) -> io::Result<()> {
        if !self.locked {
            return Ok(());
        }

        let mut bytes_returned: u32 = 0;
        let _ = unsafe {
            DeviceIoControl(
                self.handle,
                FSCTL_UNLOCK_VOLUME,
                None,
                0,
                None,
                0,
                Some(&mut bytes_returned),
                None,
            )
        };

        self.locked = false;
        Ok(())
    }

    /// Read data at a specific byte offset (must be sector-aligned).
    pub fn read_at(&self, offset: u64, buffer: &mut [u8]) -> io::Result<u32> {
        use windows::Win32::Storage::FileSystem::ReadFile;
        use windows::Win32::Storage::FileSystem::SetFilePointerEx;
        use windows::Win32::Storage::FileSystem::FILE_BEGIN;

        unsafe {
            SetFilePointerEx(self.handle, offset as i64, None, FILE_BEGIN)
                .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("Seek that bai: {}", e)))?;
        }

        let mut bytes_read: u32 = 0;
        unsafe {
            ReadFile(
                self.handle,
                Some(buffer),
                Some(&mut bytes_read),
                None,
            )
            .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("ReadFile that bai: {}", e)))?;
        }

        Ok(bytes_read)
    }

    /// Write data at a specific byte offset (must be sector-aligned).
    pub fn write_at(&self, offset: u64, buffer: &[u8]) -> io::Result<u32> {
        use windows::Win32::Storage::FileSystem::WriteFile;
        use windows::Win32::Storage::FileSystem::SetFilePointerEx;
        use windows::Win32::Storage::FileSystem::FILE_BEGIN;

        unsafe {
            SetFilePointerEx(self.handle, offset as i64, None, FILE_BEGIN)
                .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("Seek that bai: {}", e)))?;
        }

        let mut bytes_written: u32 = 0;
        unsafe {
            WriteFile(
                self.handle,
                Some(buffer),
                Some(&mut bytes_written),
                None,
            )
            .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("WriteFile that bai: {}", e)))?;
        }

        Ok(bytes_written)
    }
}

impl Drop for VolumeHandle {
    fn drop(&mut self) {
        let _ = self.unlock_volume();
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}

/// NTFS volume information retrieved from FSCTL_GET_NTFS_VOLUME_DATA.
#[derive(Debug, Clone)]
pub struct NtfsVolumeInfo {
    pub bytes_per_sector: u32,
    pub bytes_per_cluster: u32,
    pub total_clusters: u64,
    pub free_clusters: u64,
    pub mft_start_lcn: u64,
}

/// Create a sector-aligned buffer (alignment is typically 512 bytes).
pub fn aligned_buffer(size: usize) -> Vec<u8> {
    // For FILE_FLAG_NO_BUFFERING, reads must be sector-aligned (512 bytes).
    // Vec<u8> on Windows already has sufficient alignment for this.
    // We round up size to the nearest sector boundary.
    let sector_size = 512usize;
    let aligned_size = (size + sector_size - 1) / sector_size * sector_size;
    vec![0u8; aligned_size]
}

/// List all available volume letters on the system.
pub fn list_volumes() -> io::Result<Vec<VolumeInfo>> {
    use windows::Win32::Storage::FileSystem::GetLogicalDrives;
    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    let drives_bitmask = unsafe { GetLogicalDrives() };
    if drives_bitmask == 0 {
        return Err(io::Error::last_os_error());
    }

    let mut volumes = Vec::new();

    for i in 0..26u32 {
        if drives_bitmask & (1 << i) != 0 {
            let letter = (b'A' + i as u8) as char;
            let root = format!("{}:\\", letter);
            let wide_root: Vec<u16> = root.encode_utf16().chain(std::iter::once(0)).collect();

            let mut free_bytes: u64 = 0;
            let mut total_bytes: u64 = 0;
            let mut total_free_bytes: u64 = 0;

            let ok = unsafe {
                GetDiskFreeSpaceExW(
                    PCWSTR(wide_root.as_ptr()),
                    Some(&mut free_bytes),
                    Some(&mut total_bytes),
                    Some(&mut total_free_bytes),
                )
            };

            if ok.is_ok() {
                volumes.push(VolumeInfo {
                    letter,
                    total_bytes,
                    free_bytes: total_free_bytes,
                });
            }
        }
    }

    Ok(volumes)
}

/// Information about an available volume.
#[derive(Debug, Clone)]
pub struct VolumeInfo {
    pub letter: char,
    pub total_bytes: u64,
    pub free_bytes: u64,
}

impl VolumeInfo {
    /// Format total size as human-readable string.
    pub fn total_display(&self) -> String {
        format_bytes(self.total_bytes)
    }

    /// Format free space as human-readable string.
    pub fn free_display(&self) -> String {
        format_bytes(self.free_bytes)
    }
}

fn format_bytes(bytes: u64) -> String {
    const GB: u64 = 1024 * 1024 * 1024;
    const MB: u64 = 1024 * 1024;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    }
}
