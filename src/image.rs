//! Custom `.gho` image file format.
//!
//! The format consists of:
//! 1. A fixed-size header (512 bytes) with metadata
//! 2. A chunk index table (variable size, written after all chunks)
//! 3. Compressed data chunks (zstd-compressed raw cluster data)

use std::io::{self, Read, Write, Seek, SeekFrom};
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};

/// Magic bytes identifying a RustGhost image file.
pub const MAGIC: &[u8; 8] = b"RUSTGHO\0";

/// Current image format version.
pub const VERSION: u32 = 1;

/// Size of the image header in bytes.
pub const HEADER_SIZE: u64 = 512;

/// Image file header (512 bytes).
#[derive(Debug, Clone)]
pub struct ImageHeader {
    /// Magic bytes: "RUSTGHO\0"
    pub magic: [u8; 8],
    /// Format version
    pub version: u32,
    /// Bytes per cluster on the source volume
    pub cluster_size: u32,
    /// Total clusters on the source volume
    pub total_clusters: u64,
    /// Number of used (allocated) clusters backed up
    pub used_clusters: u64,
    /// Total partition size in bytes
    pub partition_size: u64,
    /// Compression type: 0=none, 1=zstd
    pub compression: u8,
    /// Compression level (for zstd, typically 1-22)
    pub zstd_level: u8,
    /// Number of chunk entries in the index
    pub chunk_count: u32,
    /// Byte offset where the chunk index table starts
    pub index_offset: u64,
}

impl ImageHeader {
    /// Create a new header with the given parameters.
    pub fn new(
        cluster_size: u32,
        total_clusters: u64,
        used_clusters: u64,
        partition_size: u64,
        zstd_level: u8,
    ) -> Self {
        ImageHeader {
            magic: *MAGIC,
            version: VERSION,
            cluster_size,
            total_clusters,
            used_clusters,
            partition_size,
            compression: 1, // zstd
            zstd_level,
            chunk_count: 0,       // filled in later
            index_offset: 0,      // filled in later
        }
    }

    /// Write the header to a writer at the current position.
    pub fn write_to<W: Write + Seek>(&self, w: &mut W) -> io::Result<()> {
        w.seek(SeekFrom::Start(0))?;

        w.write_all(&self.magic)?;                          // 8
        w.write_u32::<LittleEndian>(self.version)?;         // 4
        w.write_u32::<LittleEndian>(self.cluster_size)?;    // 4
        w.write_u64::<LittleEndian>(self.total_clusters)?;  // 8
        w.write_u64::<LittleEndian>(self.used_clusters)?;   // 8
        w.write_u64::<LittleEndian>(self.partition_size)?;  // 8
        w.write_u8(self.compression)?;                      // 1
        w.write_u8(self.zstd_level)?;                       // 1
        w.write_u32::<LittleEndian>(self.chunk_count)?;     // 4
        w.write_u64::<LittleEndian>(self.index_offset)?;    // 8
        // Total so far: 54 bytes, pad to 512
        let padding = vec![0u8; HEADER_SIZE as usize - 54];
        w.write_all(&padding)?;

        Ok(())
    }

    /// Read the header from a reader.
    pub fn read_from<R: Read + Seek>(r: &mut R) -> io::Result<Self> {
        r.seek(SeekFrom::Start(0))?;

        let mut magic = [0u8; 8];
        r.read_exact(&mut magic)?;

        if &magic != MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "File khong phai dinh dang RustGhost (.gho). Magic bytes khong khop!",
            ));
        }

        let version = r.read_u32::<LittleEndian>()?;
        if version != VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Phien ban format khong ho tro: {}. Chi ho tro v{}", version, VERSION),
            ));
        }

        let cluster_size = r.read_u32::<LittleEndian>()?;
        let total_clusters = r.read_u64::<LittleEndian>()?;
        let used_clusters = r.read_u64::<LittleEndian>()?;
        let partition_size = r.read_u64::<LittleEndian>()?;
        let compression = r.read_u8()?;
        let zstd_level = r.read_u8()?;
        let chunk_count = r.read_u32::<LittleEndian>()?;
        let index_offset = r.read_u64::<LittleEndian>()?;

        Ok(ImageHeader {
            magic,
            version,
            cluster_size,
            total_clusters,
            used_clusters,
            partition_size,
            compression,
            zstd_level,
            chunk_count,
            index_offset,
        })
    }
}

/// A chunk entry in the image index table.
/// Each chunk represents a contiguous range of clusters that was compressed and stored.
#[derive(Debug, Clone)]
pub struct ChunkEntry {
    /// Starting cluster number (LCN) of this chunk
    pub start_cluster: u64,
    /// Number of clusters in this chunk
    pub cluster_count: u32,
    /// Byte offset of the compressed data in the image file
    pub compressed_offset: u64,
    /// Size of the compressed data in bytes
    pub compressed_size: u64,
    /// Original uncompressed size in bytes
    pub original_size: u64,
}

impl ChunkEntry {
    /// Size of one ChunkEntry when serialized (in bytes).
    pub const SERIALIZED_SIZE: usize = 8 + 4 + 8 + 8 + 8; // 36 bytes

    /// Write this chunk entry to a writer.
    pub fn write_to<W: Write>(&self, w: &mut W) -> io::Result<()> {
        w.write_u64::<LittleEndian>(self.start_cluster)?;
        w.write_u32::<LittleEndian>(self.cluster_count)?;
        w.write_u64::<LittleEndian>(self.compressed_offset)?;
        w.write_u64::<LittleEndian>(self.compressed_size)?;
        w.write_u64::<LittleEndian>(self.original_size)?;
        Ok(())
    }

    /// Read a chunk entry from a reader.
    pub fn read_from<R: Read>(r: &mut R) -> io::Result<Self> {
        let start_cluster = r.read_u64::<LittleEndian>()?;
        let cluster_count = r.read_u32::<LittleEndian>()?;
        let compressed_offset = r.read_u64::<LittleEndian>()?;
        let compressed_size = r.read_u64::<LittleEndian>()?;
        let original_size = r.read_u64::<LittleEndian>()?;

        Ok(ChunkEntry {
            start_cluster,
            cluster_count,
            compressed_offset,
            compressed_size,
            original_size,
        })
    }
}

/// Write the chunk index table to a writer.
pub fn write_chunk_index<W: Write>(w: &mut W, chunks: &[ChunkEntry]) -> io::Result<()> {
    for chunk in chunks {
        chunk.write_to(w)?;
    }
    Ok(())
}

/// Read the chunk index table from a reader.
pub fn read_chunk_index<R: Read + Seek>(r: &mut R, header: &ImageHeader) -> io::Result<Vec<ChunkEntry>> {
    r.seek(SeekFrom::Start(header.index_offset))?;

    let mut chunks = Vec::with_capacity(header.chunk_count as usize);
    for _ in 0..header.chunk_count {
        chunks.push(ChunkEntry::read_from(r)?);
    }

    Ok(chunks)
}
