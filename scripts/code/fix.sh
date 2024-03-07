#!/bin/sh
set -e

cd "$(dirname "${0}")/../.."
./scripts/build/cargo.sh clippy --fix --allow-dirty --allow-staged
cargo fmt --all
