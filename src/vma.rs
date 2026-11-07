use crate::utils::{SequentialReader, get_progress_bar};
use anyhow::Context;
use bincode::{Decode, Encode, decode_from_std_read, encode_into_std_write};
use byteorder::{BigEndian, ByteOrder, LittleEndian};
use chrono::DateTime;
use std::fs::File;
use std::io::{Read, Seek, Write};
use std::os::unix::fs::FileExt;
use uuid::Uuid;

pub const INDEX_VERSION: u8 = 2;

pub const HEADER_MAGIC: [u8; 4] = *b"VMA\0";
pub const HEADER_SIZE: usize = 12288;
pub const HEADER_DEV_INFO_OFFSET: usize = 4096;
pub const HEADER_DEV_INFO_SIZE: usize = 32;
pub const HEADER_DEV_INFO_LEN: usize = 256;

pub const EXTENT_MAX_SIZE: usize = EXTENT_HEADER_SIZE + (CLUSTER_SIZE * EXTENT_HEADER_BLOCKINFO_LEN);
pub const EXTENT_HEADER_MAGIC: [u8; 4] = *b"VMAE";
pub const EXTENT_HEADER_SIZE: usize = 512;
pub const EXTENT_HEADER_BLOCKINFO_OFFSET: usize = 40;
pub const EXTENT_HEADER_BLOCKINFO_SIZE: usize = 8;
pub const EXTENT_HEADER_BLOCKINFO_LEN: usize = 59;

pub const CLUSTER_SIZE: usize = 65536;
pub const BLOCK_SIZE: usize = 4096;

pub const HEADER_SIZE_LIMIT: usize = 1024 * 1024 * 1024; // 1 GiB
pub const DEVICE_SIZE_LIMIT: usize = 10 * 1024 * 1024 * 1024 * 1024; // 10 TiB

#[derive(Clone, Encode, Decode)]
pub struct VMAIndexDevCluster {
    pub mask: u16,
    pub first_block_offset: u64,
}

impl VMAIndexDevCluster {
    pub fn is_empty(&self) -> bool {
        self.first_block_offset == 0
    }
}

#[derive(Encode, Decode)]
pub struct VMAIndexDevInfo {
    pub name: String,
    pub size: u64,
    pub clusters: Vec<VMAIndexDevCluster>,
    pub stored_blocks: u64,
}

pub struct VMAIndex {
    pub dev_info: [Option<VMAIndexDevInfo>; HEADER_DEV_INFO_LEN],
    bytes: Vec<u8>,
}

#[derive(Encode, Decode)]
pub struct VMASavedIndex {
    pub version: u8,
    pub dev_info: [Option<VMAIndexDevInfo>; HEADER_DEV_INFO_LEN],
    pub bytes: Vec<u8>,
}

impl Default for VMAIndex {
    fn default() -> Self {
        Self {
            bytes: vec![0u8; HEADER_SIZE],
            dev_info: [const { None }; HEADER_DEV_INFO_LEN],
        }
    }
}

impl VMAIndex {
    fn parse(&mut self, ignore_limits: bool) -> anyhow::Result<usize> {
        anyhow::ensure!(
            self.bytes[0..4] == HEADER_MAGIC,
            "Invalid magic string. Is this a .vma file?"
        );
        let version = BigEndian::read_u32(&self.bytes[4..8]);
        anyhow::ensure!(version == 1, "Unsupported version: {}", version);

        let blob_buffer_offset = BigEndian::read_u32(&self.bytes[48..52]);
        let blob_buffer_size = BigEndian::read_u32(&self.bytes[52..56]);
        let header_size = BigEndian::read_u32(&self.bytes[56..60]);

        anyhow::ensure!(
            blob_buffer_offset % 512 == 0,
            "File is corrupted, blob_buffer_offset % 512 != 0"
        );
        anyhow::ensure!(header_size % 512 == 0, "File is corrupted, header_size % 512 != 0");
        anyhow::ensure!(
            blob_buffer_offset + blob_buffer_size <= header_size,
            "File is corrupted, blob_buffer_offset + blob_buffer_size > header_size"
        );
        anyhow::ensure!(
            header_size >= HEADER_SIZE as u32,
            "File is corrupted, header declares an invalid size: {}",
            header_size
        );

        anyhow::ensure!(
            (header_size as usize) <= HEADER_SIZE_LIMIT || ignore_limits,
            "Header is too large: {}. Use --ignore-limits if you are sure that the file is not corrupted.",
            header_size
        );
        self.bytes.resize(header_size as usize, 0);
        Ok(blob_buffer_offset as usize)
    }

