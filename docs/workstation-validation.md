# Workstation implementation and validation

This is the historical verification of the workstation redesign. The later
algorithm completion, current build provenance and additional native captures
are recorded in [algorithm-completion.md](algorithm-completion.md). Earlier
executable hashes and counts below describe that earlier verification round.

This report concerns the native Windows Rust/egui workstation redesign. The
algorithm comparison and independent DST/DSB checks are documented separately
in [autodigitize-validation.md](autodigitize-validation.md). No physical stitch-out
was performed. No repository publication or online release is part of this work.

## Implementation boundary

The application retains eframe/egui 0.33.3 and the OpenGL stitch viewer. The new
shell, theme, object panels, inspector, sequence panel and render-only canvas
cache are separate modules. They call the existing project/history, import,
generation and export operations. Theme changes preserve artwork RGB and the
theme-independent neutral artwork plate. Locks and visual color groups belong
to the workspace; they do not invent ELAB geometry or machine commands.

The supplied dark-first direction replaces the earlier light visual system.
Impeccable's context bootstrap ran once in this session; init preserved verified
product facts. Critique, shape/new-work, native audit, polish, harden and final
documentation use the installed 4.3.1 skill. The user explicitly chose code-first
implementation. There is no approved bitmap comp or claimed pixel reproduction.
The direction seed is `ba7dba5c`, candidate four, recorded in the surface brief.

## Evidence and scope

The initial redesign evidence is archived under
`outputs/autodigitize/redesign/workstation-first-archive/`: 24 application
PNGs and 20 native computer-use JPEGs. Captures include empty and imported
workspaces, cleanup masks, selected vectors, stitches, 3D, export, an actual
worker error, collapsed panels, Russian and English, and both themes.

| Requested display class | Logical viewport | Actual physical framebuffer |
| --- | --- | --- |
| 1366×768 | 912×512 | 1368×768 |
| 1920×1080 | 1280×720 | 1920×1080 |
| 2560×1440 | 1707×960 | 2561×1440 |

These are unchanged native pixels at actual Windows 150% scale. Rounding
accounts for the two-pixel/one-pixel width differences. They are not a claim
of testing every monitor, multi-monitor transition, or Windows scale setting.
Automated pointer and layout tests exercise logical coordinates at additional
pixels-per-point settings. Screen-reader and Windows high-contrast acceptance
remain unverified.

The public evidence uses generated synthetic artwork. Private local artwork
and its derived captures remain in the ignored `outputs/autodigitize/private-real`
directory and are not copied into the public gallery.

The same authoritative paths under `.impeccable/review/workstation-first/`
hold the single post-fix confirmation (`matrix.json` and `index.html`), with
executable hashes distinguishing the two rounds. Contact sheets are derived
thumbnails; the underlying PNGs preserve their actual framebuffer pixels.

## Real worker cancellation

The capture-only probe starts the actual tracing worker, requests an ordinary
application screenshot while that worker is running, invokes the application's
unchanged cancel handler, and observes the worker's cancellation result. It
does not pause the worker or synthesize progress. Before/after project and
machine-plan snapshots are compared independently.

`private-real/gui/programmatic-probe/real-worker-loading.probe.json` records a
successful run: real stage 1, cancellation requested before completion, actual
worker cancellation observed, and exactly preserved project and last valid
2,294-command plan. Their SHA256 values are respectively
`6e2bfcaa7a1bbf58438c5512523119a923fa120dcc09ef6368ff86fbabb0f943` and
`efc10a4ba49f86dc59249a20edeee5bf6b77c5b6fe3a36672e23f221177af174`.
The loading frame was visually inspected. Native OS pointer activation of the
Cancel button during this long job is **unverified**: the computer-use capture
helper timed out; this programmatic handler check does not replace that claim.

One attempted confirmation had an invalid loading image: the Python wrapper
read the UTF-8 source filename with the Windows locale encoding, and then read
an older successful report. Visual inspection caught the path-error frame.
That attempt is rejected and retained privately as
`invalid-loading-path-error.png`. The technical recapture verifies the source
path, reads UTF-8 explicitly, requires a newly created report newer than the
run start, and checks actual 2291×2291 source dimensions. The fresh final
report passes, and its opaque loading modal was inspected. This repaired
invalid evidence; it did not involve UI edits or a further cosmetic round.

