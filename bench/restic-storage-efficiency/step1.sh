#!/bin/bash
set -e

DEV=/dev/sda
MNT=/mnt/sda

mkfs.ext4 -F "$DEV"
mkdir -p "$MNT"
mount "$DEV" "$MNT"

for i in 1 2; do
    dd if=/dev/urandom of="$MNT/20G-$i" bs=1M count=20480 status=progress
done
for i in 1 2 3 4; do
    dd if=/dev/urandom of="$MNT/1G-$i" bs=1M count=1024 status=progress
done

sync