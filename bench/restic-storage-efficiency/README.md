# Measuring incremental storage use in restic

## Description

For each case, the script:
* Initializes a restic repository.
* Backs up the first file and records the repository size.
* Backs up the second file and prints the size increase in bytes.
* Removes the repository.

## Results

### Using the [generated samples](#make-samples)

**Block device size: 50 GiB (53687091200 bytes)** \
**Changed device bytes (extracted images): 10.93 GiB (11739963811 bytes)** \
**VMA archive size before changes: 44.02 GiB (47263473664 bytes)** \
**VMA archive size after changes: 44.02 GiB (47263686656 bytes)**

| Case                                  | Additional Storage Used       | Storage Efficiency |
|---------------------------------------|-------------------------------|--------------------|
| Without vma-seq-tool                  | 44.02 GiB (47266365396 bytes) | 24.84%             |
| vma-seq-tool without UUID replacement | 29.01 GiB (31152061523 bytes) | 37.69%             |
| vma-seq-tool with UUID replacement    | 11.00 GiB (11810707491 bytes) | 99.40%             |

Sizes are measured with `du -sb` (apparent repository size, not allocated disk space or `restic stats`). Compression is disabled.

Storage efficiency = changed device bytes / additional storage used × 100%. This is not the percentage of data deduplicated.

## Reproduce

### Make samples

Skip this step if you are using existing backups.

This is a simple synthetic workload.

* Create a VM with one 50 GiB disk.
* Boot a Debian 13 Live CD.
* Run `sudo bash step1.sh` in the VM. It formats `/dev/sda` as ext4 and creates files; adjust `DEV` if needed.
* Back up the VM, decompress if needed, and save as `orig1.vma`.
* Run `sudo bash step2.sh` in the VM to modify the files.
* Back up the VM again, decompress if needed, and save as `orig2.vma`.

> I used backups of a running VM rather than `vma create`.

#### Sanity check: count differing bytes in extracted images
```shell
vma extract orig1.vma orig1
vma extract orig2.vma orig2
cmp -l orig1/disk-drive-scsi0.raw orig2/disk-drive-scsi0.raw | wc -l
```

### Run

```shell
make build
cd bench/restic-storage-efficiency
export VMA_SEQ_TOOL_BIN=../../target/release/vma-seq-tool
./bench.sh orig1.vma orig2.vma
```

You can change the path to the vma-seq-tool binary with the `VMA_SEQ_TOOL_BIN` variable:

```shell
export VMA_SEQ_TOOL_BIN=../../target/release/vma-seq-tool
```
