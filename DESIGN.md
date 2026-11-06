---
name: "EmbroideryLab workstation"
description: "A native embroidery control room with neutral artwork, precise tools and recoverable operations."
colors:
  canvas: "#16191D"
  paper: "#F7F7F7"
  paper-grid: "#E0E0E0"
  grid: "#2A2F35"
  surface: "#202429"
  panel: "#1C2025"
  elevated: "#2B3138"
  border: "#424A54"
  text: "#E3E8EE"
  muted: "#A3ADB9"
  accent: "#55C5D8"
  selection: "#193F49"
  warning: "#E4A166"
  error: "#ED817B"
  success: "#83BC9B"
  handle: "#D7E2EC"
  light-canvas: "#EDF0F3"
  light-grid: "#D2D8DE"
  light-surface: "#F8FAFB"
  light-panel: "#E6EAEE"
  light-elevated: "#FFFFFF"
  light-border: "#B0BBC6"
  light-text: "#202A34"
  light-muted: "#536272"
  light-accent: "#006B82"
  light-selection: "#D3EAF0"
  light-warning: "#97551D"
  light-error: "#AE3739"
  light-success: "#286441"
  light-handle: "#334E68"
  action-ink: "#0C1C20"
typography:
  title:
    fontFamily: "egui bundled proportional"
    fontSize: "16px"
  body:
    fontFamily: "egui bundled proportional"
    fontSize: "13px"
  button:
    fontFamily: "egui bundled proportional"
    fontSize: "13px"
  small:
    fontFamily: "egui bundled proportional"
    fontSize: "11px"
  mono:
    fontFamily: "egui bundled monospace"
    fontSize: "13px"
  canvas-label:
    fontFamily: "egui bundled proportional"
    fontSize: "12px"
  ruler:
    fontFamily: "egui bundled proportional"
    fontSize: "10px"
rounded:
  square: "0px"
  small: "2px"
  layer: "3px"
  control: "4px"
spacing:
  layer-gap: "4px"
  compact: "6px"
  item: "10px"
  surface-inset: "12px"
  section: "16px"
components:
  button-primary:
    backgroundColor: "{colors.accent}"
    textColor: "{colors.action-ink}"
    typography: "{typography.button}"
    rounded: "{rounded.control}"
    padding: "6px 10px"
    height: "28px"
  button-generate:
    backgroundColor: "{colors.accent}"
    textColor: "{colors.canvas}"
    typography: "{typography.button}"
    rounded: "{rounded.control}"
    padding: "6px 10px"
    height: "28px"
  button-secondary:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.text}"
    typography: "{typography.button}"
    rounded: "{rounded.control}"
    padding: "6px 10px"
    height: "28px"
  button-secondary-hover:
    backgroundColor: "{colors.elevated}"
    textColor: "{colors.text}"
    rounded: "{rounded.control}"
  tool-selected:
    backgroundColor: "{colors.selection}"
    textColor: "{colors.accent}"
    rounded: "{rounded.control}"
    size: "28px"
  parameter-field:
    backgroundColor: "{colors.canvas}"
    textColor: "{colors.text}"
    typography: "{typography.body}"
    rounded: "{rounded.control}"
  work-panel:
    backgroundColor: "{colors.panel}"
    textColor: "{colors.text}"
    rounded: "{rounded.square}"
    padding: "10px"
  layer-selected:
    backgroundColor: "{colors.selection}"
    textColor: "{colors.text}"
    rounded: "{rounded.layer}"
  artwork-plate:
    backgroundColor: "{colors.paper}"
    rounded: "{rounded.square}"
  canvas-label:
    backgroundColor: "{colors.paper}"
    textColor: "{colors.light-success}"
    typography: "{typography.canvas-label}"
    rounded: "{rounded.small}"
    padding: "3px"
  command-scrubber:
    backgroundColor: "{colors.elevated}"
    rounded: "{rounded.small}"
    height: "22px"
---

# Design System: EmbroideryLab workstation

## Overview

**Creative North Star: "Embroidery control room"**

A precise native Windows workstation keeps one embroidery canvas at the center of import, cleanup, editable geometry, stitch inspection and export. Charcoal and graphite chrome is the default; the secondary light theme keeps the same controls and semantic roles. Restrained cyan identifies active tools and focus, while warm orange calls out warnings and work awaiting regeneration.

The color-grading control-room direction is implemented through shallow action rows, a compact tool rail, resizable side panels and an expandable command strip. Neutral artwork paper and unchanged document RGB protect color comparison. Authored painter geometry supplies icons; quiet native state feedback replaces decorative motion. The user authorized this code-first Rust/egui/OpenGL redesign; the light-only incumbent record is archived in outputs/autodigitize/redesign/design-before.md and design-before.json.

**Key Characteristics:**