## Final checks

The consolidated production correction batch is frozen. On the final source:

| Command | Result |
| --- | --- |
| `cargo fmt --manifest-path embroidery-engine/Cargo.toml --check` | PASS |
| `cargo test --locked --manifest-path embroidery-engine/Cargo.toml --no-default-features -j1` | 217 passed, 0 failed, 1 ignored |
| `cargo test --locked --manifest-path embroidery-engine/Cargo.toml --all-targets --features capture -j1 --no-fail-fast` | 341 passed, 0 failed, 1 ignored; includes 124 GUI tests |
| `cargo clippy --locked --manifest-path embroidery-engine/Cargo.toml --all-targets --features capture -j1 -- -D warnings` | PASS |
| `cargo build --locked --manifest-path embroidery-engine/Cargo.toml --features capture --bins -j1` | PASS |

The ignored test requires an external installed Embird DSB corpus. PDF tests
ran with the available pinned PDFium runtime. Headless and all-target totals
overlap; they must not be added. Logs are in `outputs/autodigitize/redesign/`:
`test-headless-final.log`, `test-all-final-confirmed.log`, `clippy-final.log`,
`fmt-final.log`, and `build-debug-final.log`.

The complete confirmation's debug GUI SHA256 is
`8652f2c64b0e024c3c9fba9215098bca99058fe1ca8a3986806fb059cd39cc36`.
The later empty-footer-only debug build is
`96c9e51ba454f3a68c1b0d28a1f0f1f4f4b742f77513f02cb9f7a59b91ef3d15`.
The CLI hash is unchanged from the independently checked final engine:
`18b66cbe7d804a85210ee9becf8ea0c9e27c770c6ae4e465348d3667e631601d`.

Meaningful UI regressions cover pointer-anchored canvas zoom and pan without
document edits, text-versus-document shortcut ownership, cleanup history after
ordinary button focus, both redo bindings, fixed-seed reverse tolerance at two
pixels-per-point settings, zoomed source-coordinate selection and outside
release, clipped image hits, disabled no-op exclusion, first-frame modal input
blocking, machine-only property guards, locked-neighbor ordering, actual
opaque loading chrome, localized minimum-size import actions, and 2,048 long
layer names. Native state changes remain separately verified in the matrix.

An intermediate run found one test-fixture assumption: after Exclude becomes
disabled it correctly loses focus. The corrected test first clicks real
Exclude, then keeps an enabled native button focused while exercising mask
Undo/Redo/Delete. The clean complete rerun above includes that correction.

The initial finish review returned **fix**, listing eight material findings in
`outputs/autodigitize/redesign/finish-review.md`. The one bounded confirmation
contains 24 final application frames and 20 native-operation captures. The
scoped Impeccable verdict is **ship** after the isolated footer repair below.
The exact scope and scoring response are retained in
`.impeccable/review/workstation-first/finish-verdict.md`.

The scoped scoring pass resolved seven findings but kept finding 6 partial:
localized empty guidance wrapped after the list's fixed height, clipping the
footer at 912×512. To finish the requested repair, one isolated functional
correction measures that guidance before allocating the list. No populated
workspace, stitch engine, cleanup, token or other workflow changed. An added
AccessKit bounds regression checks complete 28-point footer controls above the
sequence in English/Russian, Dark/Light and both 912×512 and the declared
850×480 minimum. All eight combinations pass. The final GUI rerun passes
**125 tests**, with formatting and strict all-target Clippy clean.

This required **one additional narrowly scoped repair/check beyond the
planned cosmetic ceiling**. Only the two existing English/Russian empty912
capture paths were refreshed; the other 42 captures keep their explicit
earlier binary provenance and unchanged-component scope. The reviewer scores
only the remaining footer finding, without a new design hunt. Logs:
`test-footer-regression.log`, `test-gui-footer-final.log` and
`build-debug-footer-final.log`. The earlier 341-test run comprises the same
217 unchanged core tests and the previous 124 GUI tests; the latest GUI count
is 125, not a claim of re-running all targets as one 342-test command.

