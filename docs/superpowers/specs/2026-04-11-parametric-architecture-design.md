# Parametric Architecture — Design Spec

- **Date:** 2026-04-11
- **Status:** Proposed (awaiting user review)
- **Scope:** Spec #1 of 3. Depends on nothing. Blocks spec #2 (surfaces) and spec #3 (sketch flow redesign).

## Motivation

MyCad today holds a single transient `SketchSession`, a single `BRepModel`, and a single `Mesh`. Every extrude consumes the current sketch, hands a mesh to the renderer, and discards everything else. There is no document, no feature history, no component model, no way to persist work across sessions or reference past geometry from new operations.

This spec introduces the foundation that the rest of MyCad's feature roadmap sits on:

- A **component tree** (recursive, uniform nodes) so the user can organize work into parts and sub-parts.
- A **history DAG** with full parametric re-solve, branching without classic merge semantics, and structural sharing of common prefixes between components.
- A **feature framework** where every operation (datum plane creation, sketch, extrude, revolve, fillet, …) is a uniform, re-buildable node.
- A **topological naming system** (C-minimal, signature-based) so downstream features can reference upstream geometry across rebuilds.
- A **history side panel** visualizing the DAG and the component tree, with interactions for branch, merge, edit, and navigation.
- A **clean migration** of existing kernel code (`Sketch`, `extrude`, `BRepModel`, `tessellate_solid`) into the framework, with the old parallel paths deleted.

The explicit goal is that after this spec ships, *every* subsequent feature added to MyCad is a `Feature` in the framework — there is no non-parametric escape hatch.

## Non-goals

- **File save/load.** All framework state must be `Serialize + Deserialize`, but picking a file format and implementing disk I/O is a separate spec.
- **Undo outside sub-editors.** `Ctrl+Z` only works inside a sub-editor in v1. DAG navigation is deliberate, not undo-able.
- **Classic state-combining merge.** "Merge" in MyCad means "union of disjoint component sets from two branches into a new branch." No state reconciliation.
- **Topology-changing edit survival.** The C-minimal signature matcher does *not* attempt to resolve references across fillet face-splits, boolean fragmentation, or feature reordering. These cases hard-fail with a clear error and require re-picking.
- **Async / off-UI-thread rebuilds.** All rebuilds run synchronously. If rebuilds become slow enough to block the UI meaningfully, we add async in a later spec.
- **Diff view between branches, side-by-side rendering, search/filter in the history panel.** Future work.
- **Drag-to-reorder history nodes, drag-to-move features between components.** Explicitly out of scope; the DAG structure after creation is append-only (for new nodes) or mutate-in-place-at-tip (for parameter edits).
- **Per-component transforms for assemblies.** All components render at identity transform. Assembly workflows land later.

## Architectural decisions (Q&A log from brainstorming)

