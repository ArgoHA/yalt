# yalt implementation plan

> `yalt` (yet another labeling tool) is the selected public styling. A September
> 2026 check found an inactive `vahuir/YALT` project using the same expansion,
> but `ArgoHA/yalt` was available. Earlier version labels in this plan were
> private milestones; the first public release starts at `0.1.0`.

## Product decisions

- This is a clean implementation. No make-sense source code will be copied.
- The project is open source under Apache License 2.0.
- macOS on Apple Silicon is the initial platform.
- A project has exactly one task: object detection, polygon segmentation, or
  image classification.
- Classification has a single-label mode and a multi-label mode.
- New project data lives in `<dataset>/.yalt/project.sqlite`; private-preview
  `.labeler` projects remain readable. Images stay in the dataset and are never
  uploaded.
- JPEG, PNG, WebP, and TIFF are supported. TIFF is decoded natively and sent to
  the macOS WebView as PNG for consistent display. HEIC was evaluated and is
  deferred because the current bundled decoder does not support it reliably.
- Left and right arrows change images. Trackpad scroll and Space-drag pan the
  canvas. Pinch zooms around the pointer.
- Class shortcuts are visible beside their names: for example `person  1`.
  Pressing `1` selects `person`. Internal class IDs remain stable and are not
  coupled to shortcuts or YOLO's zero-based export index.
- Deleting an image means moving it into a project-level `deleted/` directory
  and recording the move transactionally. It is recoverable until the user
  explicitly empties that directory.

## Storage model

SQLite is the source of truth; YOLO, COCO, CSV, and class folders are exchange
formats. The first schema contains:

- project identity, task type, classification mode, and schema metadata;
- stable classes with order, color, and visible keyboard shortcut;
- indexed images with relative paths, dimensions, file metadata, and status;
- rectangle and polygon annotations;
- single- and multi-label image classifications;
- recoverable in-progress annotation drafts;
- project settings and an operation history for undo/redo.

Writes use transactions, foreign keys, WAL journaling, a busy timeout, schema
migrations, and an integrity check on open. Image coordinates are stored in
original pixel space and normalized only at import/export boundaries.

## Interaction contract

### Detection

- The active class persists between images and app sessions.
- Press-drag-release creates and saves a box; a click or negligible drag creates
  nothing and clears the selection.
- The selected box shows eight resize handles. Hovering another box reveals
  the same controls; its separate top handle moves it.
- Box bodies select but do not move. Command-drag bypasses hover editing and
  always starts a new box, including from inside an existing box.
- Boxes can also be relabeled, hidden, and deleted.
- YOLO TXT plus `labels.txt`, and COCO bounding boxes, import and export.

### Polygon segmentation

- Click to add vertices; click the first point or press Enter to finish.
- Backspace removes the last draft vertex and Escape cancels the draft.
- Completed polygons support vertex editing and relabeling.
- `Add island` (`I`) starts another contour on the selected object. All of its
  contours remain one instance in SQLite and COCO.
- COCO polygon arrays import and export. COCO RLE is not part of the first
  release.
- YOLO segmentation TXT export writes one normalized row per contour. Because
  plain YOLO TXT has no shared-instance identifier, separate islands cannot
  remain one instance in that exchange format.

### Classification

- A class click or shortcut is committed before advancing to the next image.
- Single-label mode can move an image into the directory named after its class.
- Changing a label moves it from the old class directory to the new one.
- Multi-label mode does not move files; all images remain together and export
  as `relative_path | class_name_1,class_name_2,...` CSV.
- Folder operations and their database changes are designed as one recoverable
  command so a failed move cannot silently desynchronize the project.

### Shared shortcuts

- `1`–`9`, then `0`: select the visible class shortcut.
- `[` / `]`: previous/next class.
- Left/right arrow: previous/next image.
- Space-drag or two-finger scroll: pan.
- Pinch and Command-plus/minus: zoom; `F`: fit image.
- `R`: rectangle, `P`: polygon, `E`: edit existing annotations.
- Command-Z / Shift-Command-Z: undo/redo.
- Delete/Backspace: delete the selected annotation, then the last remaining
  annotation; neither key deletes an image.
- `D D`: move the current image to the recoverable `deleted/` directory, but
  only after all of its spatial annotations have been removed.

## Visual direction

The app is a compact, dark macOS workbench rather than a web dashboard. The
palette is graphite (`#17191C`, `#202328`, `#30353B`) with high-contrast text
(`#F2F4F5`, `#9DA6AE`). Violet (`#9B6CFF`) is the first annotation color and
active accent. SF Pro/system typography keeps dense controls familiar.

The memorable element is the class color carried consistently through its
shortcut, cursor, geometry, and list state. There are no decorative gradients
or collections of generic cards.

## Phases

### 1. Foundation — done

- Tauri 2 + React/TypeScript application shell.
- Project create/open flow and recent-project list.
- Project-local SQLite schema, migrations, WAL, integrity checks, and migration
  backup hook.