- One dominant canvas with independently collapsible supporting panels.
- Physical units, actual sewing order and current command data remain explicit.
- Dark-first neutral chrome with a matching secondary light theme.
- Small control corners, flat panels and authored line icons.
- Errors, cancellation and stale previews retain recoverable work.

The frontmatter records implemented tokens, not planned values. Its px lengths serialize egui logical points; actual device pixels follow native scaling. Unprefixed chrome colors are dark-theme values, light-prefixed colors are their counterparts, and paper / paper-grid are invariant. Reused rules come from ui_theme.rs, workspace_ui.rs, layer_panels.rs, sequence_panel.rs, cleanup_ui.rs and workspace_preferences.rs. Local exceptions remain explicit below.

The sidecar extends these tokens with native source references, state rules and documentation-only component previews. Its HTML/CSS snippets translate native patterns for the design panel; they are not the application frontend, new native components or runtime assets. Derived tonal strips visualize color families and add no runtime tokens. Validation and visual acceptance belong to review reports and capture manifests, separate from this system record.

## Colors

Neutral dark surfaces recede around color-faithful artwork; cyan is an operational accent and orange is a warning signal. The light theme preserves those roles with darker readable ink.

### Primary

- **Active cyan** (accent / light-accent): active tools, selection outlines, keyboard focus, cursor, links and primary actions.
- **Selection well** (selection / light-selection): selected layer rows and active controls; it supports the accent without replacing artwork color.

### Neutral

- **Canvas surround** (canvas / light-canvas): central working area, text-field interiors and cleanup surround.
- **Graphite panel** (panel / light-panel): Objects, Inspector and quiet noninteractive surfaces.
- **Control surface** (surface / light-surface): header, tool rail, view bar, command panel, windows and resting buttons.
- **Raised tone** (elevated / light-elevated): hovered/open controls and empty command track; a tonal step without a drop shadow.
- **Structure line** (border / light-border): fine control boundaries.
- **Main ink** (text / light-text) and **muted ink** (muted / light-muted): control content and supporting labels. The native inactive slider rail also uses muted ink.
- **Handle ink** (handle / light-handle): editing handles within local canvas treatment.
- **Invariant paper** (paper) and **paper grid** (paper-grid): geometry-bounds artwork plate, unchanged by theme. Outside it, lines use theme-specific grid ink.
- **Primary action ink** (action-ink): dark header Auto Digitize foreground. The light header uses white. Cleanup Generate uses dark canvas or light elevated foreground instead.

### Semantic feedback

- **Warm warning** (warning / light-warning): stale preview, pending refinement, cautions and the unsaved asterisk beside the project name.
- **Error coral** (error / light-error): failed operations and recoverable detail.
- **Success green** (success / light-success): accepted geometry previews and direction/start marks where applicable.

Cleanup overlays are task diagnostics: red marks excluded pixels, blue current selection and green protected pixels, with a labeled legend. Their source-dependent compositing remains in cleanup_ui.rs; legend swatches do not redefine artwork RGB.

**The Data Color Rule.** Theme changes affect chrome, never artwork RGB, thread assignments or machine commands. Imported DST and DSB preview colors remain illustrative.

**The Paper Ink Rule.** Editing marks on invariant artwork paper use artwork_ink(), which returns the light-theme ink set in either chrome theme. Contrast labels use opaque local backgrounds.

## Typography

**Body and control font:** egui's bundled proportional family. **Technical font:** egui's bundled monospace family where the Monospace role is used. The build does not install a replacement system or display face.

The hierarchy is compact and work-oriented. The frontmatter gives the implemented role sizes: title/Heading for section identity; Body and Button for controls; Small for supporting information; Monospace for existing technical content. Explicit canvas labels use canvas-label, with smaller rung numbers and ruler ticks where source requests them. Strong text emphasizes project and section identity without inventing a font-weight scale. Line height and letter spacing remain egui behavior.

Units stay beside values: millimetres, degrees and commands per second. Russian and English share the hierarchy; longer translations wrap, scroll, compact or truncate only where the native component implements it. User project/object names remain data, with full names available through tooltips.

## Layout

A shallow project/action bar spans the top. The fixed tool rail is 44 logical points wide. Objects sits left of the canvas and the contextual Inspector right. Vector / Stitches / 3D and view controls share the canvas's upper edge. The bottom summary expands to the real command strip, position, speed and feedback. This is a resizable native desktop layout.

Objects starts at 228 points and permits 180–380; Inspector starts at 278 and permits 240–420. Both collapse independently and scroll. Consecutive same-color runs group visually without changing sewing order or merging separated runs. Stitch inspection starts at 105, resizes through 88–240, and collapses to 58. Dimensions and visibility persist as workspace preferences outside ELAB; layout reset keeps the theme. Locks are workspace/session editing guards, not machine instructions.

