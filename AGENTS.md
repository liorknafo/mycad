# AGENTS.md

Notes for Codex (and other AI assistants) working in this repository.
Read [`README.md`](README.md) for user-facing info and [`PLAN.md`](PLAN.md)
for the phased roadmap.

## Project at a glance

MyCad is a browser-based 3D parametric CAD app, 100% Rust, wgpu + egui.
A Cargo workspace with four crates under `crates/`:

| Crate | Role | Depends on |
|-------|------|------------|
| `mycad-kernel`   | Math, B-Rep, sketches, constraint solver, features, tessellation | (only external) |
| `mycad-renderer` | wgpu pipelines, arcball camera, mesh + overlay rendering | `mycad-kernel` |
| `mycad-ui`       | egui panels, toolbar, sketch mode, selection | `mycad-kernel` |
| `mycad-app`      | eframe entry point, document state, sketch session glue | all of the above |

Dependency rule: never add an upward edge (e.g. `kernel` must not depend
on `renderer`, `ui` must not depend on `app`). The kernel crate stays
pure and headless — no wgpu, no egui.

## Commands

```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
cargo run -p mycad-app                 # native
cd crates/mycad-app && trunk serve     # WASM
```

The WASM target is `wasm32-unknown-unknown`, built via trunk from
`crates/mycad-app`. The top-level `index.html` is just a redirect to
`crates/mycad-app/dist/index.html`.

## Kernel module map (`crates/mycad-kernel/src/`)

- `math.rs` — `Scalar = f64`, `Vec3 = DVec3`, `Mat4 = DMat4`, `Plane`,
  `CoordinateSystem`, plane ↔ 3D projection helpers.
- `brep.rs` — `BRepId` (stable `u64` identifiers) and `BRepModel` holding
  `Vertex`, `Edge`, `CoEdge`, `Wire`, `Face`, `Shell`, `Solid` in
  `HashMap<BRepId, _>`. Constructors validate topology
  (`create_wire_from_edges`, `create_shell_from_faces`,
  `create_solid_from_shells`). `CurveGeometry` and `SurfaceGeometry`
  are explicit enums (Line/Circle/Arc, Plane/Cylinder).
- `sketch.rs` — the biggest module. `Sketch` owns `entities` + `constraints`
  on a `Plane`. Entities are `Point`, `LineSegment`, `Circle`, `Arc`.
  Constraints: `Coincident`, `Horizontal`, `Vertical`, `Distance`.
  `Sketch::solve()` runs a Gauss-Newton iteration over the internal
  `SketchParameters` with a numerical Jacobian and returns a
  `SketchSolveResult` (status + DOF). `extract_closed_wire()` walks
  line endpoints to find a closed loop for extrusion (tolerance-based
  endpoint matching — see the `points_match` helper, uses 1e-3 not
  `EPSILON` because solved points drift).
- `features.rs` — `ExtrudeParams` / `ExtrudeResult` and `extrude(sketch,
  params)`. Revolve, boolean, fillet are stubs today.
- `tessellation.rs` — `Mesh { vertices, indices, normals }`,
  `tessellate_solid`, `tessellate_solid_with_edges`, `ear_clip_triangulation`.
- `export.rs` — stub for file I/O (STL/STEP/OBJ/DXF/glTF). Nothing
  implemented yet.

## Renderer module map (`crates/mycad-renderer/src/`)

- `lib.rs` — `Viewport3d` widget. Wraps an `ArcballCamera`, an optional
  `Mesh`, and sketch line overlay. Exposes `set_standard_view`,
  `fit_all`, `toggle_projection`, `set_mesh`, `screen_to_sketch_point`,
  and `ui(ui, sketch_mode) -> ViewportResponse`.
- `viewport.rs` — `ArcballCamera` with yaw/pitch/distance, orthographic
  and perspective projection, `StandardView` presets, `fit`, ray casting.
- `mesh.rs` — wgpu pipeline for the triangle mesh (per-vertex normals,
  Phong shading, indexed draw).
- `overlay.rs` — wgpu line-list pipeline used for grid, axes and sketch
  preview lines. `OverlayGeometry` builds the static grid + axis vertex
  buffer.
- `picking.rs` — stub (GPU id-buffer picking not yet wired up).

## UI module map (`crates/mycad-ui/src/`)

- `panels.rs` — `feature_tree_panel` (shows entities + DOF + constraints
  with satisfied/conflicting colors) and `property_panel`
  (edit geometry parameters for the current selection).