- Recursive JPEG/PNG indexing with missing-file preservation.
- Lazy, disk-cached thumbnails.
- Provisional project workspace and rescan action.
- Apple Silicon app/DMG bundle configuration and signing-ready release workflow.
- Automated Rust tests for database creation, indexing, and reopen/recovery.

### 2. Canvas and navigation — done

- Retina-aware layered canvas renderer and a tested, shared viewport transform.
- EXIF-oriented local image decoding, virtualized image list, and a three-image
  decoded-file cache for current/neighbor prefetching.
- Trackpad scroll-to-pan and pointer-anchored pinch-to-zoom event handling,
  verified on the target Mac.
- Geometry hit-testing primitives, tool selection, shortcut routing, and
  persistent per-image viewport/current-image state.
- Undo/redo command history and crash-recoverable image moves into `deleted/`,
  including explicit restore.
- Benchmarks completed with 10,000 files and a 24 MP image.

### 3. Detection — done

- Persistent class editor, stable spectrum colors, visible shortcut badges,
  and active-class memory.
- Click-click box creation, editing, relabeling, visibility, and deletion.
- YOLO detection import/export with `labels.txt`.
- COCO bbox import/export.
- Golden fixtures and coordinate round-trip tests.

### 4. Polygon segmentation — implemented; target verification pending

- Polygon creation, draft recovery, editing, and geometry warnings.
- COCO polygon import/export, including multiple polygons for one object.
- Golden fixtures and geometry round-trip tests.

### 5. Classification — implemented; target verification pending

- Single-label classify-and-advance workflow.
- Recoverable moves into class directories and conflict handling.
- Multi-label workflow without file movement.
- CSV and class-directory import/export with quoted paths and duplicate-name
  handling.

### 6. Product hardening and distribution — local hardening implemented; release pending

- Keyboard-accessibility review, accessible control labels, visible focus
  treatment, and locally persisted shortcut customization.
- Crash/forced-quit recovery tests for image and classification moves, plus
  malformed YOLO, COCO, and CSV import reporting.
- WebP and TIFF input support; HEIC evaluated and explicitly deferred.
- Apache-2.0 license decision and package metadata.
- `yalt` product name, icon, and `com.argoha.yalt` bundle identifier.
- Short product-workflow GIF for the GitHub README after the interaction flow
  is final.
- Apple signing/notarization, versioned GitHub release, and Homebrew tap cask.
- Main Homebrew Cask submission after the project meets its acceptance rules.

## Phase 1 acceptance record

Phase 1 was completed in the initial foundation implementation. The desktop
shell builds, Rust tests exercise project creation/index/reopen, the frontend
build passes, and release packaging is configured for `aarch64-apple-darwin`.
The initial shell used provisional branding that was replaced before the public
`0.1.0` candidate.

## Phase 2 acceptance record

Phase 2 was implemented with schema version 2. Existing Phase 1 databases are
backed up and migrated on open. Automated tests cover viewport transforms,
pointer-anchored zoom, geometry hit testing, EXIF rotation, viewport recovery,
schema migration, and delete/undo/redo/restore. The development benchmark
indexed 10,000 small images in about 1.13 seconds and created a cached thumbnail
for a 24 MP image in about 1.68 seconds on the development Mac. Trackpad zoom,
pan, recoverable deletion, automatic next-image navigation, and Command-Z were
subsequently verified in the packaged application on the target Mac.

A Phase 2 hotfix permits the app's local `blob:` image URLs in the
WebView content policy and reports decoding failures explicitly instead of
leaving the canvas on an indefinite loading indicator.

A second Phase 2 hotfix waits for both image decoding and a measured canvas
before restoring a viewport, repairs malformed viewport centers from the
earlier race, coalesces trackpad redraws to the display frame rate, and saves
viewport changes quietly after interaction becomes idle.
The delete → automatic next image → navigation → undo regression sequence was
verified after this fix, completing Phase 2 acceptance.

## Phase 3 acceptance record

Phase 3 is implemented with schema version 3. Detection
projects now have a persistent class editor with stable spectrum colors and
visible `1`–`9`, `0` shortcuts; the active class and an unfinished first box
corner survive relaunch. The canvas supports click-click creation plus box
selection, movement, corner resizing, relabeling, visibility, and deletion.
Every completed edit is written transactionally to SQLite and participates in
the same Command-Z / Shift-Command-Z history as image deletion.

`labels.txt` loading, YOLO folder import/export, and COCO bbox JSON
import/export are implemented. Imports replace boxes for matched images while
leaving unmatched images unchanged. Automated tests cover box geometry,
annotation persistence and undo/redo, draft recovery, golden YOLO and COCO
fixtures, and pixel-coordinate round trips. The refined detection workflow was
subsequently accepted on the target Mac, completing Phase 3.

