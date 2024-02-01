#!/bin/sh
set -e

cd "$(dirname "${0}")/.."
cargo fmt --all
cargo clippy --target x86_64-unknown-linux-gnu --fix --allow-dirty --allow-staged
