#!/bin/bash
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROMPT_FILE="$SCRIPT_DIR/prompt.md"
PROGRESS_FILE="$SCRIPT_DIR/progress.md"
MAX_ITERATIONS="${1:-80}"

if [ ! -f "$PROGRESS_FILE" ]; then
  printf "# Progress Log\nStarted: %s\n---\n" "$(date)" > "$PROGRESS_FILE"
fi

echo "Starting Ralph Loop — Max iterations: $MAX_ITERATIONS"
echo "Prompt: $PROMPT_FILE"
echo "Progress: $PROGRESS_FILE"
echo ""

for i in $(seq 1 "$MAX_ITERATIONS"); do
  echo ""
  echo "========================================"
  echo "  Iteration $i of $MAX_ITERATIONS"
  echo "  $(date)"
  echo "========================================"

  OUTPUT=$(claude --dangerously-skip-permissions \
    --print < "$PROMPT_FILE" 2>&1 | tee /dev/stderr) || true

  if echo "$OUTPUT" | grep -q "<promise>COMPLETE</promise>"; then
    echo ""
    echo "Ralph completed all tasks at iteration $i!"
    exit 0
  fi

  echo "Iteration $i complete. Continuing..."
  sleep 3
done

echo ""
echo "Reached max iterations ($MAX_ITERATIONS) without completing."
echo "Check progress.md for status."
exit 1