The initial viewport is 1360 × 900 logical points; the declared minimum is 850 × 480. Width below 1100 compacts Workspace to the authored settings icon. Layer kind labels depend on row width. Keep native panel constraints, scrolling and independent collapse; there is no mobile breakpoint system. Capture manifests distinguish logical size from actual pixels and scale; their existence does not certify every content length at every size.

The compact/item/section rhythm supports dense work, with tighter layer gaps and a surface inset. Item spacing is horizontal item × vertical compact; button padding uses the same pair. The interaction-size setting is 28 × 28 points, a minimum setting rather than a fixed size for every widget. Header margins are 12 × 8, Objects inset 10 and Inspector inset 12. The parameter grid uses 12 × 6 spacing.

Auto Digitize width follows available width minus 48, bounded to 240–1120 points. Its body scrolls; errors and wrapping Generate / Preview regions / Cancel remain outside that body. Source-preview height is bounded to 160–420. Cleanup Fit-relative zoom spans 25–1600%; wheel zoom follows the pointer and middle drag pans. The transform freezes during a left-button cleanup gesture to retain source-pixel mapping.

**The Measurement Rule.** The grid represents 10 mm in project coordinates and follows zoom and pan. Omit it below a 12-point screen interval; do not replace it with a decorative screen-fixed grid.

Fit design uses actual geometry/stitch bounds. Main-canvas wheel zoom follows the pointer; middle drag, Pan or held Space pans. Optional rulers and hints are presentation choices. Geometry, handles, source overlay, stitches, jumps and diagnostics share the coordinate transform.

## Elevation & Depth

Chrome is flat and tonal. Square boundaries, fine strokes and local fills distinguish regions. Window and popup shadows are explicitly NONE. Native state animation time is 0.15 seconds. Repaint follows interaction, active work or playback; idle animation is absent. Generation uses an opaque native modal, disabled fade-in, real stages and cancellation.

The 3D viewport renders the actual sequence with lit curved thread geometry, depth testing and a fabric plane. Lighting and approximate overlap belong to the embroidery preview, not interface elevation. Thread diameter is view-only; partial playback withholds the future full-design shadow. This is geometric preview, not fabric simulation or sew-out approval.

**The State Before Export Rule.** Geometry and stitch-setting changes stop playback and withhold stale stitches. Export requires a current successful plan; failure keeps editable work available.

## Shapes

Panels, canvas and artwork paper are square. Controls/windows/menus use the control radius; selected layer fills use the layer radius; swatches, local badges and command tracks use the small radius. The plate follows visible geometry bounds with an 18-point surround, preserving even-odd holes and fill RGB. Failed or over-budget fill construction retains outlines instead of inventing occupied regions.

The shared icon inventory contains 31 authored 24-unit painter drawings at an 18-point optical size. Stroke scales from 1.8 units with a 1.1-point minimum. Reuse its geometry for menus and tools, including Add, with accessible labels and tooltips. Never substitute font glyphs or runtime bitmaps.

Canvas strokes stay fine: grid 0.5, sewn lines 0.8, jumps 0.7, selected outlines 1.7 and idle outlines 0.8 points. Square polygon nodes and circular Running/Bezier handles retain different editing meanings. Opaque canvas-label backgrounds expand three points around text and separate direction/start labels from arbitrary artwork.

## Components

### Buttons and icon controls

Compact native controls use secondary surface at rest, raised tone on hover and selection fill with cyan stroke when active. Hover boundaries use muted ink; active strokes use 1.5 points. Open controls keep raised tone with cyan boundary. Icon controls and labeled actions carry a 1.5-point inner focus outline, tooltip and accessible label. Disabled controls use native 0.5 alpha and disabled help.

Auto Digitize is the primary header action; Generate embroidery commits cleanup. Their accent fills are scoped locally with distinct foreground assignments. Labeled actions measure text then add an 18-point icon, compact gap and item padding. Native buttons use weak_bg_fill for their resting surface; inactive bg_fill supplies the visible muted slider rail instead.

### Fields, sliders and disclosures

Text fields, DragValues, combos, checkboxes and radios retain native semantics. Fields use canvas interiors and cyan selection/cursor; sliders have a visible three-point rail and no trailing fill. Numeric limits, units and precision come from the real parameter. Text-edit focus owns its history and selection; focused buttons do not block shortcuts because retained text state exists.

Inspector groups stitch fields, underlay, geometry, machine profile, quality and artwork in scrollable native disclosures. Running paths retain length/ties/direction and hide irrelevant fill controls; paired Satin uses rail controls. Direction-rung lists stay bounded with fields for the selected item. Context menus share fields/history, scroll within the viewport and preserve unfinished drawing. Machine-only and lock guards remain visible.

