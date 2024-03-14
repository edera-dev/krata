#!/bin/sh
set -e

REAL_SCRIPT="$(realpath "${0}")"
cd "$(dirname "${REAL_SCRIPT}")/../.."

./hack/build/cargo.sh clippy --fix --allow-dirty --allow-staged
./hack/build/cargo.sh fmt --all
