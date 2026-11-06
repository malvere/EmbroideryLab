# EmbroideryLab

<!-- impeccable:product-schema 1 -->

## Platform

Windows desktop, with portable Rust core for later macOS/Linux support.

## Stack

Delegated explicitly by the user: Rust with egui/eframe. Native editor and independent library, open JSON project format, DST first, DSB later.

## Users

Embroidery digitizers, machine operators, designers and enthusiasts editing logos, emblems, flags and flat-color artwork. The existing Fortever FT-1201R use case remains relevant; the editor must not assume that all users have that machine.

## Product Purpose

Import JPEG/PNG/WebP/PDF/SVG, correct vector contours by hand, configure stitching, preview and simulate, export and independently validate DST. Raster format is detected from content; WebP sources embed as lossless PNG for existing ELAB compatibility. PDF artwork is imported one selected page at a time as a rendered image. SVG retains editable solid fills, outlined strokes, transforms and local use references without raster tracing; text, raster images and unsupported compositing remain explicit errors.

## Operating Context

Windows 11 VM. Local-only research and authoring. No physical machine control. User explicitly requests autonomous technical choices and no intermediate questions.

Primary workflow: import artwork → clean colors/background → vectorize → generate stitches → inspect/edit → simulate → export. Beginners need a clear first result; experienced operators need precise numerical parameters and an editable sewing order.

## Capabilities and Constraints

Running, satin, tatami, underlay, compensation, density, sequence and editable vectors. Preserve samples and Embird installation. No licensing changes, protection bypass or copied decompiled code. Physical stitch quality remains unverified until sewn on fabric.

The current core includes deterministic perceptual palette reduction, saved-source exclusion/protection masks, fixed-seed color selection, local antialiased-fringe cleanup and shared raster boundaries. Contour simplification and smoothing use physical error limits with a diagnosed valid fallback. Automatic Running/Satin construction is conservative; unsuitable regions retain editable Tatami with a reason. Generation carries object/pass roles, separate underlay settings, layer cutouts and bounded sewn-travel checks into the normalized machine plan.

Generating from edited objects retains those edits. Re-digitizing the saved source is separate: when automatic classification is enabled, it precedes restoration of proven manual overrides; changed source identities produce a conflict where a manual decision cannot be matched. Source artwork, cleanup, physical scale and import decisions remain in ELAB. See [algorithm completion](docs/algorithm-completion.md) for the implementation boundaries and [workstation usage](docs/workstation-usage.md) for the actual cleanup/re-digitization actions. These capabilities do not establish fabric quality or arbitrary-photo/lettering automation.

## Evidence on Hand

One supplied JPG, one EOF and one DST in samples/. Their correspondence is to be established; no additional user designs exist. Demo artwork must be labeled synthetic.

## Product Principles

Editable geometry before irreversible machine export. Explicit units and parameters. Separate observations from hypotheses. Honest status and recoverable file operations.

## Brand Commitments

Keep the EmbroideryLab name and existing project identity. The user explicitly requests a dark-first professional creative workstation, a secondary light theme, restrained cyan active states and warm orange warnings. Blender, Affinity Designer and DaVinci Resolve establish the desired craft level and familiar desktop behavior. This replaces the earlier session's inferred light-only direction.

## Accessibility & Inclusion

Legible controls, visible keyboard focus, accessible contrast, Russian and English UI, explicit physical units, native Windows behavior and logical-point sizing at different DPI scales. Layout must remain useful at common desktop window sizes without opening every panel at once.

## Compatibility

Preserve Rust/egui/eframe and OpenGL, existing keyboard commands, undo/redo, editable ELAB projects, raster/PDF/SVG import, normalized DST export and experimental DSB behavior. UI work must not rewrite embroidery algorithms or silently discard manual edits. Workspace preferences are separate from embroidery project data.
