# Orlo retention extension

This directory contains the published vt100 0.16.2 source already pinned by Orlo, under its
original MIT license. `UPSTREAM.json` records the original crate checksum and exact hashes of
all copied published files. No parser, rendering, process or dependency version is replaced.
The package is excluded from Orlo's ten-crate workspace and used as a normal path dependency.
The published package contains no external test files; its original library doctest remains.
Orlo's terminal regressions exercise the extension through the actual dependency.

The only upstream Rust changes are `Screen::set_scrollback_len` and its internal Grid method.
They set the primary grid's cap, drop oldest surplus rows, release excess history allocation and
clamp its viewport offset. Alternate-screen capacity stays zero, even when the active screen is
alternate. Cursor/saved state, attributes, modes and parser/Unicode decoder state are untouched.
Growing the cap cannot restore discarded rows. Orlo calls it for both text and provenance grids.

This local dependency source must be compared with upstream when upgrading vt100; retain or
replace the two methods and rerun the native retention, paired provenance, stale snapshot and
full terminal/UI regressions. It exists because 0.16.2 exposes only a scrollback viewport setter,
not a retention setter; CSI 3 J is unhandled. Resetting or replaying terminal state is unsuitable
for changing the live retention limit.
