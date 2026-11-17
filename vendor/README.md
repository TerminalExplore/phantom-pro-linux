# vendor/

`xattr/` is `xattr 0.2.3` from crates.io with one change: `libc` no longer exports `ENOATTR` on Linux, so
`src/sys/mod.rs` uses `ENODATA` there (the same value). It is pulled in by GPUI (`gpui` → `zed-async-tar` →
`xattr`) and is wired in through `[patch.crates-io]` in the top-level `Cargo.toml`. The crate keeps its own
licenses (`MIT OR Apache-2.0`, see `xattr/LICENSE-*`). Drop this directory once upstream fixes it.