    fn parse_devs(&mut self, blob_buffer_offset: usize, ignore_limits: bool) -> anyhow::Result<()> {
        let ctime = BigEndian::read_u64(&self.bytes[24..32]);
        log::info!(
            "Dump {} created at {}",
            Uuid::from_bytes_ref((&self.bytes[8..24]).try_into()?),
            DateTime::from_timestamp(ctime as i64, 0)
                .map_or_else(|| format!("invalid timestamp {ctime}"), |t| t.to_string())
        );

        let blob = &self.bytes[blob_buffer_offset..];
        let devsinfo =
            &self.bytes[HEADER_DEV_INFO_OFFSET..HEADER_DEV_INFO_OFFSET + HEADER_DEV_INFO_LEN * HEADER_DEV_INFO_SIZE];
        for (i, devinfo) in devsinfo.as_chunks::<HEADER_DEV_INFO_SIZE>().0.iter().enumerate() {
            let mut dev = VMAIndexDevInfo {
                name: "".to_string(),
                size: BigEndian::read_u64(&devinfo[8..16]),
                clusters: vec![],
                stored_blocks: 0,
            };
            if dev.size == 0 {
                continue;
            }
            anyhow::ensure!(
                (dev.size as usize) <= DEVICE_SIZE_LIMIT || ignore_limits,
                "Device is too large: {}. Use --ignore-limits if you are sure that the file is not corrupted.",
                dev.size
            );

            let name_size_pos = BigEndian::read_u32(&devinfo[0..4]) as usize;
            let name_pos_start = name_size_pos
                .checked_add(2)
                .ok_or_else(|| anyhow::anyhow!("Name size pos overflow: {}", name_size_pos))?;
            anyhow::ensure!(
                name_pos_start <= blob.len(),
                "Dev {} name size pos {} is outside the blob",
                i,
                name_size_pos
            );

            // The VMA source uses little-endian name lengths despite the specification's big-endian rule.
            let name_size = LittleEndian::read_u16(&blob[name_size_pos..name_pos_start]);
            let name_pos_end = name_pos_start
                .checked_add(name_size as usize)
                .ok_or_else(|| anyhow::anyhow!("Name size overflow: {}, pos: {}", name_size, name_pos_start))?;
            anyhow::ensure!(
                name_pos_end <= blob.len(),
                "Dev {} name pos is outside the blob: {}",
                i,
                name_pos_end
            );

            dev.name = String::from_utf8_lossy(&blob[name_pos_start..name_pos_end]).to_string();

            log::info!("Device {}, size {}, id={}", dev.name, dev.size, i);
            self.dev_info[i] = Some(dev);
        }

        Ok(())
    }

    fn allocate_clusters(&mut self) {
        for dev in self.dev_info.iter_mut().flatten() {
            dev.clusters.resize(
                dev.size.div_ceil(CLUSTER_SIZE as u64) as usize,
                VMAIndexDevCluster {
                    mask: 0,
                    first_block_offset: 0,
                },
            );
        }
    }

