# Auto-digitizing validation

The additional completion pass after the UI redesign is documented in
[algorithm-completion.md](algorithm-completion.md). Its final binaries,
layering/travel/smoothing comparisons and private-image checks supersede the
build hashes in this historical first-pass report.

This report records this working-session's checks; historical results from
`publication/EmbroideryLab/VALIDATION.md` are not counted as new test runs.
No physical sew-out has been performed.

## Baseline and environment

- Research working tree HEAD: `99340b2bf0b5c57ce4bc06090e382def1cb39d55`.
  It contains later, uncommitted image import, PDF, 3D and editing work. The
  request's public `946b186...` snapshot was not restored over those changes.
- A copy of the original current `src`, Cargo files, tests and helper scripts
  is in `outputs/autodigitize/baseline-source`; git status and binary patch
  were recorded before implementation. Existing optimized CLI/GUI executables
  were preserved in `outputs/autodigitize/baseline-bin`.
- An exact snapshot-built **debug** CLI is preserved as
  `outputs/autodigitize/baseline-bin/embroidery-cli-source-debug.exe` and is the
  comparison baseline. Candidate debug CLI is used for the same build profile.
  A redundant optimized default-feature snapshot build was stopped when it
  began recompiling GUI dependencies; its incomplete log is not a success.
- Rust/Cargo 1.99.1, Windows GNU toolchain, local MinGW-w64; OpenGL remains
  the application's existing renderer. PDFium preparation uses the existing
  pinned build script.
- Independent reader: pyembroidery **1.5.1**. Independent raster renderer:
  Pillow **13.0.0**, Python **3.12.14**. The installed Pillow extension is cp312;
  merely importing the package metadata in another Python is not sufficient.
- The VM exposes **one logical processor** (Ryzen 7 7800X3D host descriptor).
  First-run timings include process start and concurrent compiler load. They
  are recorded for reproducibility, not presented as a performance speedup.

## Reproducible corpus

Run from the repository root after building the candidate CLI:

```powershell
python tools/autodigitize_validation.py `
  --baseline outputs/autodigitize/baseline-bin/embroidery-cli-source-debug.exe `
  --candidate embroidery-engine/target/debug/embroidery-cli.exe
```

Install the exact Pillow/pyembroidery versions into the Python environment
used for the command. In this environment, the bundled Python 3.12 executable
can use `outputs/releases/v0.6.0/smoke/python-deps` as `PYTHONPATH`.

The script generates its own inputs and records command stdout/stderr, input
options, physical dimensions, elapsed time, ELAB, DST, planned commands,
quality JSON, independent 2D PNGs, an HTML comparison and machine-readable
summaries under `outputs/autodigitize/comparison/`. It reads no private artwork.
The original images and exterior-mask illustrations appear alongside vector
regions and real decoded machine trajectories. The mask illustrations are
independent pictures of the fixed input background decision, not application
screenshots or claims about an interactive gesture.

The corpus includes compensated/uncompensated rectangles, protected holes,
separate components, two holes and an island, C shape, a 0.12 mm detail,
identical geometry with asymmetrically resampled collinear vertices,
independent underlay parameters, layered red/blue/red shapes, an acute narrow
column and Y branch. Raster cases include a flag with exterior and necessary
interior white, two source resolutions at equal millimetres, two physical
sizes, antialiasing, JPEG noise, transparent random hidden RGB, O/B/8 topology,
three-color T boundaries and a 2×2 diagonal contact.

DST is read by **PyEmbroidery**, separately from the application's decoder.
Checks compare quantized needle coordinates, color controls, end record,
final position, Euclidean stitch length and stable ELAB regeneration. Hole
checks sample stitched segments every **0.05 mm** and ignore only a **0.075 mm**
boundary band: quantizing a point to a 0.1 mm grid can displace it by at most
`sqrt(2) × 0.05 = 0.070711 mm`. This is a bounded diagnostic, not an exact
containment proof. Orange travel metadata is displayed separately where
available; underlay does not count as top-fill coverage.

