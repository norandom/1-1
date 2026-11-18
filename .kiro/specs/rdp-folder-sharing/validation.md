# Windows client validation

## Desktop connections after the shared TLS change

On 18 November 2026 the Windows x64 client at `ccf31e3` ran the existing three-host smoke test through the production desktop connector. Each host delivered a desktop frame, shut down cleanly and accepted a fresh connection.

| Staged target | Negotiated route | Connect | First frame | Remote shrink and restore |
| --- | --- | ---: | ---: | --- |
| Windows workstation | TLS 1.3 with CredSSP | 481 ms | 1,336 ms | Incomplete |
| FreeRDP-based Linux host | TLS 1.3 with basic settings | 340 ms | 425 ms | Incomplete |
| IronRDP-based Linux host | TLS 1.3 with CredSSP | 371 ms | 1,786 ms | Both confirmed |

The measurements are from one LAN run. The Windows and FreeRDP results leave the native smart-resize checks open. Separate offline TLS fixtures cover the strict certificate and pin checks. Folder announcement, shell copies and transfer measurements will need evidence from the implemented share.

The command was `pwsh -NoProfile -File scripts/Invoke-Baseline.ps1 -Mode Smoke -OutDir D:/BuildArtifacts/drift-rdp/session-actions-smoke-20261118`. The workstation report is `smoke-20261118-094026/smoke-report.json` under that directory.

## Local Windows drive fixture

The Windows x64 test ran one selected, read-only drive through the pinned IronRDP 0.9.0 native backend. It verified short reads, an empty file and different byte patterns at offsets with the same low 32 bits on either side of 4 GiB in a sparse NTFS file. A denied write left the original file intact. The native folder-picker check created the Windows dialog with directory-selection options without opening a window.

The native CI tier passed 376 tests with five skipped. The doctest runner found zero cases. Both the CI run with a custom project artifact directory and standalone Cargo with a custom `TEMP` cleaned up their disposable fixture files. This milestone covers local file I/O. Granting a folder to a saved host and observing a remote drive or mount remain open in the [task plan](tasks.md).
