#!/bin/bash
set -e

export RESTIC_REPOSITORY=./bench
export RESTIC_PASSWORD=bench
export RESTIC_PACK_SIZE=128
export RESTIC_COMPRESSION=off

test_case() {
  local case_name="$1"
  local cmd1="$2"
  local cmd2="$3"
  restic init &> /dev/null

  restic backup --stdin-from-command -- $cmd1 &> /dev/null
  before=$(du -sb "$RESTIC_REPOSITORY" | cut -f1)

  restic backup --stdin-from-command -- $cmd2 &> /dev/null
  after=$(du -sb "$RESTIC_REPOSITORY" | cut -f1)

  rm -rf "$RESTIC_REPOSITORY"
  echo "$case_name: $((after-before)) bytes"
}

VMA_SEQ_TOOL_BIN="${VMA_SEQ_TOOL_BIN:-$(command -v vma-seq-tool)}"
echo "vma-seq-tool at $VMA_SEQ_TOOL_BIN"

test_case "without vma-seq-tool" \
  "cat $1" \
  "cat $2"
test_case "vma-seq-tool without UUID replacement" \
  "$VMA_SEQ_TOOL_BIN rebuild $1 -" \
  "$VMA_SEQ_TOOL_BIN rebuild $2 -"
test_case "vma-seq-tool with UUID replacement" \
  "$VMA_SEQ_TOOL_BIN rebuild -u aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee $1 -" \
  "$VMA_SEQ_TOOL_BIN rebuild -u aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee $2 -"
