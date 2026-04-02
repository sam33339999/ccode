#!/usr/bin/env bash
set -euo pipefail

cargo test -p ccode-bootstrap --test workspace_architecture