Color controls are checked at their exact needle-order and position anchors,
not only by count; normalized controls must be stationary. Each needle segment
is checked against its object's pass-specific length and machine cap. Legacy
plans cannot identify underlay roles, so their conservative length ambiguity
is explicitly listed. For DST `ThreeJump`, a second independent PyEmbroidery
read disables heuristic clipping and verifies three zero-displacement Jump
records at each intended Trim anchor. DSB checks actual Trim commands.
This verifies the selected encoding, not universal machine trim behavior.

Table jump counts are **reader-normalized records**, after PyEmbroidery's
trim-marker clipping; `planned_jump_count`, raw DST record counts, zero-Jump
trim triples and inferred Trim counts are separate JSON fields. Zero-length
trim markers do not increase physical jump distance.

The independent coverage estimate uses **0.30 mm assumed effective thread
width** and **0.23 mm sample pitch**. Pitch deliberately differs from the
0.40 mm Tatami row spacing to avoid a sampling grid aligned to every row.
It measures geometric thread buffers, not actual fabric coverage, and is
not an acceptance threshold. Legacy command files lack role annotations and
are explicitly left unscored rather than letting underlay, sewn travel or
locks hide missing top fill. In-app diagnostics can use different displayed
assumptions; compare numerical coverage only with matching parameters.

## Reproduced baseline defects

The first baseline pass independently decoded all 24 files and matched
machine needle positions to the saved command JSON. That alone is not an
algorithm-quality acceptance result. Behavioral checks were subsequently
added for the failures below.

The final snapshot-built debug baseline reproduces **all 24 DST files
byte-for-byte** from the preserved prior optimized binary. Its final corpus
has **18 PASS / 6 REVIEW**: the six reviews are the compensated rectangle,
thin detail and two members each of the angle/underlay pairwise comparisons.
All **24 DST and four selected DSB** files pass independent format, needle
coordinate, control, end-position, physical-length and sampled-hole checks.
The remaining reviews are real algorithm defects, not codec failures.
Source/binary hashes and the 24-file match are recorded in
`outputs/autodigitize/baseline-provenance.json`.

| Identical settings except stated change | Baseline observation |
| --- | --- |
| Rectangle 30×20 mm, spacing 0.4, length 3.5; compensation 0 → 0.2 mm/side | Jump records **1 → 50**; jump distance **7.211 → 26.677 mm**, including initial move |
| Same rectangle with protected interior hole | Jump records **3 → 51** |
| Two independent 10×20 mm islands in one object | Jump records **5 → 54** |
| Tatami underlay inset/spacing/length 0.3/2.0/3.0 → 1.3/3.0/1.5 mm | Identical **597 stitches, 12 jumps**: user controls had no effect |
| 10×0.12 mm detail | **0 stitches**, with successful export: silent loss |

The harness marks retained defects as `REVIEW`; it does not weaken acceptance
to make a baseline pass. Candidate results and current Rust checks are added
below only after their commands have completed.

Final gap audit before the guard reproduced another real defect on the then-current
candidate: two 10×10 mm rectangles at x=0 and x=10.1, pull compensation 0.2 mm/side,
generated successfully with no warnings. PyEmbroidery independently found **50
needle segments crossing the original gap**; the two compensated domains overlap
by 0.3 mm. Preserved evidence is in
`outputs/autodigitize/review/narrow-gap-analysis.json`, alongside its source ELAB,
plan, DST and command result. The new Rust regression
`compensation_rejects_new_contacts_across_original_component_gaps` first failed
against that behavior (0 passed, 1 failed, exit 101), before the guard was applied.

