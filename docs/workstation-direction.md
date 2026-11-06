# EmbroideryLab workstation direction

Surface: native Windows editor, `embroidery-engine/src/main.rs`. Mode: Operate.
Implementation: code-first, explicitly selected by the user on 6 November 2026.
Impeccable context bootstrap ran once earlier in this session; it is not repeated.

## Job and boundaries

Build an embroidery workstation people can use for hours: artwork import and cleanup, editable geometry, stitch generation, technical inspection, simulation and export. Preserve the Rust/egui/OpenGL architecture, algorithms, file formats, manual edits and recovery. All controls must invoke real supported operations. Artwork colors are document data, independent of the UI theme.

## Direction contract

The user pins a professional charcoal/graphite creative workstation, restrained cyan active tools and selection, orange warnings, and a secondary light theme. A digitizer works under ordinary studio lighting while comparing thread colors; quiet neutral chrome reduces glare and leaves color judgement to the artwork. There is no glow, ornamental gradient or dashboard-card layout.

The seven grounded systems considered were: a vector-editor tool bench; a physical thread-library index; a drafting plate and dimension table; a color-grading control room; a machine job ticket; a plotter legend; a repair bench with a focused work area. The required direction seed assigned candidate four (`ba7dba5c`): the color-grading control room. Its persistent media canvas, shallow tool rows and precise side controls fit the user's explicit DaVinci Resolve reference. The user's pinned facts override any random material assignment.

The catalog challengers were translated into real editor tasks before judgement: warm consumer cards weaken dense editing (declined; retain an action-only accent); orienteering provides an invariant artwork layer and clear legend (competitive on state clarity, retain that discipline); boarding-pass/gate boards provide stable row identity and priority columns (declined as the overall form, retain stable object rows); transit maps provide context-preserving focus (declined as the overall form, retain panel collapse without losing the canvas); coded catalog sleeves conceal labels (declined on product clarity); split-flap boards provide aligned measurement columns (declined as the overall form, retain numerical alignment). No fictional capability is introduced by these references.

### First viewport

The file name and dirty state sit in a calm top bar. Open, Save, Undo/Redo and Export are immediately available; Auto Digitize is the primary action. A compact vertical tool rail opens the largest area to the design. Layers on the left and the contextual inspector on the right are resizable and independently collapsible. Vector/Stitch/3D modes, Fit and view toggles belong directly above the canvas. A shallow bottom summary expands into the actual command timeline and playback controls.

### Signature interaction

The fixed-seed cleanup gesture is the product's signature: dragging changes tolerance while the selection overlay and numerical count respond; returning toward the press point shrinks the mask. Cleanup stays nondestructive, preserves enclosed white, and returns to the editor through a transactional Generate action. Inspector changes mark the actual preview stale until regeneration. View mode, theme and panel changes never change the machine plan.

### Motion and state

Native hover/focus/press feedback and interruptible panel disclosures; no entrance sequence. Repaint only for input, worker polling or active playback. Loading shows real stages and cancellation. Errors retain the document, explain recovery and do not expose stale export. Disabled actions stay visible with a reason.

## Verification contract

Run Rust formatting, build, strict lint and existing state/geometry/export regressions. Capture empty/synthetic, imported raster/cleanup, selected vector, stitches, 3D, export, loading and error states. Inspect the requested desktop classes and native scaling in one batched visual round; batch fixes and allow one confirmation round. Record actual pixels versus logical points. Native egui is outside the HTML/CSS detector's scope. Do not claim screen-reader, every monitor configuration or fabric quality from framebuffer captures.
