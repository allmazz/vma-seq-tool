#!/bin/bash
set -e

MNT=/mnt/sda

for i in 1 2; do
    dd if=/dev/urandom of="$MNT/20G-$i" \
        bs=1M count=5120 \
        seek=$((7680)) \
        conv=notrunc status=progress
done
for i in 1 2 3 4; do
    dd if=/dev/urandom of="$MNT/1G-$i" \
        bs=1M count=250 \
        seek=$((387)) \
        conv=notrunc status=progress
done

sync