The guard rejects a new contact or overlap across an originally open gap, without
clipping spans or editing source geometry. It checks same-object components and
separate visible Tatami/Satin objects, including one-sided compensation in either
object order and islands within holes. Existing shared contacts and intentional
layer overlaps retain the per-object compensation policy; they are not inferred
away. Safe 1 mm gaps, zero compensation and compensation perpendicular to a gap
remain accepted. Bounded pair and exact-node work limits fail actionably instead
of skipping unchecked intersections; all-zero compensation skips the guard.
The preserved final-engine debug CLI independently rejects that exact prior
ELAB with exit 1 and an actionable original-gap message; the command is saved
as `outputs/autodigitize/review/guard-rejection-command.json`.

## Final candidate CLI comparison

The immutable debug CLI in `outputs/autodigitize/candidate-final-engine-bin`
has SHA-256 `18b66cbe7d804a85210ee9becf8ea0c9e27c770c6ae4e465348d3667e631601d`.
It includes bounded omission diagnostics, the compensation-gap guard,
independent underlay direction and the fresh-raster color-order correction.
The final controlled-palette corpus is **24/24 PASS**, compared with the exact
snapshot baseline's **18/24**. All **24 DST and four selected DSB** outputs
pass independent format, coordinates, controls, physical-length and sampled
geometry checks; saved ELAB regeneration is byte-identical.

| Fixed case | Baseline | Final candidate |
| --- | --- | --- |
| Compensated 30×20 mm rectangle | 50 jumps / 26.677 mm | 1 jump / 7.077 mm; 513 top stitches + 49 sewn travel stitches |
| Compensated ring | 51 jumps / 48.277 mm | 3 reader jumps / 29.077 mm |
| Compensated separate islands | 54 jumps / 73.281 mm | 3 reader jumps / 35.080 mm |
| 0.12 mm detail | 0 stitches | 4 top stitches |
| Uneven collinear resampling | Different trajectories | Identical trajectories, 367 needle stitches each |
| Independent underlay settings | Both 597 stitches | Underlay 84 → 72 stitches; top fill remains 432 in both |
| Synthetic flag, 128 px | 2,990 stitches / 72 reader jumps / 778.511 mm jump | 3,088 / 70 / 730.908 mm |
| JPEG flag | 3,390 stitches / 95 reader jumps / 994.075 mm jump | 3,374 / 87 / 859.382 mm |

The required real-image comparison first exposed a separate sequencing defect:
the same eight fixed RGB colors produced **690 color blocks / 689 changes**,
compared with eight blocks / seven changes in the baseline. The added
raster-specific block check also marked six synthetic flag variants REVIEW.
Before-fix summaries and private artifacts are preserved. The final candidate
passes that check for all raster cases and the local real artwork.
Manual vector red/blue/red layer order remains unchanged.

Fresh-import batching computes a stable topological order: positive filled-area
overlaps retain their original order; shared edges and hole-contained islands
are free to batch by RGB. Every exact relation remains checked. Work estimates
are capped at two million pair checks and **500 million node products**, with
cancellation polling and atomic fallback to the complete original order. The
initial 50-million estimate caused the real raster to fall back explicitly;
the conservative estimate was increased without changing predicates. Both
tiny-budget fallback tests remain in place. Two bounded read-only reviews found
no material correctness defect; they did not substitute for executed tests.

The final local-only comparison uses the same 512 px, eight actual pinned RGBs,
100 mm and explicit legacy NearWhite rule. Both versions pass independent DST
coordinates, controls, lengths, dimensions and byte-identical ELAB regeneration;
the original image hash is unchanged. Baseline → final candidate:

| Measure | Baseline | Final candidate |
| --- | --- | --- |
| Objects | 820 | 897 |
| Needle stitches | 25,684 | 26,823 |
| Reader-normalized jumps | 10,223 | 9,949 |
| Physical jump distance | 32,211.053 mm | 33,467.363 mm |
| Color changes | 7 | 7 |
| Maximum physical stitch | 3.4 mm | 3.4 mm |

