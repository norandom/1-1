# Task 4.1 compile probe — IronRDP EGFX-generation resolution (windows-clearcodec-royalts-e2e)

Standalone, **pinned** compile/behavior probe for the IronRDP component set that tasks
4.2/4.3 will adopt. It exists to prove the upstream API surface required by
Requirements 9.1/9.2/9.3/9.5/9.6/9.7 *before* the main manifest (`src-tauri/Cargo.toml`)
is touched. Evidence and the resolved-set record live in
`.kiro/specs/windows-clearcodec-royalts-e2e/research.md` (task 4.1 section).

## Not part of any CI tier

Every tier command pins an explicit manifest (`--manifest-path src-tauri/Cargo.toml`
in `scripts/Invoke-CI.ps1` and `.dagger/src/index.ts`), so this sibling package is
invisible to `Invoke-CI -Tier rust`, the Dagger `rust-checks` entry, and `cargo fmt`
tier runs. It is intentionally a separate package (not a src-tauri example) so the
main crate's dependency set stays untouched here.

## Run

```pwsh
cargo run --locked --manifest-path probes/ironrdp-egfx-05/Cargo.toml
```

The binary performs numbered assertion checks (C1..C8) and exits non-zero on any
failure. No network access, no server: EGFX traffic is synthesized locally by
encoding `GfxPdu`s and wrapping them with the public upstream zgfx API
(`ironrdp_graphics::zgfx::compress_and_wrap_egfx`), mirroring `ironrdp-egfx`'s own
server-side framing.

## OpenH264 absence audit (Req 9.5)

```pwsh
cargo tree --manifest-path probes/ironrdp-egfx-05/Cargo.toml -i openh264
```

must fail with "did not match any packages" — proving the resolved feature graph
carries no H.264 dependency.
