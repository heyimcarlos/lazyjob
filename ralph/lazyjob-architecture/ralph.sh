#!/bin/bash
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROMPT_FILE="$SCRIPT_DIR/prompt.md"
PROGRESS_FILE="$SCRIPT_DIR/progress.md"
TASKS_FILE="$SCRIPT_DIR/tasks.json"
MAX_ITERATIONS="${1:-30}"

# Ensure we're in the script directory
cd "$SCRIPT_DIR"

if [ ! -f "$PROGRESS_FILE" ]; then
  printf "# Progress Log\nStarted: %s\n---\n" "$(date)" > "$PROGRESS_FILE"
fi

echo "Starting Ralph Loop (LazyJob Architecture Research) — Max iterations: $MAX_ITERATIONS"

for i in $(seq 1 "$MAX_ITERATIONS"); do
  echo ""
  echo "========================================"
  echo "  Iteration $i of $MAX_ITERATIONS"
  echo "========================================"

  # Capture output to file AND show to user via stderr
  OUTPUT_FILE="$SCRIPT_DIR/.ralph-output-$i.txt"

  claude --dangerously-skip-permissions \
    --print < "$PROMPT_FILE" 2>&1 \
    | tee "$OUTPUT_FILE" || true

  # Only exit if <RALPH_ALL_DONE/> appears (final completion only)
  if grep -q "<RALPH_ALL_DONE/>" "$OUTPUT_FILE" 2>/dev/null; then
    echo ""
    echo "Ralph completed all tasks at iteration $i!"
    rm -f "$SCRIPT_DIR/.ralph-output-"*.txt
    exit 0
  fi

  # Verify spec files exist for completed tasks
  echo ""
  echo "Verifying spec outputs..."
  SPEC_DIR="$SCRIPT_DIR/../../specs"
  if [ -d "$SPEC_DIR" ]; then
    SPEC_COUNT=$(find "$SPEC_DIR" -name "*.md" | wc -l | tr -d ' ')
    echo "  Specs found: $SPEC_COUNT"
  fi

  echo "Iteration $i complete. Continuing..."
  sleep 2
done

echo ""
echo "Reached max iterations ($MAX_ITERATIONS) without completing."
echo "Check progress.md for status."
echo "Output logs saved to $SCRIPT_DIR/.ralph-output-*.txt"
exit 1