All 897 candidate objects, compared by stable ID before/after batching, retain
exactly the same geometry, settings, visibility and colors. The original source,
physical width, palette, machine profile and recipe identities also match.
Only object order changes. The final sequence has about **3.9% more jump distance**
than the baseline; seven color changes do not establish globally optimal travel.
Its 784 trim intentions map to the selected ThreeJump profile. Raw DST jump
records, reader-normalized jumps and planned jumps are separate metrics.

An isolated debug generation with Cargo paused took **31.426 s**. The complete
validation rerun generated in **44.357 s** while the UI build was active;
baseline generation was **11.596 s**. These are individual process runs, not a
controlled speed benchmark. The full independent Python analysis is additional
work and is not included in generation time.

The earlier real-image project-limit rejection is preserved. Omission reporting
now retains 512 locations and one aggregate covering **11,418 omitted regions,
15,937 trace-map pixels, 607.9483 mm²**. Diagnostics include **749 skipped
underlays**, **551 uncovered-area warnings** and three analysis limits. The
candidate's advisory top-only coverage sample is **73.228%** under the stated
0.30 mm/0.23 mm assumptions; legacy baseline roles cannot be separated and are
unscored. The simplification retains the motif while losing fine detail and is
not a machine/material approval. Private source and derived images stay local;
none is embedded in public fixtures, this report or the GUI contact sheets.

The unchanged `tools/quality_v05.py` completed **PASS: eight cases, 16 DST/DSB
files, four invalid-input checks** on the same final executable. Its schema-v4
Satin fixtures still use `length_mm=8.0`; compatibility accepted them without
edits or relaxed checks. Results are only in ignored
`outputs/autodigitize/legacy-quality-v05`, since one fixture derives from local
real artwork. Four additional controlled DST swatches isolate compensation,
row spacing and underlay for a future sew-out; none has been sewn.

## Current-session checks

- Snapshot `cargo fmt --check`: **PASS**.
- Snapshot `cargo test --locked --no-default-features -j1`: **144 passed,
  0 failed, 1 ignored** (external installed DSB corpus). The first attempt
  lacked PDFium beside the isolated target and failed six PDF tests; the
  completed rerun explicitly bound the already pinned DLL via
  `EMBROIDERYLAB_PDFIUM_PATH`. Independent reader tests used the actual Python
  3.12 environment. See `outputs/autodigitize/baseline-test-headless.log`.
- Four minimal copied engine regressions against snapshot: **0 passed,
  4 expected failures**, exit 101. They reproduce row jumps (50 instead of 1),
  missing compensation at an independent shell boundary, auto-angle changing
  with collinear resampling and the empty thin object. These are deliberately
  separate from the original passing baseline suite; see
  `outputs/autodigitize/baseline-regressions.log`.
- Baseline GUI all-targets and strict Clippy: not rerun on the snapshot;
  current candidate checks are recorded separately below.
- Consolidated correction source before the targeted empty-footer repair:
  **341 tests passed, 0 failed, 1 ignored** across
  26 suites: **217 headless/library/integration and 124 GUI** tests. The ignored
  case is the external installed DSB corpus. Strict all-targets Clippy with
  `capture` passed; the final debug bins built successfully. Exact totals and
  binary hashes are in `outputs/autodigitize/redesign/all-final-summary.json`;
  the executed log is `outputs/autodigitize/redesign/test-all-final-confirmed.log`.
  Confirmation GUI SHA-256 is
  `8652f2c64b0e024c3c9fba9215098bca99058fe1ca8a3986806fb059cd39cc36`;
  the validated CLI remains `18b66cbe...631601d` as recorded above. The initial
  optimized build passed earlier; the final portable build is reported separately.
- Bounded pre-redesign native GUI flow on the copied 01:06 debug executable:
  **PASS** for the actions detailed below. The framebuffers described here
  establish the incumbent layout, not the newly requested redesign.
