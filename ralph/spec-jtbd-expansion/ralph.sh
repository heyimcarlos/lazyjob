#!/bin/bash
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROMPT_FILE="$SCRIPT_DIR/prompt.md"
PROGRESS_FILE="$SCRIPT_DIR/progress.md"
MAX_ITERATIONS="${1:-20}"

if [ ! -f "$PROGRESS_FILE" ]; then
  printf "# Progress Log\nStarted: %s\n---\n" "$(date)" > "$PROGRESS_FILE"
fi

echo "Starting Ralph Loop — Spec JTBD Expansion"
echo "Max iterations: $MAX_ITERATIONS"
echo "Monitor: tail -f $PROGRESS_FILE"
echo ""

for i in $(seq 1 "$MAX_ITERATIONS"); do
  echo ""
  echo "========================================"
  echo "  Iteration $i of $MAX_ITERATIONS"
  echo "========================================"

  OUTPUT=$(claude --dangerously-skip-permissions \
    --print < "$PROMPT_FILE" 2>&1 | tee /dev/stderr) || true

  if echo "$OUTPUT" | grep -q "<promise>COMPLETE</promise>"; then
    echo ""
    echo "Ralph completed all tasks at iteration $i!"
    exit 0
  fi

  echo "Iteration $i complete. Continuing..."
  sleep 2
done

echo "Reached max iterations ($MAX_ITERATIONS) without completing."
echo "Check progress.md for status."
exit 1