- `toolbar.rs` — tool selector (None / Line / Rectangle / Circle / Arc)
  plus the extrude / finish sketch buttons.
- `sketch_mode.rs` — `SelectableItem`, `SelectionState`,
  `pick_sketch_item(sketch, point, threshold)`. Multi-select with
  toggle semantics.

## App module map (`crates/mycad-app/src/`)

- `main.rs` — eframe entry point for both native (`eframe::run_native`)
  and wasm (`eframe::WebRunner`). Fatal-at-startup `.expect()` calls
  here are intentional.
- `lib.rs` — the big one. `MyCadApp` implements `eframe::App`.
  `SketchSession` owns the current `Sketch`, the active `SketchTool`
  and tool state (line_start, rect_start, circle_center, arc tracking).
  Snap logic is layered: first snap to existing points, then to lines,
  then to the grid (`snap_point`). Extrusion runs
  `extract_closed_wire → extrude → tessellate_solid_with_edges` and
  hands the mesh to the viewport.

## Conventions & gotchas

- `Scalar` is `f64`. Geometry uses `glam::DVec2` / `DVec3` / `DMat4`;
  wgpu vertex buffers drop to `f32` at the boundary in `mesh.rs` and
  `overlay.rs`. If you see `as f32` it's almost always that conversion.
- Floating-point comparisons: the module-level `EPSILON` is tight
  (~1e-9). In places where values come out of the solver (e.g. closed
  wire extraction in `sketch.rs`), use a looser tolerance like `1e-3`
  instead, because solved parameters drift.
- B-Rep IDs are stable `u64`s, not pointers or indices. Never assume
  IDs are contiguous or ordered. Look up via `BRepModel`'s `HashMap`s.
- `extract_closed_wire` currently only handles line entities. Arcs and
  circles aren't wired into the extrusion pipeline yet.
- Tests live inline as `#[cfg(test)] mod tests` at the bottom of each
  kernel module. There are also doctests (see `features::extrude`).
- The top-level `.hier`, `.pi`, `.cargo`, `dist/` and `target/` folders
  are tool state — leave them alone.

## Working on this code

- Start with `cargo check -p <crate>` for fast feedback on the crate
  you're touching; the workspace check is fast enough (~2s incremental)
  but per-crate is faster.
- Before declaring a change done, run `cargo clippy --workspace
  --all-targets -- -D warnings` and `cargo test --workspace`. The
  repo currently builds clippy-clean with `-D warnings`; keep it that
  way.
- Prefer growing kernel tests in-file rather than adding a new
  `tests/` directory — that's the existing pattern.
- If you add a new constraint kind, you need to touch: the
  `SketchConstraintKind` enum, `SketchParameters` (solver residuals
  + Jacobian hookup), the UI display code in `panels.rs`, and likely
  the toolbar. Don't forget a unit test exercising the solver.
- If you add a new feature operation, mirror `extrude`: a `*Params`
  input struct, a `*Result` output struct holding the mutated
  `BRepModel` and the IDs of the faces you created, and a top-level
  `pub fn` in `features.rs`.

## Known gaps (not bugs, just unfinished)

These are in `PLAN.md` but worth calling out since they shape most
"why doesn't X work?" questions:

- No undo/redo, no save/load, no STL or other export.
- Revolve, boolean, fillet, chamfer, shell, draft, pattern, mirror —
  all stubbed.
- GPU picking is a stub; current selection goes through CPU-side
  `pick_sketch_item` on sketch coordinates.
- Sketches don't persist across sessions.
- Constraint catalog is intentionally minimal (4 kinds). Tangent,
  equal, parallel, perpendicular, concentric, symmetric, fix etc.
  are future work.

## Things to avoid

- Don't reintroduce `.clone()` on `Copy` types (e.g. `SketchConstraintKind`
  is `Copy`).
- Don't add a `kernel → renderer` or `kernel → ui` dependency. The
  kernel must stay headless so it's testable without a GPU.
- Don't reach for `unwrap()` / `expect()` in kernel code paths that
  can be reached at runtime. Return `Result<_, KernelError>` instead.
  Exceptions: WASM startup code in `app/main.rs`, test code, and the
  few `expect()`s that document genuine invariants.
- Don't bake feature-specific logic into `MyCadApp` directly when it
  can live in the kernel — keep `app` as glue.
