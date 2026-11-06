# Reviewed Windows dependency notices

This support cache fills notice gaps in the exact Windows GUI dependency graph
recorded in `outputs/autodigitize/redesign/windows-metadata.json`. It does not
replace crate licenses or change the existing release packager.

`manifest.json` identifies each package/version, cached crate VCS commit,
repository, authors, upstream SPDX expression, chosen license branch, exact
source URL and SHA-256 for every supplementary file. Upstream texts retain their
original copyright notices and years. Apache-only crates additionally include
the unchanged leading copyright/license comment from their matching upstream
source; its complete source-file hash is recorded separately. AccessKit's
Chromium notice is retained in addition to the chosen primary license.

The font package retains the egui code license plus its four embedded font
notices. Sources are read at the `.cargo_vcs_info.json` commit of each cached
crate, rather than a moving branch. `upstream-tree-evidence.json` records the
primary repository trees inspected for additional license/notice files.
`windows-notice-gaps.json` is the initial inventory, before these supplements.

The new local preparation script verifies every support-file hash and crate
commit, copies full license texts, and writes per-package attribution/provenance
and aggregate coverage. A dependency without complete copied license material,
an unmatched reviewed version, or a changed support file fails preparation.
Preparation uses this local cache without network access. Newly required MinGW
runtime DLLs fail until their exact distribution notices receive a separate
review; the currently inspected executables require only Windows system DLLs.

The checked native graph has 201 dependencies, including 20 supplemented
packages. The isolated license-only preview is under
`outputs/autodigitize/redesign/license-audit-preview`; it is not a runnable app
bundle and does not replace the final release build or launch verification.
