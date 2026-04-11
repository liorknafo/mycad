# MyCad — Project Plan

Browser-based 3D parametric CAD application. Full Rust (WASM), wgpu rendering, egui UI.
Workflow: Sketch → Extrude/Revolve → Boolean → Assembly.

## Architecture

```
┌─────────────────────────────────────────────────┐
│                   Browser (WASM)                 │
├─────────────────────────────────────────────────┤
│  UI Layer (egui)                                │
│  ├─ Toolbar, menus, property panels             │
│  ├─ Feature tree / model browser                │
│  ├─ Sketch mode overlay                         │
│  └─ Viewport controls                           │
├─────────────────────────────────────────────────┤
│  Application Layer                              │
│  ├─ Command system (undo/redo)                  │
│  ├─ Selection manager                           │
│  ├─ Document model (parametric feature tree)    │
│  └─ Event bus                                   │
├─────────────────────────────────────────────────┤
│  Geometry Kernel                                │
│  ├─ B-Rep (Boundary Representation)             │
│  ├─ Sketch solver (2D constraint system)        │
│  ├─ Feature operations (extrude, revolve, bool) │
│  ├─ Tessellation (B-Rep → triangle mesh)        │
│  └─ NURBS/curve math                            │
├─────────────────────────────────────────────────┤
│  Render Engine (wgpu)                           │
│  ├─ 3D mesh rendering (Phong shading)           │
│  ├─ Edge/wireframe overlay                      │
│  ├─ Sketch 2D rendering                         │
│  ├─ Grid, axes, gizmos                          │
│  └─ Selection highlighting / picking            │
└─────────────────────────────────────────────────┘
```

## Crates

| Crate | Purpose |
|-------|---------|
| `mycad-kernel` | Math, geometry, B-Rep, sketch solver, feature ops, tessellation |
| `mycad-renderer` | wgpu rendering, camera, picking, overlays |
| `mycad-ui` | egui interface, panels, tools, sketch mode |
| `mycad-app` | Document model, command system, selection, glue |

## Technology Stack

| Layer | Technology |
|-------|-----------|
| Language | Rust (100%) |
| UI | egui via eframe |
| Rendering | wgpu (WebGPU in browser, native for dev) |
| Math | glam (fast vectors/matrices) + nalgebra (solver) |
| Serialization | serde + serde_json |
| WASM | wasm32-unknown-unknown via trunk |
| Topology graph | petgraph |
| Reference code | ../FreeCAD/src/Mod/Sketcher/App/planegcs/ |

## Phase 1 — Foundation & MVP

Goal: Draw a rectangle sketch → extrude into a cube → edit dimensions → export STL.

| # | Step | Crate |
|---|------|-------|
| 1 | Project scaffolding — Rust workspace, wasm build pipeline, HTML shell | all |
| 2 | Math primitives — Vec2, Vec3, Mat4, Plane, Transform (via glam) | kernel |
| 3 | Basic wgpu renderer — canvas init, clear color, orbit camera, grid, axes | renderer |
| 4 | egui integration — eframe + wgpu, layout with menu bar, panels, viewport | ui |
| 5 | 2D sketch entities — Point, Line, Arc, Circle with operations | kernel |
| 6 | Sketch constraint solver — Newton-Raphson, coincident, horizontal, vertical, distance, parallel, perpendicular, angle, DOF tracking | kernel |
| 7 | Sketch UI mode — enter sketch on plane, draw lines, snapping, constraint display, color by state | ui + renderer |
| 8 | Rectangle tool — two-click rectangle with auto-constraints | ui + kernel |
| 9 | B-Rep data structures — Vertex, Edge, Wire, Face, Shell, Solid, half-edge adjacency, stable IDs | kernel |
| 10 | Extrude operation — closed wire → B-Rep solid along normal | kernel |
| 11 | Tessellation — B-Rep faces → triangle mesh, normals, edge extraction | kernel |
| 12 | 3D mesh rendering — Phong shading, edge overlay, face coloring | renderer |
| 13 | GPU picking — entity IDs to offscreen buffer, click selection | renderer |
| 14 | Selection system — click select, highlight, mode switching, multi-select | app + renderer |
| 15 | Feature tree — sketch + extrude entries, click to select, right-click edit | ui + app |
| 16 | Command / undo system — undoable commands, Ctrl+Z/Y | app |
| 17 | Property panel — selected feature params, edit to trigger rebuild | ui + app |
| 18 | Document save/load — serialize feature tree to JSON, browser file API | app |
| 19 | STL export — tessellate → binary STL → file download | kernel + app |
| 20 | Integration testing — end-to-end: sketch → extrude → modify → undo → export | all |

