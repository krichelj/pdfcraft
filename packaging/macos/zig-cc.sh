#!/usr/bin/env bash
# zig wrapper for the macOS cross-build; cross-package.sh points CARGO_ZIGBUILD_ZIG_PATH here.
# - Drop `-Wl,-exported_symbols_list -Wl,<file>`: zig 0.17's Mach-O linker misreads it when rustc
#   links the dylib crate-type that rten and boa_engine declare (PdfCraft never loads those).
# - Keep only the first of each `-framework X` and system dylib `-lX`: rustc repeats them (one per
#   crate that links AppKit), Apple's ld deduplicates, zig does not, and dyld then refuses to load
#   the app ("duplicate linked dylib .../AppKit", 2026-10-08).
args=()
seen=" "
skip=0
prev=""
for a in "$@"; do
  if [ "$skip" = 1 ]; then skip=0; continue; fi
  if [ "$a" = "-Wl,-exported_symbols_list" ]; then skip=1; continue; fi
  if [ "$prev" = "-framework" ]; then
    prev=""
    case "$seen" in *" fw:$a "*) unset 'args[${#args[@]}-1]'; continue ;; esac
    seen="$seen fw:$a "
    args+=("$a")
    continue
  fi
  case "$a" in
    -lSystem | -lc | -lm | -liconv | -lobjc | -lc++ | -lresolv | -lcurses | -lz)
      case "$seen" in *" $a "*) continue ;; esac
      seen="$seen $a "
      ;;
  esac
  prev="$a"
  args+=("$a")
done
exec "${PDFCRAFT_REAL_ZIG:?}" "${args[@]}"
