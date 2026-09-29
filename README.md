# vma-seq-tool - Makes Proxmox backups deduplication-friendly

> ⚠️ **WARNING:** While this tool has been tested in typical scenarios, manipulating backup archives is inherently risky. You should **always** verify the output via `vma verify` and `vma extract`, and manually examine the extracted contents. You are entirely responsible for the integrity of your data.

## Why?

Proxmox VMA backups can store data out of order. This reduces deduplication efficiency in tools that use rolling hashes (e.g., restic, Borg, Kopia).

Extents and their clusters need not be sequential:
* **Before:** EXTENT (device 1 clusters 59-117) -> EXTENT (device 2 clusters 0-58) -> EXTENT (device 1 clusters 0-58)
* **After `vma-seq-tool rebuild`:** EXTENT (device 1 clusters 0-58) -> EXTENT (device 1 clusters 59-117) -> EXTENT (device 2 clusters 0-58)

Stable cluster ordering helps deduplication tools reuse unchanged data across backups. UUID replacement also reduces differences between extent headers.

## Installation

**Build from source:** Requires [Rust and Cargo](https://doc.rust-lang.org/cargo/getting-started/installation.html), Make, and sudo.
```shell
git clone https://github.com/allmazz/vma-seq-tool.git
cd vma-seq-tool
make install
```

## Usage

The tool operates in two passes:
1. **Index:** Scans the VMA file to map cluster locations.
2. **Rebuild:** Rebuilds the VMA sequentially using random I/O reads.

### Basic Rebuild
Perform both passes automatically:
```shell
vma-seq-tool rebuild my_backup.vma my_backup.seq.vma
```

### Two-Step (e.g., streaming from stdin)
Index building supports reading from `stdin`. Rebuilding requires a seekable input file.
```shell
# 1. Decompress and index simultaneously
zstd -dc my_backup.vma.zst | tee my_backup.vma | vma-seq-tool index - my_backup.vma.index

# 2. Rebuild using the generated index
vma-seq-tool rebuild -i my_backup.vma.index my_backup.vma my_backup.seq.vma
```
> **Warning:** Use an index built from the exact input file. Header matching is checked, but a stale or corrupted index will silently corrupt the output.

### Streaming Output
Stream directly to your backup tool using `-` as the output:
```shell
restic backup --stdin-from-command --stdin-filename /vzdump-qemu-100.vma -- vma-seq-tool rebuild my_backup.vma -
```

## Options & Optimizations

### UUID Replacement (`-u`, `--rewrite-uuid`)
Each VMA archive has a UUID stored in every extent header (~every 3 MiB). Reuse a stable UUID across backups to improve deduplication:
```shell
vma-seq-tool rebuild -u "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee" my_backup.vma my_backup.seq.vma
```

### HDD Optimization (`-s`, `--sequential-scan`)
By default, indexing seeks past data blocks. This is fast on SSDs but can be slower on HDDs. Place `-s` before the subcommand to read the entire input sequentially during indexing. Adjust the buffer with `--sequential-buffer-size` (bytes; default 8 MiB). Rebuilding still uses random reads.

## RAM Usage
The index uses about 16 bytes per 64 KiB cluster on 64-bit systems, including zero-filled clusters. Based on total device size:
* **100 GiB disk**: ~25 MiB RAM
* **1 TiB disk**: ~256 MiB RAM
* **10 TiB disk**: ~2.5 GiB RAM

Ensure your system has enough memory when processing large backups.

## Performance

Results for two backups with 10.93 GiB of changed device bytes:

| Setup                    | Additional Storage Used       | Storage Efficiency |
|--------------------------|-------------------------------|--------------------|
| Without the tool         | 44.02 GiB (47266365396 bytes) | 24.84%             |
| Without UUID replacement | 29.01 GiB (31152061523 bytes) | 37.69%             |
| With UUID replacement    | 11.00 GiB (11810707491 bytes) | 99.40%             |

Storage efficiency = changed device bytes / additional storage used × 100%.

[More about the test](bench/restic-storage-efficiency/README.md)

## Data Integrity
Verify the rebuilt archive and inspect the extracted contents:
```shell
vma verify -v my_backup.seq.vma
vma extract my_backup.seq.vma extracted-backup
```

## vmstate Support
Untested; may not work. Pull requests and sample files containing `vmstate` are welcome.

## References
* [vma wiki page](https://pve.proxmox.com/wiki/VMA)
* [vma sources](https://github.com/proxmox/pve-qemu/blob/master/debian/patches/pve/0024-PVE-Backup-add-vma-backup-format-code.patch)
* [format specs](https://github.com/proxmox/pve-qemu/blob/master/vma_spec.txt)
