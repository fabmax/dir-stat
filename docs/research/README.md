# dir-stat research

Background research for dir-stat, done before the design stage. The reports were written in `/tmp/dirstat-work/` and copied here. Paths inside them still point to the old location:
`/tmp/dirstat-work/research/` is now `docs/research/`, and `/tmp/dirstat-work/probes/` is now `docs/research/probes/`.

| Report | Contents |
| --- | --- |
| [iced-api.md](iced-api.md) | iced 0.14 API reference, checked by compiling it: app setup, `Task`/`Subscription`, widgets, canvas, theming, rfd/opener/trash, manual list virtualization, font issues. |
| [scan-engine.md](scan-engine.md) | Our own walker vs `du` vs walker crates, with benchmarks. Recommended scanner design: work-stealing threads, per-folder events, batching, cancellation. |
| [fs-semantics.md](fs-semantics.md) | Filesystem pitfalls: size on disk vs file size, hard links, APFS System/Data volumes sharing one device id, firmlinks, macOS privacy protection, iCloud, trash safety. Includes the traversal rules in pseudo-code. |
| [ux-spec.md](ux-spec.md) | UX spec with wireframes, a MUST/SHOULD/LATER tag on every feature, the visual design system and the data the UI needs (section 16). |
| [cleanup-catalog.md](cleanup-catalog.md) | 178 cleanup rules in 14 groups, each with a risk level (Safe/Low/Review/protect), a short reason and a cleanup command, plus how folders are matched and the safety guards. The Rust prototype it was generated from is in `cleanup-catalog-proto/`. |

`probes/` and `cleanup-catalog-proto/` hold the throwaway crates used to check the findings. Build folders (`target/`) and the private cargo cache are left out; rebuild with `cargo build`.
- `iced-probe`: compiling examples of every iced API in the reference, plus compile-fail tests and screenshots.
- `scan-bench`: reference scanner implementation and benchmark results.
- `fs-probe`: filesystem experiments, with raw output in `evidence/`.

The 55 MB of third-party source used as reading material (iced, dua, gdu, ncdu, xnu, …) was not copied. It can be downloaded again.