    pub fn index_extent_header(&mut self, bytes: &mut [u8; EXTENT_HEADER_SIZE], data_pos: u64) -> anyhow::Result<u64> {
        anyhow::ensure!(
            bytes[0..4] == EXTENT_HEADER_MAGIC,
            "Invalid magic string for extent at {}",
            data_pos
        );

        let original_digest = md5::Digest(bytes[24..40].try_into()?);
        bytes[24..40].fill(0);
        let digest = md5::compute(&bytes);
        bytes[24..40].copy_from_slice(original_digest.as_ref());
        anyhow::ensure!(
            original_digest == digest,
            "Header checksum mismatch for extent at {}: expected {:x}, got {:x}",
            data_pos,
            original_digest,
            digest
        );

        anyhow::ensure!(
            bytes[8..24] == self.bytes[8..24],
            "Extent at {} belongs to {}, not to the given file {}",
            data_pos,
            Uuid::from_bytes_ref((&bytes[8..24]).try_into()?),
            Uuid::from_bytes_ref((&self.bytes[8..24]).try_into()?)
        );

        let mut last_block_offset = data_pos;
        let mut found_blocks = 0u16;

        let blocksinfo = &bytes[EXTENT_HEADER_BLOCKINFO_OFFSET
            ..EXTENT_HEADER_BLOCKINFO_OFFSET + EXTENT_HEADER_BLOCKINFO_LEN * EXTENT_HEADER_BLOCKINFO_SIZE];
        for blockinfo in blocksinfo.as_chunks::<EXTENT_HEADER_BLOCKINFO_SIZE>().0 {
            let dev_id = blockinfo[3];
            if dev_id == 0 {
                continue;
            }

            let cluster_num = BigEndian::read_u32(&blockinfo[4..8]);
            let Some(dev) = self.dev_info.get_mut(dev_id as usize) else {
                anyhow::bail!(
                    "Cluster {} of extent at {} has out-of-range dev_id {}",
                    cluster_num,
                    data_pos,
                    dev_id
                );
            };
            let Some(dev) = dev else {
                anyhow::bail!(
                    "Cluster {} of extent at {} belongs to an unknown dev_id {}",
                    cluster_num,
                    data_pos,
                    dev_id
                );
            };

            let Some(cluster) = dev.clusters.get_mut(cluster_num as usize) else {
                anyhow::bail!(
                    "Unexpected cluster {} with dev_id {} found in extent at {}",
                    cluster_num,
                    dev_id,
                    data_pos
                );
            };
            anyhow::ensure!(
                cluster.is_empty(),
                "Duplicate cluster {} with dev_id {} found in extent at {}",
                cluster_num,
                dev_id,
                data_pos
            );

            let mask = BigEndian::read_u16(&blockinfo[0..2]);
            *cluster = VMAIndexDevCluster {
                mask,
                first_block_offset: last_block_offset,
            };

            let stored_blocks = mask.count_ones() as u16;
            dev.stored_blocks += stored_blocks as u64;
            found_blocks += stored_blocks;
            last_block_offset += BLOCK_SIZE as u64 * stored_blocks as u64;
        }

        let expected_block_count = BigEndian::read_u16(&bytes[6..8]);
        anyhow::ensure!(
            found_blocks == expected_block_count,
            "{} non-empty blocks found in blockinfo but the extent header at {} claims {}",
            found_blocks,
            data_pos,
            expected_block_count
        );
        Ok(last_block_offset - data_pos)
    }

    pub fn from_file(
        file: &mut File,
        progress_bar: bool,
        build_index: bool,
        ignore_limits: bool,
    ) -> anyhow::Result<Self> {
        let mut h = Self::default();
        file.read_exact(&mut h.bytes)?;
        let blob_buffer_offset = h.parse(ignore_limits)?;

        file.read_exact(&mut h.bytes[HEADER_SIZE..])?;
        h.verify_checksum()?;
        h.parse_devs(blob_buffer_offset, ignore_limits)?;

        if !build_index {
            return Ok(h);
        }
        log::info!("Building index...");
        h.allocate_clusters();
        let file_size = file.metadata()?.len();
        let progress_bar = get_progress_bar(file_size, progress_bar)?;

        let mut extent_header_bytes = [0u8; EXTENT_HEADER_SIZE];
        let mut file_pos = h.bytes.len() as u64;
        while file_pos != file_size {
            file.read_exact(&mut extent_header_bytes)?;
            file_pos += EXTENT_HEADER_SIZE as u64;
            let to_skip = h.index_extent_header(&mut extent_header_bytes, file_pos)?;

            file.seek_relative(to_skip as i64)?;
            file_pos += to_skip;
            progress_bar.set_position(file_pos);
        }

        h.verify_clusters()?;
        Ok(h)
    }