- Physical sew-out and machine/material acceptance: **not performed**.

DSB remains experimental. DST cannot preserve editable vectors or an exact
palette; keep the ELAB and the exported thread sidecar with machine files.

## Bounded native GUI evidence

`outputs/autodigitize/gui/native-qa.json` records the tested executable hash,
actions and limitations; `native-*.png` contains actual application captures.
The test used only a synthetic offset flag and isolated `APPDATA` and
`LOCALAPPDATA` under `outputs/autodigitize/gui-state`. The user's existing
Russian preferences retained their November 2 timestamp and SHA-256; no real
unsaved recovery file was created or overwritten. The scratch editor exited
normally after saving its project.

Observed native actions:

- Set physical source width to 64 mm and ran the primary import action:
  eight editable objects and 5,194 needle stitches.
- Ctrl+Z restored the prior single-object document in one operation; Ctrl+Y
  restored the import. Ctrl+E while stale was blocked with the explicit
  instruction to generate and wait before exporting.
- Reopened the saved source; dragged a fixed exterior white seed from logical
  window `(350,350)` to `(800,500)` and released **outside the image**.
  Tolerance changed **8 → 64.561** and selected exactly **57,841** exterior
  white pixels. Necessary enclosed white remained unselected.
- Delete applied the nondestructive exclusion; Ctrl+Z undid that complete
  mask operation and Ctrl+Y restored it. The document's eight objects remained
  unchanged until generation committed the new result.
- Generation then retained object IDs **2–8**, removed the excluded exterior
  component and produced seven editable objects, including actual paired-rail
  Satin outlines. The saved schema-v5 ELAB contains the full 256×256 source,
  a 57,841-pixel exclusion mask and two retained white objects. Decoded RGBA
  source pixels equal the original exactly; lossless PNG encoding bytes may
  differ.
- The same 1,389-command plan appeared in 2D and 3D: **1,364 needle stitches,
  17 jumps, 23.7×20.3 mm**. The 64 mm setting describes the complete source
  canvas including margins; the status measurement describes sewn segments.
- Russian→English changed the native controls. At simulation command **500**,
  only the already-sewn black frame was visible, with no future colored faces;
  the whole-design size remained unchanged.

The first computer-use capture was white; a later screenshot-backed state
rendered normally. Early actions attempted without screenshot geometry were
rejected by the helper and were not counted as successes. Native Escape
during a held gesture, return-to-origin within one held gesture, other zoom/
DPI configurations, protection-brush editing and full keyboard-only canvas
editing were **not** verified in this bounded run.

The native environment used 150% scale. Its drag result matches the logical
point formula `8 + 0.12 × (hypot(450,150) − 3) = 64.560998`; physical framebuffer
pixels were not substituted for egui points. Other DPI settings were not
compared in the native run.

## Selection computation timing

The release benchmark `examples/selection-benchmark.rs` measured a deterministic
generated image with 24 changing-threshold samples per mode on one exposed
CPU. These are **cache/mask computation times**, not OS-input-to-displayed-frame
latency. Full GUI end-to-end latency was not measured.

| Source | Cache once | Connected median / p95 | Exterior median / p95 | Global median / p95 |
| --- | --- | --- | --- | --- |
| 512×512 | 27.963 ms | 1.107 / 1.185 ms | 1.070 / 1.133 ms | 0.483 / 0.548 ms |
| 2048×2048 | 435.801 ms | 20.173 / 25.819 ms | 20.057 / 24.250 ms | 8.858 / 8.976 ms |

Raw hardware descriptor, build type and measurements:
`outputs/autodigitize/selection-benchmark.json`. Source distances are cached
once; each threshold recomputes the mask from the frozen gesture basis.

## Pre-redesign framebuffer layout review

