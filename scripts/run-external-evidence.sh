#!/bin/sh
set -eu

script_directory=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repository_root=$(CDPATH= cd -- "$script_directory/.." && pwd)

cd "$repository_root"
exec cargo run -p milkdrift-daemon --bin milkdrift-external-evidence -- "$@"