    pub fn from_sequential<R: Read>(
        reader: &mut SequentialReader<R>,
        progress_bar: bool,
        source_size: u64,
        build_index: bool,
        ignore_limits: bool,
    ) -> anyhow::Result<Self> {
        let mut h = Self {
            bytes: Vec::from(reader.read_exact(HEADER_SIZE)?.context("Input is too short")?),
            ..Default::default()
        };
        let blob_buffer_offset = h.parse(ignore_limits)?;

        let blob_size = h.bytes.len() - HEADER_SIZE;

        h.bytes[HEADER_SIZE..].copy_from_slice(reader.read_exact(blob_size)?.context("Input is too short")?);
        h.verify_checksum()?;
        h.parse_devs(blob_buffer_offset, ignore_limits)?;

        if !build_index {
            return Ok(h);
        }
        log::info!("Building index (sequential scan)...");
        h.allocate_clusters();
        let progress_bar = get_progress_bar(source_size, progress_bar)?;

        let mut source_pos = h.bytes.len() as u64;
        while !reader.empty() {
            let Some(extent_header_bytes) = reader.read_exact(EXTENT_HEADER_SIZE)? else {
                break;
            };
            source_pos += EXTENT_HEADER_SIZE as u64;
            let to_skip = h.index_extent_header(extent_header_bytes.try_into()?, source_pos)?;

            reader.discard(to_skip as usize)?;
            source_pos += to_skip;
            progress_bar.set_position(source_pos);
        }

        h.verify_clusters()?;
        Ok(h)
    }

    fn verify_checksum(&mut self) -> anyhow::Result<()> {
        let original_digest = md5::Digest(self.bytes[32..48].try_into()?);
        self.bytes[32..48].fill(0);
        let digest = md5::compute(&self.bytes);
        self.bytes[32..48].copy_from_slice(original_digest.as_ref());

        anyhow::ensure!(
            original_digest == digest,
            "Header checksum mismatch: expected {:x}, got {:x}",
            original_digest,
            digest
        );
        Ok(())
    }

    fn verify_clusters(&mut self) -> anyhow::Result<()> {
        for dev in self.dev_info.iter_mut().flatten() {
            let missing_clusters = dev.clusters.iter().filter(|c| c.is_empty()).count();
            if missing_clusters > 0 {
                anyhow::bail!("Device {} is missing {} clusters", dev.name, missing_clusters);
            }
        }
        Ok(())
    }

    pub fn rebuilt_size(&self) -> u64 {
        let mut total_size = self.bytes.len() as u64;
        for dev in self.dev_info.iter().flatten() {
            let extents = dev.clusters.len().div_ceil(EXTENT_HEADER_BLOCKINFO_LEN);
            total_size += extents as u64 * EXTENT_HEADER_SIZE as u64;
            total_size += dev.stored_blocks * BLOCK_SIZE as u64;
        }
        total_size
    }

    pub fn load_saved<R: Read>(&mut self, r: &mut R) -> anyhow::Result<()> {
        let saved: VMASavedIndex = decode_from_std_read(r, bincode::config::standard())?;
        anyhow::ensure!(
            saved.version == INDEX_VERSION,
            "Unsupported index version {}. Supported index versions: {}",
            saved.version,
            INDEX_VERSION
        );
        anyhow::ensure!(
            self.bytes == saved.bytes,
            "Index is corrupted or belongs to a different file"
        );
        self.dev_info = saved.dev_info;
        self.verify_clusters()
    }