The public application screenshot API produced eight project frames (2D/3D,
RU/EN, both viewport sizes) and four import-dialog frames. The requested
**logical viewports** are 1360×900 and 1024×680. At the native Windows 150%
scale, their unchanged **physical framebuffers** are respectively 2040×1350
and 1536×1020. Both dimensions have the same 1.5 ratio. The original physical
PNGs are retained; no resampling is used to manufacture size acceptance.

The initial capture harness wrongly required physical pixels to equal logical
points. This was corrected to validate the uniform native scale explicitly;
the applications had exited successfully and produced valid frames.
`gui/captures.json` and `gui-import/captures.json` record executable/project
hashes, actual pixels, logical viewports and scale. Contact sheets were
visually reviewed: primary actions and import controls remain visible at
1024 points in Russian and English. The captured views show the same plan
and stitched dimensions. Their executable hashes identify this pre-redesign
evidence; redesigned native acceptance is recorded separately.


## Redesigned workstation: first batched native review

The first review contains **24 application framebuffers and 20 native operation
captures**, with synthetic public artwork only. Its immutable GUI SHA-256 is
`c4cd02b0e5e597700b95e3e8cc0781f1227f5d2fb1b4bfca691e2ac9dd710a8f`.
Its immutable manifest is
`outputs/autodigitize/redesign/workstation-first-archive/matrix.json`.
The authoritative `.impeccable/review/workstation-first` paths are overwritten
only by the single confirmation pass and must not be read as c4cd02b0 evidence. Dark Russian/English 2D and 3D,
secondary light, initial empty state, source/cleanup, native Save/Export,
real errors and collapsed workspaces are represented.

At the actual Windows **150%** scale, logical **912×512, 1280×720 and
1707×960** yielded physical framebuffers **1368×768, 1920×1080 and
2561×1440**. These approximate the requested 1366×768, 1920×1080 and
2560×1440 window classes. Original PNGs are retained. Contact sheets are
thumbnails only. Sky's whole-window JPEGs report their own logical dimensions
and can include the helper's pointer halo; those are not additional physical
framebuffer or application styling claims. Other OS DPI scales were not tested.

Observed operations include fixed-seed exterior selection of **57,841 pixels**,
nondestructive Exclude and button Undo/Redo, document Ctrl+Z/Y, stale-export
blocking, fresh generation, Vector/Stitches/3D switching, native ELAB save and
DST export. The resulting plan has **1,388 commands, 1,364 needle stitches,
17 planned jumps and four color changes**, with **23.7×20.3 mm** sewn bounds.
PyEmbroidery independently decodes the actual GUI export; its machine bytes
are identical to CLI regeneration. The full 256×256 source RGBA is unchanged,
the exclusion RLE has 57,841 pixels and both enclosed white objects remain.
The 1,389-command static fixture is a separately preserved project with its
existing layer order; its own 2D/3D captures agree with each other.

Light theme, Russian language, both collapsed sidebars and collapsed timeline
survived an actual application restart. Their changes leave the saved project
hash unchanged. The user's real preference file retains its original hash;
all app state and recovery work used isolated local directories. The QA app
closed normally. First-pass findings are cleanup Ctrl+Z after button focus,
a black unthemed viewport-header band in Light, letter-wrapped tool-rail
Add/Create and the Russian Add/Create translation collision. This first set
is evidence for the single correction batch, not final visual acceptance.

The native helper observed genuine worker errors and retained the previous
document, but missed the short loading states. On one long private job its
framebuffer capture timed out, and metadata recovery found that the job had
completed. **Native OS-click active cancellation is unverified.** The scratch
import was undone and the app closed; no further manual long-job attempt ran.

