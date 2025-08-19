#!/bin/bash

EXCLUDES=(
    "target"
    ".git"
    "*.log"
    "__pycache__"
    ".idea"
    "deploy.sh"
)

RSYNC_ARGS=()
for e in "${EXCLUDES[@]}"; do
    RSYNC_ARGS+=(--exclude "$e")
done

rsync -avz "${RSYNC_ARGS[@]}" ./ {{ip}}:~/red_dev/

echo "Done"
