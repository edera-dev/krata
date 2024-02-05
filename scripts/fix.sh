#!/bin/sh
set -e

cd "$(dirname "${0}")/.."
cargo clippy --target x86_64-unknown-linux-gnu --fix --allow-dirty --allow-staged
cargo fmt --all