A separately labeled application-owned capture probe completed the required
real loading/cancel evidence. It used the actual preview/trace pipeline worker,
requested a real egui framebuffer while phase 1 was pending, invoked the actual
cancel handler before completion and observed the worker's cancellation error.
There was no artificial pause or fake progress. The cancel token changed
false→true; the previous **2,294-command** valid plan remained non-stale.
Independent before/after document and plan SHA-256 hashes match exactly.
The probe GUI SHA-256 is
`29d6d3a09309799b2aab1f280a8431b792bca5c2f9f87d42a8335c5aa4749182`;
it includes capture instrumentation and is not the c4cd02b0 capture binary.
Sanitized metadata is in `native/programmatic-worker-supplement.json` beside
the first matrix. The actual private PNG and source paths remain solely in the
ignored private output directory and are excluded from public contact sheets.
This verifies the application handler programmatically, not a native Cancel
click or a physical machine operation.

## Redesigned workstation: single confirmation

The single confirmation replaced the same authoritative capture paths after
the eight ordered corrections. The final matrix is
`.impeccable/review/workstation-first/matrix.json`; the standalone mirror is
`outputs/autodigitize/redesign/workstation-confirmation/index.html`.
It contains **24 final application PNGs and 20 native operation JPEGs**.
The before/after page retains a separately identified pre-redesign image;
that historical image is not a 25th final capture. The three window classes
and actual 150% scale remain as recorded above. Debug GUI SHA-256 is
`8652f2c64b0e024c3c9fba9215098bca99058fe1ca8a3986806fb059cd39cc36`.
The restart and two genuine-error images use the packaged release GUI
`3f1a2fe8311d093f082a5f0650a559c56083e17b2f3092afde2a952ef7b93eb4`;
each native record identifies its actual binary.

Native wheel zoom progressed **100→127→162→205%**; selection at 205% and
after Fit both selected exactly **57,841 source pixels**. The fixed-seed
drag released outside the preview retained those pixels at tolerance 73.4.
After clicking Exclude, Ctrl+Z restored the mask, Ctrl+Y reapplied it, button
Undo restored it and Ctrl+Shift+Z reapplied it. Exclude was observed disabled
when no pixel would change and was not clicked in that state. The retained
first-review filename `cleanup-keyboard-undo-no-change` now shows successful
Undo; `cleanup-redo-button` shows the successful keyboard redo.

Document Undo/Redo and stale export protection passed again. Dirty
File→New showed a save/discard/cancel modal with background controls disabled
on its first observed frame; Cancel retained the valid **1,388-command**
document. Native Save and Export produced the same ELAB SHA-256
`7447b227720aa8b11c0314ead0ceaff43f14f90f59bdce5372f2c4c67a6db081`
and DST SHA-256
`0ee5e35fcfa0e994a79bcfe67328829bad2af13542e524e3196d772c4ab99f4b`
as the first review. PyEmbroidery and CLI regeneration confirm identical
machine bytes, **1,364 needle stitches, four color changes**, the retained
full source RGBA, **57,841 excluded pixels** and both enclosed white objects.
The independent result is `native/independent-native-export.json`.

The actual packaged GUI launched with **System32 and Windows only** on PATH,
external PDFium environment variables unset, and isolated APPDATA/LOCALAPPDATA.
It restored Light/Russian and both collapsed sidebars plus timeline, then
retained the previous document after a real synthetic job hit the 4,096-region
limit. It closed normally. The preference file before/after restart is
identical; the user's real preferences retain their original SHA-256.
`native/portable-gui-lifecycle.json` records this lifecycle check. The parent's
separate portable manifest/PDF and release CLI evidence is in
`outputs/autodigitize/redesign/portable-smoke/summary.json` and
`outputs/autodigitize/redesign/release-corpus/summary.json`.

The final debug GUI also repeated the genuine application-owned worker probe
at the same private path. Its pending phase-1 framebuffer was saved before
the actual cancel handler changed the token false→true; the worker observed
cancellation and the previous 2,294-command plan remained non-stale. Document
SHA-256 `6e2bfcaa7a1bbf58438c5512523119a923fa120dcc09ef6368ff86fbabb0f943`
and plan SHA-256
`efc10a4ba49f86dc59249a20edeee5bf6b77c5b6fe3a36672e23f221177af174`
match before/after. Only sanitized metadata is public; the original image
stays under ignored `outputs/autodigitize/private-real/gui/programmatic-probe`.

