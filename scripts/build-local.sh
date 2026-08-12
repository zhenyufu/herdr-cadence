#!/bin/sh
set -eu

cargo build --release --locked
mkdir -p bin
install -m 0755 target/release/herdr-cadence bin/herdr-cadence
herdr plugin link "$(pwd)"
printf 'Built and linked %s\n' "$(pwd)/bin/herdr-cadence"