## Phase 2 — Core Modeling

| # | Step | Crate |
|---|------|-------|
| 21 | Additional sketch tools — arc, circle, polygon, spline, trim, offset, mirror | kernel + ui |
| 22 | Additional sketch constraints — tangent, equal, symmetric, concentric, midpoint, fix | kernel |
| 23 | Revolve operation | kernel |
| 24 | Boolean operations — union, subtract, intersect on B-Rep | kernel |
| 25 | Fillet — rolling-ball fillet on edges | kernel |
| 26 | Chamfer — distance/angle chamfer on edges | kernel |
| 27 | Multi-body support | app + kernel |
| 28 | Extrude enhancements — cut, up-to-face, symmetric, draft angle | kernel |
| 29 | Section view — clip plane, cross-section rendering | renderer |

## Phase 3 — Advanced Features

| # | Step | Crate |
|---|------|-------|
| 30 | Shell operation | kernel |
| 31 | Draft operation | kernel |
| 32 | Linear & circular pattern | kernel + app |
| 33 | Mirror feature | kernel + app |
| 34 | STEP import/export | kernel |
| 35 | IGES import/export | kernel |
| 36 | DXF import/export | kernel |
| 37 | OBJ & glTF export | kernel |
| 38 | Measurement tools — distance, angle, area, volume | ui + kernel |
| 39 | Dimension-driven input — click + type value | ui |

## Phase 4 — Assembly & Polish

| # | Step | Crate |
|---|------|-------|
| 40 | Assembly document — components, instances, transforms | app |
| 41 | Assembly mates — coincident, concentric, distance, angle, fixed | kernel + app |
| 42 | Assembly constraint solver | kernel |
| 43 | Interference detection | kernel |
| 44 | Bill of materials | app + ui |
| 45 | View cube widget | renderer + ui |
| 46 | Appearance system — materials, transparency per face/body | renderer + app |
| 47 | Dark/light theme | ui |
| 48 | Performance — frustum culling, LOD, incremental tessellation, spatial index | renderer + kernel |
| 49 | Keyboard shortcut customization | ui + app |
| 50 | Command palette (Ctrl+Shift+P) | ui |

## FreeCAD Reference

Key source files in `../FreeCAD/` for each area:

| Area | Files |
|------|-------|
| Constraint solver | `src/Mod/Sketcher/App/planegcs/GCS.h`, `Constraints.h`, `Geo.h` |
| Sketch object | `src/Mod/Sketcher/App/SketchObject.h`, `Sketch.h`, `Constraint.h` |
| Feature base | `src/Mod/PartDesign/App/Feature.h`, `FeatureAddSub.h` |
| Extrude | `src/Mod/PartDesign/App/FeatureExtrude.h/.cpp` |
| Fillet/Chamfer | `src/Mod/PartDesign/App/FeatureFillet.h`, `FeatureChamfer.h` |
| Boolean | `src/Mod/PartDesign/App/FeatureBoolean.h` |
| Body (feature tree) | `src/Mod/PartDesign/App/Body.h` |
| Pattern/Mirror | `src/Mod/PartDesign/App/FeatureLinearPattern.h`, `FeatureMirrored.h` |
| B-Rep / TopoShape | `src/Mod/Part/App/TopoShape.h`, `BRepMesh.h` |
