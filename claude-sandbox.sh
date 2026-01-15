#!/bin/bash

# Usage: claude-sandbox /path/to/project [additional paths...]

if [ $# -eq 0 ]; then
    echo "Usage: claude-sandbox <folder> [additional folders...]"
    exit 1
fi

# Build volume mount arguments
VOLUMES=""
for path in "$@"; do
    abs_path=$(realpath "$path")
    folder_name=$(basename "$abs_path")
    VOLUMES="$VOLUMES -v $abs_path:/workspace/$folder_name"
done

# Run Claude Code in container
docker run -it --rm \
    $VOLUMES \
    -e ANTHROPIC_API_KEY \
    claude-code-sandbox