| # | Question | Decision |
|---|----------|----------|
| 1 | Document model | Component tree (Fusion-style, uniform nodes, recursive) |
| 2 | Scope of surface creation | Separate spec (spec #2) — out of scope here |
| 3 | Surface type system for spec #2 | DatumPlane only; recorded here for reference, not implemented in this spec |
| 4 | Parametric level | L4 branching without classic merge (merge = union of disjoint component sets) |
| 5 | History scope | Global DAG with per-node component tags (one timeline, filtered views per component) |
| 6 | Branch/component coupling | Branch holds a set of components. Branching clones components (fresh IDs). Merge unions disjoint sets. No classic state merge. |
| 7 | Node granularity | Feature-level. One node per high-level feature; sub-editors carry their own local state and local undo. |
| 8 | Topological naming | C-minimal signatures (entity kind + geometry type + adjacency fingerprint + quantized centroid + quantized measure + sibling rank); two-stage matcher; test corpus is a spec deliverable. Broken references hard-fail. |
| 9 | Undo/redo | Local per sub-editor only; `Ctrl+Z` is a no-op outside a sub-editor in v1. |
| 10 | Migration scope | Full migration — existing `Sketch`, `extrude()`, `BRepModel` move into the framework; old paths deleted. |
| 11 | Rebuild trigger + cache | Debounced auto (~150ms) + per-node cache keyed by NodeId. |
| 12 | Persistence | Out of scope; framework state is `Serialize + Deserialize` but no file I/O. |

---

## Section 1: Architecture & data model

### Crate layout

A new module in the kernel: `crates/mycad-kernel/src/parametric/`. No new crate. Sub-modules:

- `parametric::types` — `HistoryNode`, `Component`, `Branch`, `Document`, identifiers.
- `parametric::feature` — the `Feature` trait, `Operation` enum, `InputRef`, `FeatureOutput`.
- `parametric::document` — `Document` impl: branch/component/merge operations, dirty propagation.
- `parametric::rebuild` — the rebuild engine, topological ordering, cache management.
- `parametric::naming` — C-minimal signature matcher, `EntitySignature`, version upgrade hooks.
- `parametric::errors` — framework-specific variants added to `KernelError`.

Existing modules (`sketch.rs`, `features.rs`, `brep.rs`, `tessellation.rs`) stay in place. Their contents become the payloads of feature operations; their free functions are removed or made private.

### Core types

```rust
pub type NodeId = Uuid;
pub type ComponentId = Uuid;
pub type BranchId = Uuid;

/// A single node in the history DAG. Represents one feature operation.
/// Mutable in its parameters; immutable in its identity and parent link.
pub struct HistoryNode {
    pub id: NodeId,
    pub operation: Operation,                    // editable — edit marks node dirty
    pub parent: Option<NodeId>,                  // upstream state this op mutates
    pub inputs: Vec<InputRef>,                   // cross-lineage references
    pub component_tags: HashSet<ComponentId>,    // components whose current state passes through this node
    pub cached_output: Option<FeatureOutput>,    // None = dirty or never computed
    pub dirty: bool,
    pub signature_version: u32,
    pub error: Option<NodeError>,                // set when build or resolve fails
}

pub enum Operation {
    CreateDatumPlane(CreateDatumPlaneOp),
    CreateSketch(CreateSketchOp),
    Extrude(ExtrudeOp),
    // future: Revolve, Fillet, Chamfer, Boolean, ...
}

pub struct Component {
    pub id: ComponentId,
    pub name: String,
    pub parent: Option<ComponentId>,             // sub-component relationship (tree)
    pub tip: NodeId,                             // current tip in the history DAG
}

pub struct Branch {
    pub id: BranchId,
    pub name: String,
    pub components: Vec<ComponentId>,            // components present on this branch
}

pub struct Document {
    pub nodes: HashMap<NodeId, HistoryNode>,
    pub components: HashMap<ComponentId, Component>,
    pub branches: HashMap<BranchId, Branch>,
    pub root_node: NodeId,                       // synthetic empty-state sentinel
    pub current_branch: BranchId,
    pub active_component: Option<ComponentId>,   // where new ops target
}
```

### How the pieces relate

- **The DAG is the source of truth.** Nodes are the primary entity. Components and branches are *views* over the DAG.
- **A component is a pointer** — its `tip` points at a node. A component's full history is the chain of `parent` links from tip back to the root.
- **A branch is a set of components** — switching branches swaps `current_branch` and changes which node chains the UI is looking at.
- **Shared history is structural.** When two branches contain components whose chains trace through the same ancestor nodes, those nodes are literally the same `HistoryNode` in the `nodes` map, tagged with both component IDs. Editing a shared ancestor ripples into every tagged component.
- **The root node** is a synthetic empty-state sentinel. Every component's chain terminates at the root. Simplifies rebuild (always a defined starting state) and branching (always a defined ancestor).

### Initial document state

On document creation:

- One `root_node` (empty state).
- One `Branch` named `"main"` containing one `Component`.
- One `Component` named `"Part 1"` with `tip = root_node`, `parent = None`.
- `current_branch = main`, `active_component = Some(Part 1)`.

---

## Section 2: Feature trait & rebuild engine

### The Feature trait

```rust
pub trait Feature: Serialize + DeserializeOwned {
    /// Declared inputs — drives dependency tracking and dirty propagation.
    /// MUST be complete: every upstream entity this op reads.
    fn inputs(&self) -> Vec<InputRef>;

    /// Run the operation against the parent state, producing this node's output.
    /// Must be pure: same inputs → same output.
    fn build(&self, ctx: &BuildContext) -> Result<FeatureOutput, BuildError>;
}

pub struct BuildContext<'a> {
    pub parent: &'a FeatureOutput,
    pub references: HashMap<NodeId, &'a FeatureOutput>,  // populated per inputs()
    pub resolve: &'a dyn SignatureResolver,
}
```

### FeatureOutput — cumulative state snapshot

```rust
pub struct FeatureOutput {
    pub brep: BRepModel,                    // cumulative B-Rep state after this op
    pub produced: Vec<ProducedEntity>,      // entities this op specifically created
    pub mesh: Option<Mesh>,                 // display mesh for the renderer
    pub datum_planes: Vec<DatumPlaneEntry>, // datum planes in the cumulative state
    pub sketches: Vec<SketchEntry>,         // sketches in the cumulative state
}

pub struct ProducedEntity {
    pub brep_id: BRepId,
    pub signature: EntitySignature,
}
```

Each node's output is a *complete* state snapshot, not a delta. Downstream consumers read from the snapshot directly. For v1 this simplifies the engine at the cost of memory; document sizes MyCad supports today make the cost negligible. A switch to delta-based outputs can happen later if memory becomes a constraint.

### InputRef — how features reference other things

```rust
pub enum InputRef {
    BRepEntity {
        producing_node: NodeId,
        signature: EntitySignature,
    },
    DatumPlane {
        producing_node: NodeId,
        plane_id: Uuid,
    },
    SketchEntity {
        producing_node: NodeId,
        sketch_id: Uuid,
        entity_id: SketchEntityId,
    },
    World(WorldRef),  // world origin, world axes — stable, never breaks
}

pub enum WorldRef {
    Origin,
    AxisX,
    AxisY,
    AxisZ,
    PlaneXY,
    PlaneXZ,
    PlaneYZ,
}
```

Features declare inputs explicitly. The engine uses this to validate the DAG (no cycles, no forward references) and to build the dirty propagation graph.

### Rebuild engine

```rust
pub struct RebuildEngine {
    pending_since: Option<Instant>,         // debounce timer start
    dirty_queue: HashSet<NodeId>,
}
```

**Dirty propagation (on edit):**

1. User edits node N's parameters. Mark N dirty.
2. Walk forward through the DAG: any node whose `parent` is in the dirty set OR whose `inputs` reference a dirty node also becomes dirty.
3. Repeat until fixpoint (the whole reachable downstream set is marked).
4. Schedule rebuild: `pending_since = Some(now())`.

**Rebuild (after debounce expires, ~150ms):**

1. Compute topological order of the dirty set using `parent` + `inputs` edges (Kahn's algorithm).
2. For each dirty node in order:
   - Fetch parent output from cache (guaranteed present and clean by topo order).
   - Resolve each `InputRef` via the `SignatureResolver`.
   - On resolve failure → set `node.error`, mark cached_output `None`, **hard-fail**: abort rebuild. Nodes downstream of this node are left stale.
   - Otherwise call `feature.build(ctx)`. On build error → same hard-fail path.
   - On success, store result in `cached_output`, clear `dirty` and `error`.
3. After rebuild (success, partial, or failure), notify the app to refresh the renderer from each component's tip.

**Cancellation:** if new edits land during the debounce window, restart the timer and add to the dirty set. In v1, rebuilds run synchronously on the UI thread — no mid-rebuild preemption. Rebuild slowness that hurts UX is addressed by moving to async in a later spec.

**Hard-fail semantics:** when a node errors, rebuild stops at that node. Downstream nodes retain their last-known-good `cached_output` (marked stale in the UI). The errored node shows red in the history panel with its error message. The user must fix the error before rebuild proceeds.

### Cache

Per-node output lives in `HistoryNode.cached_output` directly. Invalidation == `dirty = true && cached_output = None`. Keyed implicitly by `NodeId`. No separate cache structure, no per-branch caching (unnecessary because node IDs are globally unique in the DAG — shared prefixes mean literally the same node, computed once).

### Constraints on feature implementations

- **Purity.** `Feature::build` must be deterministic. No clocks, no RNG, no I/O, no ambient state. Enforced by code review.
- **Serde.** Every `Operation` variant derives `Serialize + Deserialize`. Enforced by the trait bound.
- **Input completeness.** `inputs()` must return every cross-lineage reference the build uses. A feature that reads an entity not in its declared inputs is a bug — dirty propagation will miss it. Caught by tests.

---

## Section 3: Topological naming (C-minimal)

### Signature definition

```rust
pub struct EntitySignature {
    pub version: u32,
    pub kind: EntityKind,                  // Vertex | Edge | Face
    pub geometry: GeometryKind,            // Point | Line | Circle | Arc | Plane | Cylinder
    pub adjacency: Vec<GeometryKind>,      // sorted list of neighbors' geometry kinds
    pub centroid: QuantizedPoint3,         // value * 1000, rounded to i64 (1e-3 mm grid)
    pub measure: QuantizedScalar,          // area for Face, length for Edge, 0 for Vertex
    pub sibling_rank: u32,                 // rank within the producing node's produced list,
                                            // sorted by (centroid.x, .y, .z, measure)
}

pub struct QuantizedPoint3 { x: i64, y: i64, z: i64 }
pub struct QuantizedScalar(i64);
```

Everything is deterministic and comparable by equality. No floats in hash keys, no tolerance ambiguity at comparison time.

### Computing a signature

Signatures are computed by the *producing feature's* build step, at the point where an entity is added to `FeatureOutput.produced`. The feature walks the BRep adjacency of the new entity, collects neighbor entity kinds, quantizes, and stores the signature alongside the `BRepId`.

Signatures are computed once at build time. Matching reads stored signatures on both sides.

### Matching algorithm (the resolver)

```rust
pub trait SignatureResolver {
    fn resolve(&self, input: &InputRef) -> Result<BRepId, ResolveError>;
}

pub enum ResolveError {
    NoMatch,
    AmbiguousExact,
    Ambiguous,
    ProducingNodeMissing,
}
```

Algorithm for `InputRef::BRepEntity`:

```text
let stored = input.signature;
let candidates = producing_node.output.iter_with_signatures();

// Stage 1: exact match on (kind, geometry, adjacency, sibling_rank)
let stage1 = candidates.filter(|(_, s)|
    s.kind == stored.kind
 && s.geometry == stored.geometry
 && s.adjacency == stored.adjacency
 && s.sibling_rank == stored.sibling_rank);

match stage1.len() {
    1 => return Ok(stage1[0].id),
    0 => { /* fall through */ }
    _ => return Err(AmbiguousExact),
}

// Stage 2: relax sibling_rank, match by nearest centroid
let stage2 = candidates.filter(|(_, s)|
    s.kind == stored.kind
 && s.geometry == stored.geometry
 && s.adjacency == stored.adjacency);

if stage2.is_empty() { return Err(NoMatch); }

let best = stage2.min_by_key(|(_, s)| manhattan(s.centroid, stored.centroid))?;
let tie_count = stage2.filter(|(_, s)| manhattan(s.centroid, best.centroid) < 1).count();
if tie_count > 1 { return Err(Ambiguous); }

Ok(best.id)
```

Outcomes per `resolve` call:

- `Ok(BRepId)` — reference resolved, used by the downstream build.
- `Err(NoMatch)` — no candidate survives the filter. The entity no longer exists.
- `Err(Ambiguous)` / `Err(AmbiguousExact)` — multiple candidates, matcher refuses to guess.

Any `Err` → hard-fail on the consuming node.

### Version bump protocol

`signature_version` is a `u32` stored per node. Current version is `CURRENT_SIGNATURE_VERSION: u32`. When a document loads nodes with older versions:

1. Re-run `build()` on affected nodes to regenerate signatures in the new format. Only signatures on `produced` entries change; feature parameters are untouched.
2. For each downstream `InputRef` whose `signature.version < CURRENT_SIGNATURE_VERSION`, call `upgrade_signature(&old) -> EntitySignature` to translate it forward.
3. Re-run `resolve()` with the upgraded signature. On success, update the `InputRef` in place. On failure, hard-fail that consuming node; user re-picks.

Version upgrades live in `parametric::naming::upgrades` with one function per `V_n → V_{n+1}` jump.

### Test harness — first-class spec deliverable

Directory: `crates/mycad-kernel/tests/signature_corpus/`. Each case is a sub-directory:

```text
signature_corpus/
├── extrude_depth_change/
│   ├── before.ron
│   ├── edit.ron
│   ├── expectations.ron
│   └── README.md
├── sketch_rectangle_resize/
├── datum_plane_offset_change/
├── sketch_add_unrelated_entity/
└── extrude_profile_swap/
```

The harness:

1. Loads `before.ron` (a serialized `Document`).
2. Applies `edit.ron` (a mutation spec — node id + new parameters).
3. Runs the rebuild engine.
4. Walks every `InputRef` in downstream nodes and asserts `resolve()` matches `expectations.ron`.

Failure = regression. New bugs become new corpus entries.

**Initial corpus (ships with this spec):**

1. `extrude_depth_change` — change extrude depth, top-face reference must still resolve to the same BRepId.
2. `sketch_rectangle_resize` — move one corner, the line references (used downstream) must still resolve.
3. `datum_plane_offset_change` — change an offset plane's distance, direct `DatumPlane` references must still resolve (exercises the by-id path, not the signature path).
4. `sketch_add_unrelated_entity` — add a new line to a sketch that has no downstream reference to the new line; no downstream node should be marked dirty (tests dirty-propagation precision).
5. `extrude_profile_swap` — replace the sketch profile entirely; a downstream face reference is expected to hard-fail with `ResolveError::NoMatch`. Asserts the documented failure mode.

Cases 1–4 must return `Ok(expected_id)`. Case 5 must return the documented `Err`. All five must pass for the spec to be considered delivered.

### Deliberate non-goals (documented and tested)

- Face splits from fillets/chamfers — not supported, hard-fails.
- Boolean fragmentation — not supported.
- Feature reordering — not supported.
- Cross-session tolerance drift beyond 1e-3 mm — not supported. Quantization is the stability guarantee.

---

## Section 4: Branching, merging, components

### Branch operations

```rust
impl Document {
    /// Create a new branch by cloning components from a source branch at some node.
    pub fn branch_from(
        &mut self,
        source: BranchId,
        split_at: NodeId,
        name: String,
    ) -> Result<BranchId, KernelError>;

    /// Union two branches' component sets into a new branch.
    /// Always succeeds — component sets are disjoint by construction.
    pub fn merge_branches(
        &mut self,
        source: BranchId,
        other: BranchId,
        name: String,
    ) -> Result<BranchId, KernelError>;

    /// Switch which branch is active. Cheap — pointer move, no rebuild.
    pub fn checkout(&mut self, branch: BranchId) -> Result<(), KernelError>;

    /// Create a new empty component on the current branch.
    pub fn new_component(
        &mut self,
        name: String,
        parent: Option<ComponentId>,
    ) -> Result<ComponentId, KernelError>;

    /// Delete a branch. Refuses if it would orphan components that exist only on this branch
    /// (for v1 we log orphaned nodes but do not GC them).
    pub fn delete_branch(&mut self, branch: BranchId) -> Result<(), KernelError>;

    /// Append a new feature node onto the active component's tip on the current branch.
    pub fn append_op(&mut self, op: Operation) -> Result<NodeId, KernelError>;
}
```

### Cloning semantics on branch

When `branch_from(source, split_at, name)`:

1. Collect `source.components`.
2. For each component `C`:
   - Walk `C`'s chain from `C.tip` backwards through `parent` links.
   - If the chain passes through `split_at`, create a cloned component `C'` with a fresh `ComponentId`. Set `C'.tip = split_at`. Add `C'.id` to `component_tags` of every node from `split_at` backwards through the chain.
   - If the chain does not pass through `split_at`, clone `C` at its current tip (rare case — component created after `split_at`).
3. Create a new `Branch` containing all cloned `ComponentId`s.

Nodes are structurally shared — no content is copied. Only `component_tags` sets grow. Cost of branching is O(number of affected nodes) for tag updates.

### Merge semantics

`merge_branches(source, other, name)` produces a new `Branch` whose `components` field is the union of both source branches' component lists. Component IDs are fresh on every branch operation, so the union is always disjoint by construction — no conflict resolution is needed.

**Merging does not create a new history node.** It is purely a metadata operation on the `branches` table. The DAG structure is unchanged.

### Editing a past node

When the user edits the parameters of a non-tip node:

1. The engine detects that the edit target is not a tip of any active component on the current branch.
2. The UI prompts: *"Editing this node will create a new branch. Continue?"* (with an opt-out setting for the prompt).
3. On confirmation, call `branch_from(current_branch, edit_target, auto_name)`, `checkout(new_branch)`, then open the sub-editor on the new branch's equivalent node.
4. The original branch is unchanged — it still holds the original (unedited) chain.

**Editing a past node in-place is structurally impossible.** Editing always branches. This protects the L4 "we never lose history" invariant.

Editing the tip of the current branch does *not* branch — it mutates the node's `operation` field in place and triggers dirty propagation. The tip is "where you're working."

### Active component and sub-components

`Component.parent` records the sub-component tree. A sub-component is a `Component` whose `parent` is another `Component`'s id. There is no structural difference between a top-level component and a sub-component — same type, same operations.

`Document.active_component` records which component new operations target. When the user enters a sub-editor, the sub-editor edits a node belonging to the active component. Switching the active component updates which node chain new operations append to.

When a sub-component is created:

- Lives on the same branch as its parent component.
- Inherits no history structurally — it starts with an empty chain pointing at `root_node`.
- Its `parent` field records the tree relationship.

### Garbage collection

A node stays alive as long as any branch contains any component whose chain passes through it. When a branch is deleted:

1. Walk each component in the branch's `components` list.
2. For each, walk the chain from tip to root and remove the component's id from each node's `component_tags`.
3. Any node whose `component_tags` becomes empty (and is not `root_node`) is eligible for deletion.
4. **In v1, eligible nodes are logged but not actually deleted.** This simplifies future undo-across-sessions and reduces the risk of deleting something load-bearing. GC runs manually or via a later spec.

### Naming

Branch and component names are freely editable display strings. Identifiers are UUIDs; names are decoration.

---

## Section 5: Migration, sub-editors, undo

### What moves and how

**`crates/mycad-kernel/src/sketch.rs`** stays as a library of sketch entity types and the solver. The `Sketch` struct becomes the payload of a `CreateSketchOp`:

```rust
pub struct CreateSketchOp {
    pub host: InputRef,   // DatumPlane or flat face the sketch lives on
    pub sketch: Sketch,   // existing struct, unchanged
}

impl Feature for CreateSketchOp {
    fn inputs(&self) -> Vec<InputRef> { vec![self.host.clone()] }
    fn build(&self, ctx: &BuildContext) -> Result<FeatureOutput, BuildError> {
        // Copy parent BRep, add the sketch to the cumulative state, compute signatures for
        // referenceable sketch entities (endpoints, centers), return the new output.
    }
}
```

Solver, entity types, constraints — untouched. Only the ownership moves.

In v1, `host` is a `DatumPlane` reference only. Sketching on solid faces lands in spec #2 once `CreateDatumPlane` and face references exist.

**`crates/mycad-kernel/src/features.rs`** — `ExtrudeParams` becomes `ExtrudeOp`:

```rust
pub struct ExtrudeOp {
    pub profile: InputRef,               // reference to a closed loop in a sketch
    pub depth: Scalar,
    pub direction: ExtrudeDirection,
}

pub enum ExtrudeDirection {
    Up,
    Down,
    Symmetric,
}

impl Feature for ExtrudeOp {
    fn inputs(&self) -> Vec<InputRef> { vec![self.profile.clone()] }
    fn build(&self, ctx: &BuildContext) -> Result<FeatureOutput, BuildError> {
        // Existing extrude() logic, but reads profile from ctx.references.
    }
}
```

The free function `extrude(sketch, params)` is **deleted**. All callers route through `Document::append_op(Operation::Extrude(ExtrudeOp { ... }))`.

**`crates/mycad-kernel/src/brep.rs`** — `BRepModel` stays as a data type but is no longer a top-level store. Each `FeatureOutput` owns a `BRepModel`. The app reads the active component's tip output for rendering.

**`crates/mycad-kernel/src/tessellation.rs`** — `tessellate_solid` and friends stay. They're called by `ExtrudeOp::build` to populate `FeatureOutput.mesh`.

**`crates/mycad-renderer/src/lib.rs`** — `Viewport3d::set_mesh` keeps its signature, but now receives meshes computed from component tips. It gains a way to hold *multiple* meshes (one per visible component) rather than a single static mesh. On rebuild, the app diffs the set and updates the renderer.

**`crates/mycad-app/src/lib.rs`** — largest delta. `MyCadApp` loses `sketch_session: Option<SketchSession>` and gains `document: Document` plus `sub_editor: Option<SubEditorState>`. All command paths (Sketch button, Extrude button, keyboard shortcuts) go through the framework.

### Migration order (committable checkpoints)

Each step must leave the workspace green (`cargo test --workspace` and `cargo clippy --workspace --all-targets -- -D warnings`). If it doesn't, stop and fix before proceeding.

1. **Framework scaffolding.** Land `parametric::types`, `parametric::feature`, `parametric::document`, `parametric::rebuild`, `parametric::naming`. No features yet. Unit tests: DAG ops, dirty propagation, cache, branch/merge, signature matcher (isolated from real features).
2. **First feature: `CreateDatumPlaneOp`.** Minimal payload. Exercises the whole rebuild path end-to-end. Test: create a document, append a datum plane, verify the tip's output contains the plane. This is where the framework proves itself load-bearing for the first time.
3. **Second feature: `CreateSketchOp`.** Migrate the existing `Sketch` into the feature payload. Wire the sketch sub-editor (see below). Update `MyCadApp` to use the sub-editor for sketch mode. Keep the old `SketchSession` alive but route it through the sub-editor path.
4. **Third feature: `ExtrudeOp`.** Delete the free function `extrude`. Migrate the Extrude button and `E` keyboard shortcut to construct an `ExtrudeOp` and call `Document::append_op`. Ship the signature corpus tests — cases 1, 2, and 5 become exercisable here.
5. **History side panel UI.** Land the right-side panel per Section 6. Branch/merge/checkout operations become reachable from the UI. Keyboard shortcuts `Ctrl+B` / `Ctrl+M` wired.
6. **Old code deletion.** Remove the old `SketchSession` struct, the `sketch_session` field, `MyCadApp::perform_extrude`, and any remaining non-framework code paths. Grep for `SketchSession` and `fn extrude(` — both must be gone. The left-side "Features" panel is removed (its content has moved to the right panel).

### Sub-editors

A sub-editor is a modal UI for editing the payload of one feature node. Its state is local, not part of the DAG.

```rust
pub enum SubEditorState {
    Sketch {
        node_id: NodeId,
        local_sketch: Sketch,
        undo_stack: Vec<Sketch>,
        redo_stack: Vec<Sketch>,
        tool: SketchTool,
        hover_point: Option<Point2>,
        snapped_point: Option<Point2>,
        line_start: Option<Point2>,
        rect_start: Option<Point2>,
        circle_center: Option<Point2>,
        arc_center: Option<Point2>,
        arc_start: Option<Point2>,
    },
    ExtrudeParams {
        node_id: NodeId,
        local_params: ExtrudeOp,
        undo_stack: Vec<ExtrudeOp>,
        redo_stack: Vec<ExtrudeOp>,
    },
    DatumPlaneParams {
        node_id: NodeId,
        local_params: CreateDatumPlaneOp,
        undo_stack: Vec<CreateDatumPlaneOp>,
        redo_stack: Vec<CreateDatumPlaneOp>,
    },
}
```

**Commit rules:**

- **Commit (Finish / OK)** copies the local state into the node's `operation` field, marks the node dirty, triggers rebuild. The sub-editor closes.
- **Cancel (Esc / Cancel)** discards the local state. The node is unchanged, no rebuild.
- Entering a sub-editor on an already-committed node creates a fresh local copy. Exiting without committing leaves the node unchanged.
- Only one sub-editor can be open at a time. Trying to open a second prompts "discard current edits?".

**Edge case — entering a sub-editor on a non-tip node:** per Section 4, editing a past node auto-branches first, then the sub-editor opens on the new branch's equivalent node. The sub-editor itself is always editing a tip.

### Undo / redo

1. **Inside a sub-editor, `Ctrl+Z` and `Ctrl+Y` operate on the sub-editor's `undo_stack` / `redo_stack`.** They do not touch the DAG. Stacks live in memory and clear on commit.
2. **Outside any sub-editor, `Ctrl+Z` and `Ctrl+Y` are no-ops in v1.**

DAG navigation is deliberate — the user uses the history panel. A later spec may add "undo last committed feature" as a shortcut if users miss it.

### Error handling

`KernelError` grows:

```rust
pub enum KernelError {
    // existing variants...
    InputResolutionFailed { node: NodeId, input: InputRef, cause: ResolveError },
    BuildFailed { node: NodeId, reason: String },
    CycleDetected { nodes: Vec<NodeId> },
    InvalidBranchOperation(String),
    ActiveComponentMissing,
    RootNodeEdit,
}
```

Per the repo convention, kernel code does not panic on reachable paths. Errors propagate to the app and surface in the status bar and the history panel's node state.

### Test coverage required for the PR to merge

- DAG operations unit tests: branch, merge, checkout, tip edit, past-node edit (auto-branch), GC logging.
- Dirty propagation unit tests: mark one node, verify the downstream set is exactly the reachable forward closure.
- Rebuild engine unit tests: topological order correctness, cache hit skip, hard-fail propagation.
- Signature matcher unit tests on synthetic entities (no real BReps) covering each `ResolveError` path.
- The five initial corpus cases from Section 3.
- One end-to-end integration test: create document → append datum plane → append sketch referencing the plane → append extrude referencing a closed loop in the sketch → rebuild → assert the extrude's tip has a valid BRep and mesh.
- One migration smoke test: the existing "draw rectangle → extrude" app-level flow produces the same final mesh through the new framework.

---

## Section 6: History side panel UI

### Panel location and layout

A new side panel, docked on the **right** of the main window. Default width ≈ 280px. The panel is stacked vertically:

```text
┌────────────── History ──────────────┐
│ Branch: [main ▼]  [+ Branch] [Merge]│
├──────────────────────────────────────┤
│                                      │
│          DAG graph area              │
│       (tree of node icons)           │
│                                      │
├──────────────────────────────────────┤
│ Components                           │
│   ▾ Part 1          (active)         │
│     ◆ DatumPlane 1                   │
│     ◆ Sketch 1                       │
│     ◆ Extrude 1                      │
│   ▸ Sub-component A                  │
├──────────────────────────────────────┤
│  Properties                          │
│  (collapsible; selected node's params)│
└──────────────────────────────────────┘
```

- **Top:** branch selector + new-branch + merge buttons.
- **Middle:** the DAG visualization.
- **Lower:** the component tree, with a per-component linear feature view.
- **Bottom:** the Properties panel, now tied to the selected node.

The existing left-side "Features" panel in `MyCadApp` is **deleted**. Everything it showed lives in the right panel.

### DAG visualization

Rendered as a mini graph, in the style of a simplified `git log --graph`:

- Each **node** is a small icon (≈16–20px) with an operation-type glyph: cube for extrude, square for sketch, flat rectangle for datum plane. Extend the glyph set as new operations land.
- **Branches** are vertical lanes. The active branch is the leftmost lane, highlighted.
- **Parent edges** are straight vertical lines within a lane.
- **Branch points** are curved lines to a new lane.
- **Shared prefix nodes** appear once in whichever lane their oldest branch occupies, with a small fan-out marker showing how many branches descend.
- Node state colors:
  - **Green** — clean, rebuilt successfully.
  - **Grey** — stale (downstream of an errored node, showing last-known-good).
  - **Red** — errored (build or resolve failed). Hover shows the error message.
  - **Yellow** — dirty, pending rebuild. Visible only during the debounce window.
- **Tip nodes** (one per component on the current branch) have a thick outline. The **active component's** tip has a double outline.

**Interactions:**

- **Click** a node → select it. Properties panel updates. Non-tip nodes show read-only params.
- **Double-click** a node → enter sub-editor. If non-tip, prompts "create new branch?" first per Section 4.
- **Right-click** a node → context menu: *Branch from here*, *Copy node ID*, *Show signature* (dev aid), *Delete* (tip-only, non-shared-only).
- **Hover** a node → tooltip with operation type, short parameter summary, error if any.

There is **no rollback bar**. Navigation is explicit via click/double-click only.

### Branch selector

Dropdown at the top: `[main ▼]`. Clicking opens a list of branches with their component counts: `main (3 components)`, `experiment-1 (1 component)`. Clicking a branch name checks it out.

Adjacent buttons:

- **`+ Branch`** opens a dialog: choose source node (default = current tip), name the new branch, confirm. Calls `branch_from`.
- **`Merge`** opens a dialog: choose another branch to union with the current one. Calls `merge_branches`. Always succeeds (component sets are disjoint by construction).

### Component list

Tree view of the current branch's components, with sub-components nested:

- Active component shown bold with an "(active)" badge.
- **Click** a component row → make it active.
- **Double-click** a component row → zoom the viewport to fit that component.
- **Right-click** → rename, delete (removes from current branch only), new sub-component.
- **Expand arrow** → show the linear chain of feature nodes belonging to that component (in order, per that component's chain — not the full DAG). Clicking a feature in this linear list selects the corresponding node in the DAG visualization above.

This gives two views of the same data: the DAG (branches, shared history) on top, the per-component linear view on bottom.

### Renderer behavior

The renderer currently shows a single `Mesh`. After this spec, it shows the union of meshes from all component tips on the current branch — one mesh per component, drawn with identity transform (per-component transforms land later).

On rebuild, the app diffs the visible mesh set against the previous set and updates the renderer. Previous meshes stay visible until new ones arrive, so a slow rebuild doesn't blank the viewport.

### Keyboard shortcuts (v1)

| Shortcut | Action |
|----------|--------|
| `Ctrl+Z` / `Ctrl+Y` | Undo/redo inside the active sub-editor. No-op otherwise. |
| `Ctrl+B` | New branch from current tip |
| `Ctrl+M` | Open merge dialog |
| `Esc` | Cancel sub-editor or dismiss dialog |
| `Enter` | Confirm dialog |

Existing command shortcuts (`S`, `E`, `L`, `R`, `C`, `A`) keep their behavior but route through the framework. Spec #3 (sketch flow redesign) will repurpose some of these.

### Not in v1

- Diff view between branches.
- Side-by-side rendering of two branches.
- Collapsing history nodes into groups.
- Search / filter in the history panel.
- Drag-to-reorder history nodes.

---

## Risks and open questions

- **Rebuild performance for large documents.** v1 assumes rebuilds are fast enough for synchronous execution on the UI thread. We don't know the crossover point where this stops being true. Mitigation: the per-node cache skips clean nodes, and debouncing coalesces bursts. If real-world use hits noticeable stalls, the next step is off-thread rebuild, which is a well-understood refactor.
- **Signature matcher coverage.** C-minimal is deliberately narrow. We will encounter cases in real use that the matcher handles wrongly or hard-fails on when users expect success. The mitigation is the test corpus discipline — every wrong case becomes a new corpus entry, and the matcher's definition of correctness is the corpus.
- **UI density in the history panel.** If users create many branches, the lane layout may become cluttered. v1 uses a simple fixed-width lane per branch. If this breaks down in practice, we add lane recycling or collapsing in a follow-up.
- **GC liveness.** v1 logs orphaned nodes but does not delete them. Long-running documents will accumulate dead nodes. Acceptable for v1; revisit when persistence lands.
- **Cross-component references.** A feature in component A can declare an `InputRef` pointing into a node tagged with component B. This is how shared-history ripple works. But it also means deleting component B can break component A. Mitigation for v1: `delete_branch` and component deletion check for orphaned references and refuse if any are found.

## Spec deliverables checklist

- [ ] `parametric::types`, `parametric::feature`, `parametric::document`, `parametric::rebuild`, `parametric::naming` modules land with the types and traits described above.
- [ ] `CreateDatumPlaneOp`, `CreateSketchOp`, `ExtrudeOp` implement `Feature` and are reachable through `Document::append_op`.
- [ ] The old `extrude()` free function and `SketchSession` struct are deleted.
- [ ] The left-side "Features" panel is replaced by the right-side "History" panel with DAG, component tree, and properties sub-regions.
- [ ] Keyboard shortcuts `Ctrl+B`, `Ctrl+M`, sub-editor `Ctrl+Z`/`Ctrl+Y` are wired.
- [ ] `KernelError` gains the framework variants.
- [ ] All framework state is `Serialize + Deserialize`.
- [ ] Unit test coverage per Section 5 is present and green.
- [ ] The five initial signature corpus cases pass.
- [ ] End-to-end integration test (datum → sketch → extrude) passes.
- [ ] Migration smoke test (existing rectangle → extrude flow) passes.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` are both green on the final commit.