Native confirmation verifies cleanup wheel zoom through 205%, Fit reset and
the same 57,841 exterior pixels at both scales; release outside the preview;
disabled no-op Exclude; keyboard Undo, both redo bindings and button Undo;
document history/stale export protection; actual new-project Modal/Cancel;
vector and 3D views; real Windows Save/Export; Light/Russian and collapsed
workspace persistence. Both enclosed white components survive, source RGBA is
unchanged, and the actual saved DST is byte-identical to CLI regeneration and
the first native pass. Held-middle-button input is unavailable in the native
automation helper: manual middle-pan remains **unverified**, with real-handler
tests at pixels-per-point 1 and 2 providing programmatic coverage.

## Local runnable release

The final optimized `capture`-enabled binaries were built with the existing
GNU Windows toolchain, `--release --bins -j1`, path remapping and deterministic
PE timestamps. The cached dependency build was reused; the final crate build
took 90 seconds. Normal launches do not activate capture/probe instrumentation.

`tools/prepare-autodigitize-app.ps1 -BuildProfile release` prepared
`outputs/autodigitize/app/` without compiling, deleting, archiving or publishing.
It contains 601 recorded files: GUI, CLI, pinned PDFium, instructions, product
and design documents, and dependency notices. All copied sizes and SHA256
values were checked against `build.json`. The Windows dependency graph has
201 notice sets, including 20 exact-commit reviewed supplements, with zero
unresolved entries. Preparation used no network. PE imports require Windows
system DLLs plus the included PDFium; no MinGW runtime DLL was required.

| Release file | SHA256 |
| --- | --- |
| `embroidery-lab.exe` | `472aca2c1ac8dc7780cec95c1a13666c2f8fc5fc98ae39db4ee0e5ea27b228a9` |
| `embroidery-cli.exe` | `fb2d5fa3eaed8c83a35891ee37e0a467edee16e96a9496c1761c310dbfadfd19` |
| `pdfium.dll` | `4b0dec92555d69437bef0df37be630181ab3e83a6823ca264ede355a4f2ef994` |

The **packaged** release CLI passed the same 24/24 independent synthetic
checks, including DST and selected DSB reads and four trial swatches, under
`PATH=C:\Windows\System32;C:\Windows` with the external PDFium variable absent.
A PDF fixture was also rendered through the bundled DLL and generated a DST
with 1,476 independently decoded needle stitches. Logs and hashes are in
`redesign/release-corpus/` and `redesign/portable-smoke/` beneath the ignored
autodigitize output tree. Actual packaged GUI lifecycle is checked with the
native confirmation, separate from the earlier debug framebuffer binary.
The packaged GUI was actually opened under the restricted PATH, restored
Light/Russian and collapsed panels/timeline, retained its valid document after
a genuine region-limit error, and closed normally. Its saved ELAB remained
unchanged. Three native captures identify the release executable separately
from the 17 debug native captures and 24 debug framebuffers.
That complex-workflow native run used release `3f1a2fe8311d093f082a5f0650a559c56083e17b2f3092afde2a952ef7b93eb4`.
The subsequent footer-only release rebuild took 30.46 seconds and has the
current GUI hash in the table; the independently checked CLI is unchanged.
The exact delivered `472aca…b228a9` executable was then launched under the
same restricted environment and observed through the native view tree with
its valid 1,388-command plan. It closed normally, leaving no application
window. Saved project and real user preferences remained unchanged. This was
a lifecycle-only final-artifact check, with no saved screenshots or new
cosmetic inspection. Proof: `native/portable-footer-gui-lifecycle.json` in
the authoritative confirmation tree.

## Remaining product limits

There is no fabricated split comparison view, layer drag-and-drop, automatic
overlap subtraction or wide-Satin splitting. Supported existing actions remain
available through their actual handlers. DSB remains experimental. A rendered
3D preview communicates the machine plan; it does not certify tension,
stabilizer choice, registration, fabric coverage or stitch-out quality.