    pub fn save<W: Write>(self, w: &mut W) -> anyhow::Result<()> {
        let saved = VMASavedIndex {
            version: INDEX_VERSION,
            dev_info: self.dev_info,
            bytes: self.bytes,
        };
        encode_into_std_write(&saved, w, bincode::config::standard())?;
        Ok(())
    }
}

pub fn write_sequential<W: Write>(
    input: &mut File,
    output: &mut W,
    index: &mut VMAIndex,
    rewrite_uuid: Option<&Uuid>,
    progress_bar: bool,
) -> anyhow::Result<()> {
    log::info!("Rebuilding in sequential order...");
    let mut bytes_written = 0u64;
    let new_size = index.rebuilt_size();
    let progress_bar = get_progress_bar(new_size, progress_bar)?;

    if let Some(rewrite_uuid) = rewrite_uuid {
        index.bytes[8..24].copy_from_slice(rewrite_uuid.as_bytes());

        index.bytes[32..48].fill(0);
        let digest = md5::compute(&index.bytes);
        index.bytes[32..48].copy_from_slice(digest.as_ref());

        log::info!("Rewrote UUID to {}, new digest is {:x}", rewrite_uuid, digest);
    }
    output.write_all(&index.bytes)?;
    bytes_written += index.bytes.len() as u64;

    let mut extent_buffer = vec![0u8; EXTENT_MAX_SIZE];
    for (dev_id, dev) in index.dev_info.iter().enumerate() {
        let Some(dev) = dev else { continue };
        let clusters = &dev.clusters;
        if clusters.is_empty() {
            continue;
        }

        for start in (0..clusters.len()).step_by(EXTENT_HEADER_BLOCKINFO_LEN) {
            let end = (start + EXTENT_HEADER_BLOCKINFO_LEN).min(clusters.len());
            let non_empty_blocks: u16 = clusters[start..end].iter().map(|c| c.mask.count_ones() as u16).sum();

            let mut w = &mut extent_buffer[..EXTENT_HEADER_SIZE];
            w.write_all(&EXTENT_HEADER_MAGIC)?;
            w.write_all(&[0u8; 2])?;
            w.write_all(&non_empty_blocks.to_be_bytes())?;
            w.write_all(&index.bytes[8..24])?;
            w.write_all(&[0u8; 16])?;
            for (i, c) in clusters.iter().enumerate().take(end).skip(start) {
                w.write_all(&c.mask.to_be_bytes())?;
                w.write_all(&[0u8])?;
                w.write_all(&[dev_id as u8])?;
                w.write_all(&(i as u32).to_be_bytes())?;
            }
            w.fill(0);

            let digest = md5::compute(&extent_buffer[..EXTENT_HEADER_SIZE]);
            extent_buffer[24..40].copy_from_slice(digest.as_ref());

            let mut offset = EXTENT_HEADER_SIZE;
            let mut stored_clusters = clusters[start..end].iter().filter(|c| c.mask != 0).peekable();
            while let Some(c) = stored_clusters.next() {
                let mut read_size = c.mask.count_ones() as usize * BLOCK_SIZE;
                while let Some(next) = stored_clusters.peek() {
                    if c.first_block_offset.checked_add(read_size as u64) != Some(next.first_block_offset) {
                        break;
                    }
                    read_size += next.mask.count_ones() as usize * BLOCK_SIZE;
                    stored_clusters.next();
                }

                input.read_exact_at(&mut extent_buffer[offset..offset + read_size], c.first_block_offset)?;
                offset += read_size;
            }

            assert_eq!(offset, EXTENT_HEADER_SIZE + non_empty_blocks as usize * BLOCK_SIZE);
            output.write_all(&extent_buffer[..offset])?;
            bytes_written += offset as u64;
            progress_bar.set_position(bytes_written);
        }
    }

    anyhow::ensure!(
        bytes_written == new_size,
        "Expected to write {}, but did {}. This is a bug or index is corrupted",
        new_size,
        bytes_written
    );
    Ok(())
}
