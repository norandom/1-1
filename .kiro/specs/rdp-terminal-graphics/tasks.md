# Terminal graphics defaults tasks

- [ ] 1. Measure the EGFX cache size
  - Add `ONE_ONE_EGFX_CACHE=large` to `CodecPolicy` with a test that the flag is the only difference. Measure as in the design on Windows and xrdp; record in `measurements.md`; decide with the maintainer.
  - _Requirements: 1.1, 1.2, 3.2_
- [ ] 2. Adopt the cache decision
  - Large adopted: remove `SMALL_CACHE` and the variable; tests pin the new advertisement. Otherwise remove the variable.
  - _Depends: 1_
  - _Requirements: 1.3, 3.1_
- [ ] 3. Check whether hypr_rdp negotiates EGFX
  - From the session log's channel observation. If it does, close requirement 2 as not applicable.
  - _Requirements: 2.1_
- [ ] 4. Advertise QOI and QOIZ for non-EGFX sessions
  - Enable the session features, list both codecs in `fastpath_bitmap()`, label them in the codec observation. Verify Windows and xrdp connect unchanged and hypr_rdp sends them.
  - _Depends: 3_
  - _Requirements: 2.1, 2.2, 3.1_