The Phase 3 interaction refinement makes class selection immediately enter box
drawing mode and renames Select to the clearer Edit mode (`E`). Edit mode
reveals move and corner-resize targets on hover without requiring a preliminary
selection. The object list is now the primary right-panel content; class
creation, import/export, and shortcut help are disclosed only on request.
Both image and object panels can be collapsed independently, and the redundant
autosave explanation was removed from the annotation workspace.

## Phase 4 acceptance record

Phase 4 is implemented without a schema migration; the
existing annotation and draft tables already support polygon records. Polygon
projects now share the persistent class editor and minimalist Objects panel.
Clicking adds vertices, clicking the highlighted first vertex or pressing
Enter finishes a contour, Backspace removes the last draft vertex, and Escape
cancels the draft. Every vertex is autosaved and restored after relaunch.

Edit mode (`E`) discovers polygons directly under the pointer. A polygon can
be moved as one object or reshaped by dragging any visible vertex. The selected
object inspector reports vertex count, contour count, area, and compact
self-intersection or very-small-area warnings. Imported multi-contour COCO
objects stay grouped as one annotation. COCO polygon arrays import and export
with derived bounding boxes and area; RLE is reported as unsupported and
skipped. Automated tests cover draft recovery, annotation undo/redo, geometry
editing helpers, warnings, a golden multi-contour fixture, and COCO round-trip
behavior. Manual interaction acceptance on the target Mac remains.

A Phase 4 refinement adds contextual multi-island drawing for a selected
polygon and makes island completion plus draft removal one transaction. COCO
continues to store all islands as one object. YOLO segmentation export is now
available and explicitly emits one row per island, since standard TXT cannot
represent their shared instance identity. Detection and segmentation YOLO
exports ask for one new named export folder and omit per-image TXT files for
images without annotations, avoiding a target directory full of empty files.

A follow-up names that proposed export folder `labels-yolo`, making it clear
that it contains annotation TXT files rather than images.

## Phase 5 acceptance record

Phase 5 is implemented with schema version 4. Single-
label projects now move each classified image into the selected class directory
and advance immediately. The original relative path is retained, destination
name conflicts receive a stable image-ID suffix, and classification moves use a
project-local journal so interruption recovery and Command-Z / Shift-Command-Z
cover both the file and database assignment.

Multi-label projects toggle any number of classes without moving files. Both
modes provide persistent class management, visible number shortcuts, per-image
label state, labeled-image progress, quoted CSV import/export, and copy-based
class-directory export. Single-label projects also import existing class
directories. Exact relative paths are preferred during import; ambiguous
duplicate file names are skipped instead of being guessed. Automated tests
cover single-label move/undo/redo, multi-label no-move behavior, quoted CSV
paths, and destination-name conflicts. Manual interaction acceptance on the
target Mac remains.

A Phase 5 hotfix accepts Finder folder drops anywhere on the New Project
sheet and fills the shared image-folder path for every task type. It also keeps
the initial-image restore from rerunning after a classification save, so a
single-label class click or number shortcut remains advanced to the next image.

A Phase 5 refinement replaces the duplicated classification class manager
with one label list and inline maintenance controls. Undo and redo now reveal
the image affected by the history entry for detection, segmentation,
classification, and image moves. The image status bar can copy the full image
path, and the recent-project screen is a three- or four-column contact sheet
using the first active image in each project as its preview.

## Phase 6 local hardening record

The public `0.1.0` candidate adds WebP and TIFF indexing/decoding, with TIFF
converted to PNG only for WebView display while source files stay untouched.
HEIC remains unsupported until a reliable bundled decoder is selected. YOLO
polygon import now complements YOLO polygon export and matches D-FINE-seg's
normalized segmentation rows; each row imports as one instance because plain
YOLO TXT carries no shared-island identity.

Keyboard-only operation was reviewed across the project sheet, image list,
toolbar, canvas, object controls, and classification controls. Tool, fit,
island, and class-navigation keys can be customized from the project menu and
are persisted on the Mac; number keys remain reserved for class selection and
standard macOS undo/redo and zoom chords remain fixed. Recovery tests now
simulate forced termination after filesystem moves but before SQLite commits
for both deletion and single-label classification. Malformed YOLO rows, COCO
objects, and unterminated classification CSV fields have regression coverage
and produce user-visible reports.

Apache-2.0 is assigned in `LICENSE` and package metadata. `yalt` branding, the
app icon, and the `com.argoha.yalt` bundle identifier are applied. No signing,
notarization, GitHub release, Homebrew tap, or public source publication has
been performed.

The current detection interaction replaces click-click creation with
press-drag-release. New and hovered boxes expose eight resize handles plus a
separate top move handle; dragging the body itself is intentionally disabled.
Command-drag bypasses an existing box so overlapping annotations can share a
starting point. A click without sufficient movement only clears the selection.
Backspace now walks backward through the remaining objects and never deletes
the image. Image deletion uses the reserved `D D` sequence and is blocked until
all detection or segmentation objects on that image have been removed.