The advanced Layer stitching disclosure separates project-wide hidden-fill
removal and millimetre registration overlap from machine limits. It uses the
same native checkbox, DragValue, history and stale-plan feedback as existing
properties. Editable contours and sewing order remain visible and unchanged;
only generated sewing domains are cut back.

### Navigation and workspace

The rail exposes Select/edit nodes, Pan, contour, Satin, cleanup, authored Add, Fit, grid and panels. Vector / Stitches / 3D changes presentation within the same workspace. Workspace controls theme, RU/EN, rulers/hints, panels and reset. Theme/view/panel changes do not rewrite the plan. The project name truncates locally; its warm unsaved asterisk remains outside truncation.

### Objects and sewing order

Rows combine visibility, unmodified thread swatch, selectable name, optional kind and lock. Top-to-bottom order is sewing order; stable identity survives grouping and reorder. Click selects one, Ctrl-click toggles, Shift-click selects ranges and Ctrl+A selects layers when appropriate. Earlier/Later are unavailable across locked neighbors. Visibility affects generated content; locks protect editing; group collapse is presentation only. Merge, split, branch conversion and smoothing remain explicit recoverable operations.

### Cleanup workspace

Fixed-seed tolerance drag is the signature: returning toward the press point narrows the mask. Shift adds, Alt subtracts, Escape cancels; Russian distinguishes selection Add from object creation. Connected/exterior/global modes, protect/restore brushes and labeled overlays keep choices inspectable. Bounded preview refines at source resolution after release. Exclude is unavailable when no unprotected selected pixel would change; no-op actions do not consume history.

The optional automatic choice selects Running / Satin / Tatami conservatively; unsupported shapes retain editable Tatami with an object-specific explanation. This does not promise arbitrary artwork recognition. Source, mask, regions and actual-stitch views share the cleanup viewport. Fit/zoom/pan reveal detail without altering masks. Cancel retains the document. Generate waits for gestures/refinement and commits through the transactional worker. Reimport preserves matched manual decisions unless explicitly reset; unmatched choices require recovery. PDF selects one rendered page; source artwork is embedded with editable ELAB projects.

Raster preview and tracing identify JPEG, PNG and WebP by file contents, so a
download with a misleading extension reaches the same protected workflow.
Existing JPEG/PNG source bytes remain intact; WebP embeds as lossless PNG for
ELAB reader compatibility, retaining RGBA and mask coordinates.

### Command strip and feedback

One 22-point command scrubber samples colors from the actual sequence. Its cyan position mark has a wider surface-colored backing. Click/drag pauses and inspects; focused arrow keys step commands. Numeric position and commands-per-second sit beside it. Play/Pause, Stop, counts and sewn width × height use real plan data. Jumps are travel, not trims or a quality score. Progress does not change full sewn extents.

Stale work shows warm regeneration feedback and disables playback/export until success. Collapsed errors point to expanded detail; expanded errors wrap in a bounded scroll. Quality warnings locate advisory findings without implying approval. The import profile remains labeled experimental.

### Protective decisions and file modes

Native Save / Discard / Cancel protects new/open/close; recovery stays separate from the last save. Pending decisions block background pointer actions and shortcuts from their first frame. Failure/cancellation retains work. Editable geometry/history/recovery remain separate from workspace preferences.

DST/DSB imports are stitch-only previews, not recovered vectors or exact thread colors. Editing, editable-project Save, regeneration and machine-profile changes remain unavailable. DSB preserves raw trim/stop/needle records and blocks DST conversion; export stays visibly experimental. Running extraction preserves hidden originals and explicit closed loops; Satin/branch previews remain source-bound. These are durable guards, not new embroidery algorithms from this redesign.

## Do's and Don'ts

### Do:

- Do use corresponding theme tokens for chrome and invariant paper ink for artwork editing marks.
- Do keep the canvas dominant and Objects, Inspector and Stitch inspection independently collapsible.
- Do keep physical units, sewing order, preview freshness and document mode explicit.
- Do give authored icon controls accessible labels, tooltips and visible keyboard focus.
- Do preserve real data, recoverable originals, manual edits and transactional cancellation.
- Do label synthetic artwork and retain raster provenance with evidence captures.

### Don't:

- Don't recolor artwork for the theme or treat thread RGB as interface palette tokens.
- Don't introduce ornamental gradients, glow, dashboard-card composition, idle animation or runtime bitmap icons.
- Don't substitute glyphs for authored icons or hide the dirty marker in a truncated filename.
- Don't present stale stitches as current, enable unsupported machine-only edits or discard DSB controls through DST conversion.
- Don't bypass Save / Discard / Cancel, reset unmatched manual decisions silently or apply geometry previews to another source.
- Don't imply vector reconstruction, exact imported thread colors, arbitrary junction/cycle-to-Satin conversion, optimal sewing order or physical sew-out validation.
- Don't describe native framebuffer review as web-detector, screen-reader, every-DPI or whole-application certification.
