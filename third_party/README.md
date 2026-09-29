# Patched dependencies

## glib 0.18.5

`glib/` is glib 0.18.5 exactly as published on crates.io (SHA-256
`233daaf6e83ae6a12a52055f568f9d7cf4671dabb78ff9560ab6da230ce00ee5`) with one change: the fix for
[GHSA-wrw7-89jp-8q8g](https://github.com/advisories/GHSA-wrw7-89jp-8q8g) (RUSTSEC-2024-0429),
backported from gtk-rs-core commit
[`b5a4071`](https://github.com/gtk-rs/gtk-rs-core/commit/b5a4071e439bef2b5eea76c3aa25e5ae84839e34)
([gtk-rs-core#1343](https://github.com/gtk-rs/gtk-rs-core/pull/1343)). Its `Cargo.toml` also
allows compiler warnings, which Cargo hid for the crates.io copy and would otherwise print 34 times
per Linux build.

`VariantStrIter::impl_get` passed `&p` to `g_variant_get_child`, which writes a string pointer
through it. With optimizations the compiler keeps `p` null, so iterating the strings of an `as`
`Variant` dereferences a null pointer. The fix passes `&mut p`. glib 0.20 ships it; the 0.18 branch
never had a release with it.

Only the Linux build compiles glib. Tauri 2's Linux crates (tao, wry, muda, tray-icon, webkit2gtk,
libappindicator) require gtk 0.18, and gtk 0.18 requires glib 0.18. The root `Cargo.toml` points
`[patch.crates-io]` at this folder, so Cargo builds this copy instead of the crates.io one.

`scripts/dependency-patches.test.mjs` fails when a crates.io glib older than 0.20 returns, when this
copy loses the fix, or when nothing needs this copy any more.

**Removing it:** once those crates depend on gtk 0.19 or later (glib 0.22), delete this folder and
the `[patch.crates-io]` entry, then run `cargo update -p glib`.
