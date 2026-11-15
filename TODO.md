# Maintenance TODO

Items are ordered by security and correctness impact. Complete each item with a focused conventional commit before moving to the next.

## 1. Update vulnerable dependencies

- [x] Upgrade `zip` to a supported 9.x release and update `rustix` to at least 1.1.6.
- [x] Review dependency audit results and preserve the Rust 1.88 MSRV.
- [x] Verify ZIP/GZIP loading and package/build checks. (Build and Clippy passed; dependency audit reported no advisories.)

## 2. Keep valid TCP messages intact during recovery

- [x] Do not resynchronize to a storage header embedded in a valid incomplete raw frame.
- [x] Cover fragmented frames containing storage-framed payload bytes.

## 3. Preserve the selected log while loading

- [x] Track selection by log identity when timestamp merges insert rows before it.
- [x] Preserve current tail-follow behavior (the existing auto-scroll override remains unchanged).

## 4. Measure and improve large multi-file merges

- [ ] Benchmark overlapping timestamp ranges across large files.
- [ ] Reduce repeated copying while preserving stable chronological order.

## 5. Sanitize decoded verbose text

- [ ] Apply the same display sanitization to decoded strings as raw payload text.
- [ ] Cover terminal control characters in verbose values.

## 6. Make filter configuration writes durable

- [ ] Replace direct truncating writes with a same-directory temporary file and atomic replacement.
- [ ] Report malformed or unreadable configuration and bound actual bytes read.

## 7. Reuse compiled text filters

- [ ] Recompile only when the active text filter changes.
- [ ] Make invalid-regex behavior clear and consistent.

## 8. Keep the interface responsive during full scans and exports

- [ ] Move long full-log filtering and export work off the UI event loop or process it incrementally.
- [ ] Buffer export writes.

## 9. Make Source and CTX navigation scale

- [ ] Allow navigating all sidebar entries, not only the visible top counts.
- [ ] Distinguish files that share a basename.

## 10. Scroll large log details

- [ ] Add detail-pane scrolling and generate only the visible hex dump rows.

## 11. Distinguish timestamp domains

- [ ] Preserve whether timestamps are storage absolute time or daemon-relative ticks.
- [ ] Define ordering and display behavior for mixed domains, restarts, and wraparound.

## 12. Avoid misreading unsupported verbose argument types

- [ ] Validate TYLE and supported widths before consuming argument bytes.
- [ ] Stop decoding or label unsupported types without shifting subsequent arguments.
