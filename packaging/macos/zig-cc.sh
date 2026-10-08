#!/usr/bin/env bash
# zig 0.17's Mach-O linker misreads `-Wl,-exported_symbols_list -Wl,<file>`, which rustc passes
# when linking the dylib crate-type that rten and boa_engine declare (PdfCraft never loads those
# dylibs). Drop that pair and run the real zig; cross-package.sh points CARGO_ZIGBUILD_ZIG_PATH here.
args=()
skip=0
for a in "$@"; do
  if [ "$skip" = 1 ]; then skip=0; continue; fi
  if [ "$a" = "-Wl,-exported_symbols_list" ]; then skip=1; continue; fi
  args+=("$a")
done
exec "${PDFCRAFT_REAL_ZIG:?}" "${args[@]}"