The first final private probe attempt produced a file-not-found image because
the launch harness decoded its Cyrillic source path with Windows ACP. An older
passing report remained present, so that image/report pair was incorrectly
accepted. It was rejected during root's image inspection. The technical
recapture used explicit UTF-8 decoding, verified the source file, moved the
old report aside and required the new report timestamp to follow the run
start (**2026-11-06 08:31:32 UTC**; fresh report **08:31:42 UTC**).
The fresh report passed and root inspected the actual phase-1 loading
frame with an opaque modal. Final private PNG SHA-256 is
`0328028a5e929d5cf34cc0695cfbddd8e6e526adaa58462d3a5f613ff598dc1e`;
proof SHA-256 is
`070f0fcc8d67eb51cf00add5817d1e7a4d8ec2aebc27db277fd6db9028aa49c1`.
The invalid private image is retained as `invalid-loading-path-error.png`.
This repaired harness evidence; application source and all 44 public capture
bytes remain unchanged. The sanitized supplement records fresh-report and
source-path checks, timestamps and unchanged document/plan hashes.

**Native OS-click active cancellation and held middle-button pan remain
unverified.** The documented automation drag API has no held-button argument;
the real pan handler and logical coordinate/event tests passed in Rust.
Other OS DPI, NVDA acceptance and a physical fabric sew-out are also outside
these results. No additional cosmetic round was performed.

The final reviewer then found one remaining functional layout regression:
the zero-object guidance wrapped after the objects-list height was measured,
clipping the footer in the narrow English empty workspace. Moving that
guidance before measurement repaired the layout. This was an additional
targeted functional correction after the consolidated confirmation; only
the existing English/Russian **912×512** empty-workspace capture paths were
replaced. Both raw **1368×768** images were inspected and show the complete
footer controls. The other **42 captures** and real-worker proof were retained.
Prior two images and metadata remain under
`outputs/autodigitize/redesign/footer-targeted-pre-repair`.

These two frames use debug GUI SHA-256
`96c9e51ba454f3a68c1b0d28a1f0f1f4f4b742f77513f02cb9f7a59b91ef3d15`.
Each matrix entry identifies its actual binary; the other images retain
their earlier hashes. The repair changed `layer_panels.rs`; **125 GUI tests
passed**, including footer bounds in English/Russian, Dark/Light and both
912×512/850×480 logical sizes, with strict Clippy and formatting checks passing.
The GUI test log is `outputs/autodigitize/redesign/test-gui-footer-final.log`;
the two-frame repair manifest is
`outputs/autodigitize/redesign/footer-targeted-confirmation.json`.
The follow-up review is limited to the previously remaining finding 6.

The fresh reviewer marked that remaining finding **resolved** and returned
**disposition: ship**, explicitly covering the eight scored fixes only.
`finish-verdict.md` is retained beside both the authoritative matrix and its
confirmation mirror. The extra isolated footer repair and its two refreshed
frames exceeded the initially planned cosmetic verification ceiling; they
are recorded separately rather than described as an unchanged two-round
process.

The final delivered release GUI SHA-256
`472aca2c1ac8dc7780cec95c1a13666c2f8fc5fc98ae39db4ee0e5ea27b228a9`
also completed a lifecycle-only native launch and Alt+F4 close. PATH contained
System32 and Windows only, external PDFium variables were unset, preferences
were isolated, and the observed real application view retained the valid
1,388-command/1,364-needle project. No new picture or cosmetic inspection was
added. No application window remained afterward; project bytes and the user's
real preferences stayed unchanged. The proof is
`native/portable-footer-gui-lifecycle.json`. Earlier complex-state evidence
keeps its own binary hashes.
