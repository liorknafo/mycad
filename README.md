# MyCad

A browser-based 3D parametric CAD application written entirely in Rust.

MyCad targets a SolidWorks / Fusion 360-style modeling workflow — draw a 2D
sketch on a plane, extrude it into a solid, edit the parameters and watch the
model rebuild. It runs both natively on desktop and in the browser via WebGPU.

- **Kernel** — custom geometry kernel with B-Rep topology, a 2D constraint
  solver, feature operations and tessellation
- **Renderer** — [`wgpu`](https://github.com/gfx-rs/wgpu) (WebGPU in the browser,
  Vulkan/Metal/DX12 natively)
- **UI** — [`egui`](https://github.com/emilk/egui) immediate-mode GUI via
  [`eframe`](https://github.com/emilk/egui/tree/master/crates/eframe)

## Current Status

Phase 1 (foundation & MVP) is largely working end-to-end:

- 2D sketching: point, line, rectangle, circle, arc tools with snap-to-grid
  and snap-to-geometry
- Constraint solver: Gauss-Newton with coincident, horizontal, vertical and
  distance constraints, DOF tracking, color-coded constraint visualization
- B-Rep model: half-edge topology (vertex → edge → wire → face → shell →
  solid) with stable IDs
- Extrude: closed wire profile → 3D solid with bottom, top and side faces
- Tessellation: ear-clipping triangulation, per-vertex normals, edge extraction
- 3D viewport: arcball camera, perspective/orthographic projection, standard
  views, grid, axes, Phong-shaded mesh with wireframe overlay
- UI: feature tree, property panel, toolbar, constraint visualization

Not yet implemented: revolve, boolean, fillet/chamfer, file import/export
(STL, STEP, OBJ, DXF, glTF), GPU picking, undo/redo, save/load. See
[`PLAN.md`](PLAN.md) for the full roadmap.

## Build & Run

### Prerequisites

```bash
rustup toolchain install stable
rustup target add wasm32-unknown-unknown   # only needed for the browser build
```

### Native (desktop)

```bash
cargo run -p mycad-app
```

### WASM (browser)

Install [trunk](https://trunkrs.dev/):

```bash
cargo install trunk
```

Serve locally from the `mycad-app` crate:

```bash
cd crates/mycad-app
trunk serve
```

Then open `http://127.0.0.1:8080` in a WebGPU-capable browser (Chrome/Edge 113+
or Firefox Nightly with WebGPU enabled).

## Development

Build the whole workspace:

```bash
cargo build --workspace
```

Run all tests:

```bash
cargo test --workspace
```

Lint with clippy:

```bash
cargo clippy --workspace --all-targets -- -D warnings
```

Format:

```bash
cargo fmt --all
```

## Project Structure

```
crates/
├── mycad-kernel/     Math, B-Rep, sketch + solver, features, tessellation
├── mycad-renderer/   wgpu rendering, arcball camera, grid/axes/mesh/overlays
├── mycad-ui/         egui panels, toolbar, sketch mode, selection
└── mycad-app/        eframe entry point, document state, command glue
```

The crates are layered: `kernel` depends on nothing in this workspace,
`renderer` depends on `kernel`, `ui` depends on `kernel` + `renderer`, and
`app` ties them all together.

See [`PLAN.md`](PLAN.md) for the phased roadmap and
[`CLAUDE.md`](CLAUDE.md) for developer-oriented architecture notes.

## License

TBD.
