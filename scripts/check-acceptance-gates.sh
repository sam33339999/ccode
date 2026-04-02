#!/usr/bin/env bash
set -euo pipefail

cargo test -p ccode-bootstrap --test workspace_architecture
cargo test -p ccode-bootstrap --test spec_governance_gate
cargo test -p ccode-bootstrap --test acceptance_parity_gate
