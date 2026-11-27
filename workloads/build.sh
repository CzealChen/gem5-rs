#!/usr/bin/env bash
set -euo pipefail

src=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
out=${1:-"$src/build"}
cc=${CC_RV64:-riscv64-none-elf-gcc}
mkdir -p -- "$out"

"$cc" --version | sed -n '1p'
flags=(-march=rv64i -mabi=lp64 -mcmodel=medany -mno-relax -msmall-data-limit=0
       -O1 -ffreestanding -fno-lto -fno-pic -fno-pie -fno-stack-protector)
link=(-static -nostdlib -nostartfiles -nodefaultlibs -no-pie
      -Wl,--no-relax,--build-id=none -T "$src/link.ld")

for file in start.S rv64i.S loop.c term.c; do
    "$cc" "${flags[@]}" -c "$src/$file" -o "$out/${file%.*}.o"
done
"$cc" "${flags[@]}" "${link[@]}" "$out/start.o" "$out/rv64i.o" -o "$out/rv64i.elf"
"$cc" "${flags[@]}" "${link[@]}" "$out/start.o" "$out/loop.o" "$out/term.o" -o "$out/loop.elf"
printf 'Built %s\n' "$out/rv64i.elf" "$out/loop.elf"
