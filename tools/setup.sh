#!/bin/sh
# One-time setup for a fresh clone: use the repo's git hooks (.githooks/).
# Run from the repo root: sh tools/setup.sh
set -e
git config core.hooksPath .githooks
echo "git hooks enabled: pre-push runs cargo fmt --check and cargo clippy -D warnings"
