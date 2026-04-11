# Parametric Architecture Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Introduce a parametric framework (history DAG, component tree, feature trait, rebuild engine, signature-based topological naming, history side panel) and migrate the existing `Sketch` / `extrude` / `BRepModel` code into it, deleting every non-framework path.

**Architecture:** A new `parametric` module in `mycad-kernel` owns the document state. A `Document` contains a `HashMap<NodeId, HistoryNode>` DAG, a set of `Component` pointers into the DAG, and a set of `Branch`es. Every operation implements a `Feature` trait with explicit `inputs()` for dependency tracking. Edits mark nodes dirty; a debounced rebuild walks the dirty set in topological order, using a signature-based resolver for cross-node references with hard-fail on any unresolvable reference. The app replaces `SketchSession` with a `sub_editor: Option<SubEditorState>` that carries local undo and commits to the document on finish.

**Tech Stack:** Rust workspace, `mycad-kernel` (no UI deps), `mycad-renderer` (wgpu + egui), `mycad-ui` (egui), `mycad-app` (eframe). New deps: `uuid` (v4, serde feature). Existing deps used: `glam` (with serde), `serde`, `thiserror`, `petgraph`, `approx`, `ron` (new dev-dep for corpus tests).

**Spec:** `docs/superpowers/specs/2026-04-11-parametric-architecture-design.md`

**Preconditions before starting:**
- `cargo test --workspace` and `cargo clippy --workspace --all-targets -- -D warnings` must be green on the current main. Run both and confirm.
- No uncommitted changes in the working tree.

---

## Task 1: Dependencies, module scaffold, and ParametricError

**Files:**
- Modify: `Cargo.toml` (workspace, add `uuid` and `ron`)
- Modify: `crates/mycad-kernel/Cargo.toml`
- Create: `crates/mycad-kernel/src/parametric/mod.rs`
- Create: `crates/mycad-kernel/src/parametric/errors.rs`
- Modify: `crates/mycad-kernel/src/lib.rs`

- [ ] **Step 1: Add uuid and ron to workspace dependencies**

Edit `Cargo.toml` at the workspace root. Locate the `[workspace.dependencies]` table and add:

```toml
uuid = { version = "1", features = ["v4", "serde"] }
ron = "0.8"
```

- [ ] **Step 2: Add uuid (and ron dev-dep) to mycad-kernel/Cargo.toml**

Edit `crates/mycad-kernel/Cargo.toml`. Under `[dependencies]` add:

```toml
uuid = { workspace = true }
```

Under `[dev-dependencies]` add:

```toml
ron = { workspace = true }
```

- [ ] **Step 3: Create the parametric module entry file**

Create `crates/mycad-kernel/src/parametric/mod.rs` with the file-level doc comment and sub-module declarations:

```rust
//! Parametric framework: history DAG, component tree, feature trait, rebuild engine.
//!
//! Every operation that mutates the document is a `HistoryNode` whose `operation`
//! payload implements [`feature::Feature`]. A [`document::Document`] owns the
//! DAG, a set of [`types::Component`]s, and a set of [`types::Branch`]es.
//! Edits are re-solved by [`rebuild::RebuildEngine`], which resolves cross-node
//! references through [`naming::SignatureResolver`].

pub mod errors;
pub mod types;
```

For now, only `errors` and `types` exist; later tasks add `feature`, `document`, `rebuild`, `naming`, and `ops` sub-modules.

- [ ] **Step 4: Create ParametricError**

Create `crates/mycad-kernel/src/parametric/errors.rs`:

```rust
//! Errors raised by the parametric framework.

use thiserror::Error;
use uuid::Uuid;

/// Errors returned from [`crate::parametric`] operations and feature builds.
#[derive(Debug, Error)]
pub enum ParametricError {
    #[error("input resolution failed for node {node}: {reason}")]
    InputResolutionFailed { node: Uuid, reason: String },

    #[error("feature build failed at node {node}: {reason}")]
    BuildFailed { node: Uuid, reason: String },

    #[error("dependency cycle detected among nodes {nodes:?}")]
    CycleDetected { nodes: Vec<Uuid> },

    #[error("invalid branch operation: {0}")]
    InvalidBranchOperation(String),

    #[error("no active component on the current branch")]
    ActiveComponentMissing,

    #[error("cannot edit the root node")]
    RootNodeEdit,

    #[error("node {0} not found in the document")]
    NodeNotFound(Uuid),

    #[error("component {0} not found in the document")]
    ComponentNotFound(Uuid),

    #[error("branch {0} not found in the document")]
    BranchNotFound(Uuid),

    #[error("unknown parametric error: {0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, ParametricError>;
```

- [ ] **Step 5: Register parametric in lib.rs**

Edit `crates/mycad-kernel/src/lib.rs`. Current contents:

```rust
pub mod math;
pub mod sketch;
pub mod brep;
pub mod features;
pub mod tessellation;
pub mod export;
```

Add a new line at the end:

```rust
pub mod parametric;
```

- [ ] **Step 6: Verify the workspace still builds**

Run: `cargo build --workspace`
Expected: success (the new module has no code referring to the empty `types` sub-module yet, so we need to create a placeholder).

If build fails with "file not found for module `types`", create a placeholder file:

Create `crates/mycad-kernel/src/parametric/types.rs` with one line:

```rust
//! Core types for the parametric framework. Populated in Task 2.
```

Re-run `cargo build --workspace`. Expected: success.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml crates/mycad-kernel/Cargo.toml crates/mycad-kernel/src/lib.rs crates/mycad-kernel/src/parametric/
git commit -m "feat(kernel): scaffold parametric module and ParametricError"
```

---

## Task 2: Core types — IDs, HistoryNode, Component, Branch, Document

**Files:**
- Modify: `crates/mycad-kernel/src/parametric/types.rs`
- Create: `crates/mycad-kernel/src/parametric/types_tests.rs` (inline `#[cfg(test)]` at the bottom of `types.rs` is the repo convention — see CLAUDE.md)

- [ ] **Step 1: Write the failing test for ID generation and HistoryNode construction**

Append to `crates/mycad-kernel/src/parametric/types.rs`:

```rust
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// Unique, serializable identifier for a history node. Stable across sessions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NodeId(pub Uuid);

impl NodeId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for NodeId {
    fn default() -> Self {
        Self::new()
    }
}

/// Unique, serializable identifier for a component.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ComponentId(pub Uuid);

impl ComponentId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for ComponentId {
    fn default() -> Self {
        Self::new()
    }
}

/// Unique, serializable identifier for a branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BranchId(pub Uuid);

impl BranchId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for BranchId {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_ids_are_unique() {
        let a = NodeId::new();
        let b = NodeId::new();
        assert_ne!(a, b);
    }

    #[test]
    fn component_ids_are_unique() {
        let a = ComponentId::new();
        let b = ComponentId::new();
        assert_ne!(a, b);
    }

    #[test]
    fn branch_ids_are_unique() {
        let a = BranchId::new();
        let b = BranchId::new();
        assert_ne!(a, b);
    }
}
```

- [ ] **Step 2: Run the tests — they should pass now**

Run: `cargo test -p mycad-kernel parametric::types::tests -- --nocapture`
Expected: 3 passing tests.

- [ ] **Step 3: Add HistoryNode, NodeError, Operation placeholder, Component, Branch, Document structs**

Append to `crates/mycad-kernel/src/parametric/types.rs` (before the `#[cfg(test)]` block):

```rust
/// Placeholder for feature operations. The full enum lives in
/// `parametric::feature::Operation` and is re-exported here for convenience
/// once it exists (Task 3). For now it's a unit-struct placeholder so
/// `HistoryNode` can compile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaceholderOperation;

/// The error state of a node after a failed rebuild.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeError {
    pub reason: String,
}

/// A cached feature output payload. The full type lives in
/// `parametric::feature::FeatureOutput` once that module exists (Task 3).
/// Until then, an empty placeholder keeps `HistoryNode` buildable.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FeatureOutputPlaceholder;

/// A single node in the history DAG. Represents one feature operation.
///
/// Mutable in its `operation` (parameter edits mutate the node in place and
/// mark it dirty). Immutable in its identity (`id`) and parent link
/// (`parent`). The cumulative cached output is stored in `cached_output`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryNode<Op = PlaceholderOperation, Out = FeatureOutputPlaceholder> {
    pub id: NodeId,
    pub operation: Op,
    pub parent: Option<NodeId>,
    pub inputs: Vec<crate::parametric::types::InputRefPlaceholder>,
    pub component_tags: HashSet<ComponentId>,
    pub cached_output: Option<Out>,
    pub dirty: bool,
    pub signature_version: u32,
    pub error: Option<NodeError>,
}

/// Placeholder for `InputRef` so `HistoryNode` has a vector type. Replaced
/// in Task 3 once the feature module defines the real `InputRef`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputRefPlaceholder;

/// A component: a tree node in the document that points at a tip in the history DAG.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Component {
    pub id: ComponentId,
    pub name: String,
    pub parent: Option<ComponentId>,
    pub tip: NodeId,
}

/// A branch: a named set of components that share a point of view on the DAG.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Branch {
    pub id: BranchId,
    pub name: String,
    pub components: Vec<ComponentId>,
}

/// The full document. Contains the DAG, components, branches, and active pointers.
///
/// The concrete `Op`/`Out` type parameters get filled in at the top level by the
/// `document` submodule once the `feature` module is in place. For now the
/// struct carries placeholders so it compiles.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document<Op = PlaceholderOperation, Out = FeatureOutputPlaceholder> {
    pub nodes: HashMap<NodeId, HistoryNode<Op, Out>>,
    pub components: HashMap<ComponentId, Component>,
    pub branches: HashMap<BranchId, Branch>,
    pub root_node: NodeId,
    pub current_branch: BranchId,
    pub active_component: Option<ComponentId>,
}
```

Then extend the test module with basic construction tests:

```rust
    #[test]
    fn history_node_fields_round_trip() {
        let id = NodeId::new();
        let component_id = ComponentId::new();
        let mut tags = HashSet::new();
        tags.insert(component_id);
        let node: HistoryNode = HistoryNode {
            id,
            operation: PlaceholderOperation,
            parent: None,
            inputs: vec![],
            component_tags: tags,
            cached_output: None,
            dirty: false,
            signature_version: 1,
            error: None,
        };
        assert_eq!(node.id, id);
        assert_eq!(node.component_tags.len(), 1);
        assert!(!node.dirty);
    }

    #[test]
    fn component_has_tip_pointer() {
        let tip = NodeId::new();
        let comp = Component {
            id: ComponentId::new(),
            name: "Part 1".into(),
            parent: None,
            tip,
        };
        assert_eq!(comp.tip, tip);
    }

    #[test]
    fn branch_is_named_component_set() {
        let id = BranchId::new();
        let branch = Branch {
            id,
            name: "main".into(),
            components: vec![ComponentId::new(), ComponentId::new()],
        };
        assert_eq!(branch.id, id);
        assert_eq!(branch.components.len(), 2);
    }
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p mycad-kernel parametric::types::tests -- --nocapture`
Expected: 6 passing tests.

- [ ] **Step 5: Run clippy to catch style issues**

Run: `cargo clippy -p mycad-kernel -- -D warnings`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add crates/mycad-kernel/src/parametric/types.rs
git commit -m "feat(kernel): add core parametric types (IDs, HistoryNode, Component, Branch, Document)"
```

---

## Task 3: Feature trait, InputRef, FeatureOutput, BuildContext

**Files:**
- Create: `crates/mycad-kernel/src/parametric/feature.rs`
- Modify: `crates/mycad-kernel/src/parametric/mod.rs`
- Modify: `crates/mycad-kernel/src/parametric/types.rs` (swap placeholders for real types)

- [ ] **Step 1: Create the feature module**

Create `crates/mycad-kernel/src/parametric/feature.rs`:

```rust
//! The feature abstraction: every operation that mutates the document is a
//! variant of [`Operation`], whose payload implements [`Feature`].
//!
//! A feature declares its inputs explicitly (`inputs()`) so the rebuild
//! engine can build a dependency graph and propagate dirty markers. The
//! `build()` method takes a `BuildContext` containing the parent output
//! and resolved references, and produces a new [`FeatureOutput`].

use crate::brep::{BRepId, BRepModel};
use crate::math::Plane;
use crate::parametric::errors::ParametricError;
use crate::parametric::naming::{EntitySignature, SignatureResolver};
use crate::parametric::types::{ComponentId, NodeId};
use crate::sketch::{Sketch, SketchEntityId};
use crate::tessellation::Mesh;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Stable identifier for a datum plane entry inside a `FeatureOutput`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DatumPlaneId(pub Uuid);

impl DatumPlaneId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for DatumPlaneId {
    fn default() -> Self {
        Self::new()
    }
}

/// Stable identifier for a sketch entry inside a `FeatureOutput`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SketchInstanceId(pub Uuid);

impl SketchInstanceId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for SketchInstanceId {
    fn default() -> Self {
        Self::new()
    }
}

/// Reference to a stable world-space entity. Always resolves; never breaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WorldRef {
    Origin,
    AxisX,
    AxisY,
    AxisZ,
    PlaneXY,
    PlaneXZ,
    PlaneYZ,
}

/// An explicit reference to an entity in some other feature node's output.
/// The rebuild engine reads these to populate `BuildContext::references` and
/// to build the dependency graph for dirty propagation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InputRef {
    BRepEntity {
        producing_node: NodeId,
        signature: EntitySignature,
    },
    DatumPlane {
        producing_node: NodeId,
        plane_id: DatumPlaneId,
    },
    SketchEntity {
        producing_node: NodeId,
        sketch_id: SketchInstanceId,
        entity_id: SketchEntityId,
    },
    World(WorldRef),
}

impl InputRef {
    /// The node this reference points into, if any. `None` for `World(..)`.
    pub fn producing_node(&self) -> Option<NodeId> {
        match self {
            Self::BRepEntity { producing_node, .. } => Some(*producing_node),
            Self::DatumPlane { producing_node, .. } => Some(*producing_node),
            Self::SketchEntity { producing_node, .. } => Some(*producing_node),
            Self::World(_) => None,
        }
    }
}

/// A datum plane entry inside a feature output. Has a stable id so later
/// features can reference it via `InputRef::DatumPlane`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatumPlaneEntry {
    pub id: DatumPlaneId,
    pub name: String,
    pub plane: Plane,
}

/// A sketch entry inside a feature output. Carries the sketch and a stable id
/// so downstream features can reference its entities.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SketchEntry {
    pub id: SketchInstanceId,
    pub name: String,
    pub sketch: Sketch,
}

/// A produced B-Rep entity and its computed signature, stored at build time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProducedEntity {
    pub brep_id: BRepId,
    pub signature: EntitySignature,
}

/// Cumulative output of a feature node: everything the component contains
/// after the operation runs, not just the delta.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FeatureOutput {
    pub brep: BRepModel,
    pub produced: Vec<ProducedEntity>,
    pub mesh: Option<Mesh>,
    pub datum_planes: Vec<DatumPlaneEntry>,
    pub sketches: Vec<SketchEntry>,
}

impl FeatureOutput {
    /// The empty root output. Used as the synthetic starting state.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Lookup a datum plane entry by id.
    pub fn find_datum_plane(&self, id: DatumPlaneId) -> Option<&DatumPlaneEntry> {
        self.datum_planes.iter().find(|d| d.id == id)
    }

    /// Lookup a sketch entry by id.
    pub fn find_sketch(&self, id: SketchInstanceId) -> Option<&SketchEntry> {
        self.sketches.iter().find(|s| s.id == id)
    }
}

/// Context passed into `Feature::build`. Contains the parent node's output
/// and any cross-node inputs the feature declared via `inputs()`.
pub struct BuildContext<'a> {
    pub parent: &'a FeatureOutput,
    pub references: HashMap<NodeId, &'a FeatureOutput>,
    pub resolve: &'a dyn SignatureResolver,
    pub this_node: NodeId,
    pub target_component: ComponentId,
}

/// The feature abstraction. Every variant of [`Operation`] carries a payload
/// that implements this trait.
pub trait Feature {
    /// Declare every upstream entity this feature reads. Drives dependency
    /// tracking and dirty propagation. MUST be complete — an unreported
    /// dependency is a bug that will be caught by the dirty-propagation tests.
    fn inputs(&self) -> Vec<InputRef>;

    /// Run the operation. Must be deterministic: same inputs → same output.
    /// No clocks, no RNG, no filesystem, no ambient kernel state.
    fn build(&self, ctx: &BuildContext) -> Result<FeatureOutput, ParametricError>;
}

/// Every concrete feature operation type. Variants are added in later tasks
/// as each feature migrates into the framework.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Operation {
    /// Placeholder variant so the enum compiles before any real features exist.
    /// Removed after Task 10 lands `CreateDatumPlaneOp`. Kept around only
    /// during early tasks to keep the tests buildable.
    #[doc(hidden)]
    Noop,
}

impl Feature for Operation {
    fn inputs(&self) -> Vec<InputRef> {
        match self {
            Self::Noop => vec![],
        }
    }

    fn build(&self, _ctx: &BuildContext) -> Result<FeatureOutput, ParametricError> {
        match self {
            Self::Noop => Ok(FeatureOutput::empty()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noop_operation_builds_empty_output() {
        let op = Operation::Noop;
        assert!(op.inputs().is_empty());
    }

    #[test]
    fn world_ref_is_stable() {
        let r = InputRef::World(WorldRef::PlaneXY);
        assert!(r.producing_node().is_none());
    }

    #[test]
    fn datum_plane_id_is_unique() {
        let a = DatumPlaneId::new();
        let b = DatumPlaneId::new();
        assert_ne!(a, b);
    }

    #[test]
    fn feature_output_empty() {
        let out = FeatureOutput::empty();
        assert!(out.produced.is_empty());
        assert!(out.datum_planes.is_empty());
        assert!(out.sketches.is_empty());
        assert!(out.mesh.is_none());
    }
}
```

**Note:** this references `crate::parametric::naming::{EntitySignature, SignatureResolver}`, which doesn't exist yet. Task 4 creates it. For now, to keep this task buildable, we stub it in the next step.

- [ ] **Step 2: Create a minimal naming stub so feature.rs compiles**

Create `crates/mycad-kernel/src/parametric/naming.rs` with just enough to satisfy `feature.rs`:

```rust
//! Topological naming. The real matcher lands in Task 4.
//!
//! This file contains only the bare types needed so [`crate::parametric::feature`]
//! compiles. Task 4 fills in the signature computation and matching algorithm.

use crate::brep::BRepId;
use crate::parametric::errors::ParametricError;
use serde::{Deserialize, Serialize};

/// Signature format version. Bumped when the layout of [`EntitySignature`] changes.
pub const CURRENT_SIGNATURE_VERSION: u32 = 1;

/// Kind of B-Rep entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EntityKind {
    Vertex,
    Edge,
    Face,
}

/// Geometric type of a B-Rep entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GeometryKind {
    Point,
    Line,
    Circle,
    Arc,
    Plane,
    Cylinder,
}

/// Quantized 3D point (value * 1000, rounded). No floats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct QuantizedPoint3 {
    pub x: i64,
    pub y: i64,
    pub z: i64,
}

/// Quantized scalar (value * 1000, rounded). No floats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct QuantizedScalar(pub i64);

/// A signature for a B-Rep entity. Populated in Task 4.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EntitySignature {
    pub version: u32,
    pub kind: EntityKind,
    pub geometry: GeometryKind,
    pub adjacency: Vec<GeometryKind>,
    pub centroid: QuantizedPoint3,
    pub measure: QuantizedScalar,
    pub sibling_rank: u32,
}

/// Errors returned from [`SignatureResolver::resolve`].
#[derive(Debug, Clone)]
pub enum ResolveError {
    NoMatch,
    AmbiguousExact,
    Ambiguous,
    ProducingNodeMissing,
}

impl std::fmt::Display for ResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoMatch => write!(f, "no matching candidate"),
            Self::AmbiguousExact => write!(f, "multiple exact matches"),
            Self::Ambiguous => write!(f, "multiple nearest candidates"),
            Self::ProducingNodeMissing => write!(f, "producing node missing"),
        }
    }
}

impl From<ResolveError> for ParametricError {
    fn from(e: ResolveError) -> Self {
        ParametricError::Other(e.to_string())
    }
}

/// Resolves an `InputRef::BRepEntity` to a current `BRepId` in the producing
/// node's output, using stored signatures.
pub trait SignatureResolver {
    fn resolve_brep_entity(
        &self,
        producing_node_output: &crate::parametric::feature::FeatureOutput,
        stored_signature: &EntitySignature,
    ) -> Result<BRepId, ResolveError>;
}
```

- [ ] **Step 3: Register feature and naming in parametric/mod.rs**

Edit `crates/mycad-kernel/src/parametric/mod.rs` to look like this:

```rust
//! Parametric framework: history DAG, component tree, feature trait, rebuild engine.

pub mod errors;
pub mod feature;
pub mod naming;
pub mod types;
```

- [ ] **Step 4: Replace placeholder types in types.rs with the real Feature types**

Edit `crates/mycad-kernel/src/parametric/types.rs`. Find the placeholder-using `HistoryNode` and `Document` and replace the default type parameters. Change:

```rust
pub struct HistoryNode<Op = PlaceholderOperation, Out = FeatureOutputPlaceholder> {
```

to:

```rust
pub struct HistoryNode {
```

and update all fields to use concrete types. Replace the previous struct with:

```rust
/// A single node in the history DAG. Represents one feature operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryNode {
    pub id: NodeId,
    pub operation: crate::parametric::feature::Operation,
    pub parent: Option<NodeId>,
    pub inputs: Vec<crate::parametric::feature::InputRef>,
    pub component_tags: HashSet<ComponentId>,
    pub cached_output: Option<crate::parametric::feature::FeatureOutput>,
    pub dirty: bool,
    pub signature_version: u32,
    pub error: Option<NodeError>,
}
```

Replace `Document` with:

```rust
/// The full document. Contains the DAG, components, branches, and active pointers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    pub nodes: HashMap<NodeId, HistoryNode>,
    pub components: HashMap<ComponentId, Component>,
    pub branches: HashMap<BranchId, Branch>,
    pub root_node: NodeId,
    pub current_branch: BranchId,
    pub active_component: Option<ComponentId>,
}
```

Delete `PlaceholderOperation`, `FeatureOutputPlaceholder`, and `InputRefPlaceholder` — they are no longer used.

Update the test in `types.rs` that uses `PlaceholderOperation` to use the real `Operation::Noop`:

```rust
    #[test]
    fn history_node_fields_round_trip() {
        let id = NodeId::new();
        let component_id = ComponentId::new();
        let mut tags = HashSet::new();
        tags.insert(component_id);
        let node = HistoryNode {
            id,
            operation: crate::parametric::feature::Operation::Noop,
            parent: None,
            inputs: vec![],
            component_tags: tags,
            cached_output: None,
            dirty: false,
            signature_version: 1,
            error: None,
        };
        assert_eq!(node.id, id);
        assert_eq!(node.component_tags.len(), 1);
        assert!(!node.dirty);
    }
```

- [ ] **Step 5: Verify**

Run: `cargo test -p mycad-kernel parametric:: -- --nocapture`
Expected: all `parametric` tests pass.

Run: `cargo clippy -p mycad-kernel -- -D warnings`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add crates/mycad-kernel/src/parametric/
git commit -m "feat(kernel): add Feature trait, InputRef, FeatureOutput, and naming stubs"
```

---

## Task 4: Signature computation and resolver (C-minimal matcher)

**Files:**
- Modify: `crates/mycad-kernel/src/parametric/naming.rs`

- [ ] **Step 1: Write failing tests for quantization**

Append to `crates/mycad-kernel/src/parametric/naming.rs`:

```rust
use crate::math::{Scalar, Vec3};

impl QuantizedPoint3 {
    /// Quantize a 3D point to the 1e-3 mm grid (value * 1000, rounded).
    pub fn from_point(p: Vec3) -> Self {
        Self {
            x: (p.x * 1000.0).round() as i64,
            y: (p.y * 1000.0).round() as i64,
            z: (p.z * 1000.0).round() as i64,
        }
    }

    /// Manhattan distance between two quantized points.
    pub fn manhattan(a: Self, b: Self) -> i64 {
        (a.x - b.x).abs() + (a.y - b.y).abs() + (a.z - b.z).abs()
    }
}

impl QuantizedScalar {
    pub fn from_scalar(s: Scalar) -> Self {
        Self((s * 1000.0).round() as i64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantized_point_rounds_to_1e3_grid() {
        let p = Vec3::new(1.2345, -0.0009, 3.0);
        let q = QuantizedPoint3::from_point(p);
        assert_eq!(q.x, 1234); // 1.2345 → 1234 (round)
        assert_eq!(q.y, -1); // -0.0009 → -1 (round)
        assert_eq!(q.z, 3000);
    }

    #[test]
    fn quantized_point_equal_for_noise() {
        let a = QuantizedPoint3::from_point(Vec3::new(1.0000001, 2.0, 3.0));
        let b = QuantizedPoint3::from_point(Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(a, b);
    }

    #[test]
    fn quantized_scalar_rounds() {
        let a = QuantizedScalar::from_scalar(5.0);
        let b = QuantizedScalar::from_scalar(5.0004);
        assert_eq!(a, b);
    }

    #[test]
    fn manhattan_distance_zero_for_equal_points() {
        let a = QuantizedPoint3 { x: 1, y: 2, z: 3 };
        assert_eq!(QuantizedPoint3::manhattan(a, a), 0);
    }

    #[test]
    fn manhattan_distance_sums_axes() {
        let a = QuantizedPoint3 { x: 0, y: 0, z: 0 };
        let b = QuantizedPoint3 { x: 1, y: 2, z: -3 };
        assert_eq!(QuantizedPoint3::manhattan(a, b), 6);
    }
}
```

- [ ] **Step 2: Run the tests**

Run: `cargo test -p mycad-kernel parametric::naming::tests -- --nocapture`
Expected: 5 passing tests.

- [ ] **Step 3: Write failing tests for `SignatureResolver` with a fake implementation**

Append the following test-only helper and resolver tests to the `tests` module in `naming.rs`:

```rust
    use crate::brep::BRepId;
    use crate::parametric::feature::{FeatureOutput, ProducedEntity};

    fn sig(
        kind: EntityKind,
        geom: GeometryKind,
        adj: &[GeometryKind],
        centroid: QuantizedPoint3,
        measure: i64,
        rank: u32,
    ) -> EntitySignature {
        EntitySignature {
            version: CURRENT_SIGNATURE_VERSION,
            kind,
            geometry: geom,
            adjacency: adj.to_vec(),
            centroid,
            measure: QuantizedScalar(measure),
            sibling_rank: rank,
        }
    }

    fn make_output_with(entities: Vec<(u64, EntitySignature)>) -> FeatureOutput {
        let mut out = FeatureOutput::empty();
        for (id, signature) in entities {
            out.produced.push(ProducedEntity {
                brep_id: BRepId(id),
                signature,
            });
        }
        out
    }

    /// Default resolver used by the rebuild engine.
    struct DefaultResolverForTest;

    impl SignatureResolver for DefaultResolverForTest {
        fn resolve_brep_entity(
            &self,
            output: &FeatureOutput,
            stored: &EntitySignature,
        ) -> Result<BRepId, ResolveError> {
            default_resolve(output, stored)
        }
    }

    #[test]
    fn resolver_exact_match_returns_id() {
        let s = sig(
            EntityKind::Face,
            GeometryKind::Plane,
            &[GeometryKind::Plane, GeometryKind::Plane],
            QuantizedPoint3 { x: 0, y: 0, z: 5000 },
            25_000,
            0,
        );
        let out = make_output_with(vec![(42, s.clone())]);
        let resolver = DefaultResolverForTest;
        let id = resolver.resolve_brep_entity(&out, &s).unwrap();
        assert_eq!(id, BRepId(42));
    }

    #[test]
    fn resolver_no_match_errors() {
        let stored = sig(
            EntityKind::Face,
            GeometryKind::Plane,
            &[GeometryKind::Plane],
            QuantizedPoint3 { x: 0, y: 0, z: 5000 },
            25_000,
            0,
        );
        let out = make_output_with(vec![]);
        let resolver = DefaultResolverForTest;
        let err = resolver.resolve_brep_entity(&out, &stored).unwrap_err();
        assert!(matches!(err, ResolveError::NoMatch));
    }

    #[test]
    fn resolver_nearest_centroid_picks_closest() {
        let stored = sig(
            EntityKind::Face,
            GeometryKind::Plane,
            &[GeometryKind::Plane],
            QuantizedPoint3 { x: 0, y: 0, z: 5000 },
            25_000,
            0,
        );
        // Two candidates. Exact match on (kind, geom, adjacency). Different sibling_rank.
        // Stage 1 will fail (rank mismatch); stage 2 picks nearest centroid.
        let c1 = sig(
            EntityKind::Face,
            GeometryKind::Plane,
            &[GeometryKind::Plane],
            QuantizedPoint3 { x: 0, y: 0, z: 5100 },
            25_000,
            1,
        );
        let c2 = sig(
            EntityKind::Face,
            GeometryKind::Plane,
            &[GeometryKind::Plane],
            QuantizedPoint3 { x: 0, y: 0, z: 9000 },
            25_000,
            2,
        );
        let out = make_output_with(vec![(7, c1), (8, c2)]);
        let resolver = DefaultResolverForTest;
        let id = resolver.resolve_brep_entity(&out, &stored).unwrap();
        assert_eq!(id, BRepId(7));
    }

    #[test]
    fn resolver_ambiguous_centroid_errors() {
        let stored = sig(
            EntityKind::Face,
            GeometryKind::Plane,
            &[GeometryKind::Plane],
            QuantizedPoint3 { x: 0, y: 0, z: 5000 },
            25_000,
            0,
        );
        // Two candidates with the same (kind, geom, adjacency) and identical centroids:
        // stage 2 must reject as ambiguous.
        let c1 = sig(
            EntityKind::Face,
            GeometryKind::Plane,
            &[GeometryKind::Plane],
            QuantizedPoint3 { x: 0, y: 0, z: 5100 },
            25_000,
            1,
        );
        let c2 = sig(
            EntityKind::Face,
            GeometryKind::Plane,
            &[GeometryKind::Plane],
            QuantizedPoint3 { x: 0, y: 0, z: 5100 },
            25_000,
            2,
        );
        let out = make_output_with(vec![(7, c1), (8, c2)]);
        let resolver = DefaultResolverForTest;
        let err = resolver.resolve_brep_entity(&out, &stored).unwrap_err();
        assert!(matches!(err, ResolveError::Ambiguous));
    }

    #[test]
    fn resolver_exact_ambiguous_errors() {
        let stored = sig(
            EntityKind::Face,
            GeometryKind::Plane,
            &[GeometryKind::Plane],
            QuantizedPoint3 { x: 0, y: 0, z: 5000 },
            25_000,
            0,
        );
        // Two candidates that exactly match on (kind, geom, adjacency, sibling_rank):
        // stage 1 must reject as AmbiguousExact.
        let out = make_output_with(vec![(7, stored.clone()), (8, stored.clone())]);
        let resolver = DefaultResolverForTest;
        let err = resolver.resolve_brep_entity(&out, &stored).unwrap_err();
        assert!(matches!(err, ResolveError::AmbiguousExact));
    }
```

- [ ] **Step 4: Implement `default_resolve`**

Append to `naming.rs` (not inside the test module):

```rust
/// Default implementation of signature resolution. Used by the rebuild engine.
/// See spec Section 3 for the algorithm.
pub fn default_resolve(
    output: &crate::parametric::feature::FeatureOutput,
    stored: &EntitySignature,
) -> Result<crate::brep::BRepId, ResolveError> {
    // Stage 1: exact match on (kind, geometry, adjacency, sibling_rank)
    let stage1: Vec<&crate::parametric::feature::ProducedEntity> = output
        .produced
        .iter()
        .filter(|p| {
            p.signature.kind == stored.kind
                && p.signature.geometry == stored.geometry
                && p.signature.adjacency == stored.adjacency
                && p.signature.sibling_rank == stored.sibling_rank
        })
        .collect();

    match stage1.len() {
        1 => return Ok(stage1[0].brep_id),
        0 => {}
        _ => return Err(ResolveError::AmbiguousExact),
    }

    // Stage 2: relax sibling_rank, pick nearest by centroid
    let stage2: Vec<&crate::parametric::feature::ProducedEntity> = output
        .produced
        .iter()
        .filter(|p| {
            p.signature.kind == stored.kind
                && p.signature.geometry == stored.geometry
                && p.signature.adjacency == stored.adjacency
        })
        .collect();

    if stage2.is_empty() {
        return Err(ResolveError::NoMatch);
    }

    let best = stage2
        .iter()
        .min_by_key(|p| QuantizedPoint3::manhattan(p.signature.centroid, stored.centroid))
        .copied()
        .unwrap();

    // Tie-breaker: if more than one candidate is within 1 quantum of best,
    // refuse to guess.
    let best_distance = QuantizedPoint3::manhattan(best.signature.centroid, stored.centroid);
    let tie_count = stage2
        .iter()
        .filter(|p| {
            QuantizedPoint3::manhattan(p.signature.centroid, stored.centroid) - best_distance < 1
        })
        .count();

    if tie_count > 1 {
        return Err(ResolveError::Ambiguous);
    }

    Ok(best.brep_id)
}

/// Zero-state resolver that delegates to `default_resolve`. Constructed by
/// the rebuild engine per call, since `default_resolve` has no persistent state.
pub struct DefaultResolver;

impl SignatureResolver for DefaultResolver {
    fn resolve_brep_entity(
        &self,
        output: &crate::parametric::feature::FeatureOutput,
        stored: &EntitySignature,
    ) -> Result<crate::brep::BRepId, ResolveError> {
        default_resolve(output, stored)
    }
}
```

- [ ] **Step 5: Run the tests**

Run: `cargo test -p mycad-kernel parametric::naming::tests -- --nocapture`
Expected: 10 passing tests (5 quantization + 5 resolver).

- [ ] **Step 6: Clippy**

Run: `cargo clippy -p mycad-kernel -- -D warnings`
Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add crates/mycad-kernel/src/parametric/naming.rs
git commit -m "feat(kernel): implement C-minimal signature resolver with two-stage matching"
```

---

## Task 5: Document basics — `Document::new`, lookup, and `append_op`

**Files:**
- Create: `crates/mycad-kernel/src/parametric/document.rs`
- Modify: `crates/mycad-kernel/src/parametric/mod.rs`
- Modify: `crates/mycad-kernel/src/parametric/types.rs` — move `Document` to only live in `document.rs`, re-export through types or keep the struct here and add behavior via impl blocks.

**Decision:** keep `Document` the data definition in `types.rs`, and put behavior (impl methods) in `document.rs`. This matches the repo's "small files, one responsibility" goal.

- [ ] **Step 1: Create document.rs with `Document::new`**

Create `crates/mycad-kernel/src/parametric/document.rs`:

```rust
//! Behavior for [`crate::parametric::types::Document`]: construction, lookup,
//! append_op, new_component, branching, merging, checkout, deletion.

use crate::parametric::errors::{ParametricError, Result};
use crate::parametric::feature::{FeatureOutput, Operation};
use crate::parametric::naming::CURRENT_SIGNATURE_VERSION;
use crate::parametric::types::{
    Branch, BranchId, Component, ComponentId, Document, HistoryNode, NodeId,
};
use std::collections::{HashMap, HashSet};

impl Document {
    /// Create an empty document with one root node, one "main" branch, and
    /// one "Part 1" component pointing at the root.
    pub fn new() -> Self {
        let root_id = NodeId::new();
        let mut nodes = HashMap::new();

        let component_id = ComponentId::new();
        let mut component_tags = HashSet::new();
        component_tags.insert(component_id);

        // The root node is a synthetic empty-state sentinel. Its output is
        // pre-computed to `FeatureOutput::empty()` so the rebuild engine
        // never has to run it.
        nodes.insert(
            root_id,
            HistoryNode {
                id: root_id,
                operation: Operation::Noop,
                parent: None,
                inputs: vec![],
                component_tags: component_tags.clone(),
                cached_output: Some(FeatureOutput::empty()),
                dirty: false,
                signature_version: CURRENT_SIGNATURE_VERSION,
                error: None,
            },
        );

        let mut components = HashMap::new();
        components.insert(
            component_id,
            Component {
                id: component_id,
                name: "Part 1".into(),
                parent: None,
                tip: root_id,
            },
        );

        let branch_id = BranchId::new();
        let mut branches = HashMap::new();
        branches.insert(
            branch_id,
            Branch {
                id: branch_id,
                name: "main".into(),
                components: vec![component_id],
            },
        );

        Self {
            nodes,
            components,
            branches,
            root_node: root_id,
            current_branch: branch_id,
            active_component: Some(component_id),
        }
    }

    // --- Lookups -------------------------------------------------------------

    pub fn node(&self, id: NodeId) -> Result<&HistoryNode> {
        self.nodes.get(&id).ok_or(ParametricError::NodeNotFound(id.0))
    }

    pub fn node_mut(&mut self, id: NodeId) -> Result<&mut HistoryNode> {
        self.nodes
            .get_mut(&id)
            .ok_or(ParametricError::NodeNotFound(id.0))
    }

    pub fn component(&self, id: ComponentId) -> Result<&Component> {
        self.components
            .get(&id)
            .ok_or(ParametricError::ComponentNotFound(id.0))
    }

    pub fn branch(&self, id: BranchId) -> Result<&Branch> {
        self.branches
            .get(&id)
            .ok_or(ParametricError::BranchNotFound(id.0))
    }

    /// The component currently receiving new operations.
    pub fn active_component(&self) -> Result<&Component> {
        let id = self
            .active_component
            .ok_or(ParametricError::ActiveComponentMissing)?;
        self.component(id)
    }

    // --- Append -------------------------------------------------------------

    /// Append a new feature node to the active component's tip on the current branch.
    /// Returns the new node's id. The node starts dirty; a rebuild must be run
    /// before its `cached_output` is populated.
    pub fn append_op(&mut self, op: Operation) -> Result<NodeId> {
        let component_id = self
            .active_component
            .ok_or(ParametricError::ActiveComponentMissing)?;
        let component = self
            .components
            .get(&component_id)
            .ok_or(ParametricError::ComponentNotFound(component_id.0))?
            .clone();

        let new_id = NodeId::new();
        let mut tags = HashSet::new();
        tags.insert(component_id);

        let inputs = op.inputs_list();

        self.nodes.insert(
            new_id,
            HistoryNode {
                id: new_id,
                operation: op,
                parent: Some(component.tip),
                inputs,
                component_tags: tags,
                cached_output: None,
                dirty: true,
                signature_version: CURRENT_SIGNATURE_VERSION,
                error: None,
            },
        );

        // Update component tip to point at the new node.
        if let Some(c) = self.components.get_mut(&component_id) {
            c.tip = new_id;
        }

        Ok(new_id)
    }

    /// Create a new empty component on the current branch, with its tip at the root.
    pub fn new_component(
        &mut self,
        name: String,
        parent: Option<ComponentId>,
    ) -> Result<ComponentId> {
        if let Some(p) = parent {
            if !self.components.contains_key(&p) {
                return Err(ParametricError::ComponentNotFound(p.0));
            }
        }

        let id = ComponentId::new();
        self.components.insert(
            id,
            Component {
                id,
                name,
                parent,
                tip: self.root_node,
            },
        );

        // Add to current branch.
        let current_branch_id = self.current_branch;
        let branch = self
            .branches
            .get_mut(&current_branch_id)
            .ok_or(ParametricError::BranchNotFound(current_branch_id.0))?;
        branch.components.push(id);

        // Also add the component to the root node's component_tags.
        if let Some(root) = self.nodes.get_mut(&self.root_node) {
            root.component_tags.insert(id);
        }

        Ok(id)
    }
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

// --- Helper: Operation::inputs_list -------------------------------------------

impl Operation {
    /// Internal helper so `append_op` can grab `inputs()` without importing `Feature`.
    pub(crate) fn inputs_list(&self) -> Vec<crate::parametric::feature::InputRef> {
        use crate::parametric::feature::Feature;
        <Self as Feature>::inputs(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_document_has_root_main_and_part1() {
        let doc = Document::new();
        assert_eq!(doc.nodes.len(), 1);
        assert_eq!(doc.components.len(), 1);
        assert_eq!(doc.branches.len(), 1);
        assert!(doc.active_component.is_some());

        let root = doc.node(doc.root_node).unwrap();
        assert!(root.cached_output.is_some());
        assert!(!root.dirty);

        let comp = doc.active_component().unwrap();
        assert_eq!(comp.name, "Part 1");
        assert_eq!(comp.tip, doc.root_node);

        let branch = doc.branch(doc.current_branch).unwrap();
        assert_eq!(branch.name, "main");
        assert_eq!(branch.components.len(), 1);
    }

    #[test]
    fn append_op_updates_tip_and_marks_dirty() {
        let mut doc = Document::new();
        let new_id = doc.append_op(Operation::Noop).unwrap();

        let node = doc.node(new_id).unwrap();
        assert!(node.dirty);
        assert_eq!(node.parent, Some(doc.root_node));

        let comp = doc.active_component().unwrap();
        assert_eq!(comp.tip, new_id);
    }

    #[test]
    fn new_component_starts_at_root() {
        let mut doc = Document::new();
        let id = doc.new_component("Part 2".into(), None).unwrap();
        let c = doc.component(id).unwrap();
        assert_eq!(c.tip, doc.root_node);
        let branch = doc.branch(doc.current_branch).unwrap();
        assert!(branch.components.contains(&id));
    }

    #[test]
    fn sub_component_has_parent_id() {
        let mut doc = Document::new();
        let parent = doc.active_component.unwrap();
        let child = doc.new_component("Sub".into(), Some(parent)).unwrap();
        assert_eq!(doc.component(child).unwrap().parent, Some(parent));
    }

    #[test]
    fn node_not_found_error() {
        let doc = Document::new();
        let bogus = NodeId::new();
        assert!(matches!(doc.node(bogus), Err(ParametricError::NodeNotFound(_))));
    }
}
```

- [ ] **Step 2: Register document in parametric/mod.rs**

Edit `crates/mycad-kernel/src/parametric/mod.rs`:

```rust
//! Parametric framework: history DAG, component tree, feature trait, rebuild engine.

pub mod document;
pub mod errors;
pub mod feature;
pub mod naming;
pub mod types;
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p mycad-kernel parametric:: -- --nocapture`
Expected: all parametric tests pass.

Run: `cargo clippy -p mycad-kernel -- -D warnings`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add crates/mycad-kernel/src/parametric/
git commit -m "feat(kernel): add Document::new, append_op, new_component, and lookup helpers"
```

---

## Task 6: Branching — `branch_from`, `merge_branches`, `checkout`, `delete_branch`

**Files:**
- Modify: `crates/mycad-kernel/src/parametric/document.rs`

- [ ] **Step 1: Write failing tests for branch_from, checkout, merge, delete**

Add these tests to the existing `tests` module in `document.rs`:

```rust
    #[test]
    fn branch_from_clones_components_with_new_ids() {
        let mut doc = Document::new();
        let source_branch = doc.current_branch;
        // Append one operation so there's something to share.
        let _n1 = doc.append_op(Operation::Noop).unwrap();
        let split_at = doc.active_component().unwrap().tip;

        let new_branch = doc
            .branch_from(source_branch, split_at, "experiment".into())
            .unwrap();

        // The new branch exists with one component whose id is not the original.
        let source_comp_id = doc.branches[&source_branch].components[0];
        let new_branch_struct = doc.branch(new_branch).unwrap();
        assert_eq!(new_branch_struct.name, "experiment");
        assert_eq!(new_branch_struct.components.len(), 1);
        let new_comp_id = new_branch_struct.components[0];
        assert_ne!(new_comp_id, source_comp_id);

        // The new component's tip is at split_at.
        assert_eq!(doc.component(new_comp_id).unwrap().tip, split_at);

        // The split_at node's component_tags contains both component ids.
        let node = doc.node(split_at).unwrap();
        assert!(node.component_tags.contains(&source_comp_id));
        assert!(node.component_tags.contains(&new_comp_id));

        // Nodes from root up through split_at all carry the new tag too.
        let mut cur = Some(split_at);
        while let Some(id) = cur {
            let n = doc.node(id).unwrap();
            assert!(n.component_tags.contains(&new_comp_id));
            cur = n.parent;
        }
    }

    #[test]
    fn checkout_changes_current_branch_cheap() {
        let mut doc = Document::new();
        let source_branch = doc.current_branch;
        let _n1 = doc.append_op(Operation::Noop).unwrap();
        let split_at = doc.active_component().unwrap().tip;
        let new_branch = doc.branch_from(source_branch, split_at, "exp".into()).unwrap();

        doc.checkout(new_branch).unwrap();
        assert_eq!(doc.current_branch, new_branch);
        let expected_active = doc.branches[&new_branch].components[0];
        assert_eq!(doc.active_component, Some(expected_active));
    }

    #[test]
    fn merge_branches_unions_component_sets() {
        let mut doc = Document::new();
        let main_id = doc.current_branch;
        // Create a second branch containing a second "Part 2" component.
        // Easiest: branch_from, then new_component on the new branch.
        let split_at = doc.root_node;
        let other = doc.branch_from(main_id, split_at, "alt".into()).unwrap();
        doc.checkout(other).unwrap();
        doc.new_component("Part 2".into(), None).unwrap();
        doc.checkout(main_id).unwrap();

        let merged = doc.merge_branches(main_id, other, "combined".into()).unwrap();
        let mb = doc.branch(merged).unwrap();
        let main_len = doc.branch(main_id).unwrap().components.len();
        let other_len = doc.branch(other).unwrap().components.len();
        assert_eq!(mb.components.len(), main_len + other_len);
    }

    #[test]
    fn delete_branch_removes_from_branches_table() {
        let mut doc = Document::new();
        let main_id = doc.current_branch;
        let extra = doc.branch_from(main_id, doc.root_node, "extra".into()).unwrap();
        doc.delete_branch(extra).unwrap();
        assert!(!doc.branches.contains_key(&extra));
    }

    #[test]
    fn delete_current_branch_errors() {
        let mut doc = Document::new();
        let main_id = doc.current_branch;
        let err = doc.delete_branch(main_id).unwrap_err();
        assert!(matches!(err, ParametricError::InvalidBranchOperation(_)));
    }
```

- [ ] **Step 2: Run tests — they should fail with "method not found"**

Run: `cargo test -p mycad-kernel parametric::document::tests -- --nocapture`
Expected: compilation error on `branch_from`, `checkout`, `merge_branches`, `delete_branch`.

- [ ] **Step 3: Implement branch_from**

Append to the `impl Document` block in `document.rs`:

```rust
    // --- Branching ----------------------------------------------------------

    /// Create a new branch by cloning components from a source branch at `split_at`.
    /// Every component on the source branch whose chain passes through `split_at`
    /// gets a fresh [`ComponentId`] on the new branch, tagged into the ancestor
    /// nodes from `split_at` back through root.
    pub fn branch_from(
        &mut self,
        source: BranchId,
        split_at: NodeId,
        name: String,
    ) -> Result<BranchId> {
        let source_components = self
            .branches
            .get(&source)
            .ok_or(ParametricError::BranchNotFound(source.0))?
            .components
            .clone();

        // Validate split_at exists.
        if !self.nodes.contains_key(&split_at) {
            return Err(ParametricError::NodeNotFound(split_at.0));
        }

        let mut cloned_component_ids = Vec::with_capacity(source_components.len());

        for source_comp_id in &source_components {
            let source_comp = self
                .components
                .get(source_comp_id)
                .ok_or(ParametricError::ComponentNotFound(source_comp_id.0))?
                .clone();

            // Determine the cloned component's tip:
            // - If source_comp's chain passes through split_at, tip = split_at.
            // - Otherwise tip = source_comp.tip (component is independent of the split).
            let passes_through = {
                let mut cur = Some(source_comp.tip);
                let mut found = false;
                while let Some(id) = cur {
                    if id == split_at {
                        found = true;
                        break;
                    }
                    cur = self.nodes.get(&id).and_then(|n| n.parent);
                }
                found
            };

            let new_tip = if passes_through {
                split_at
            } else {
                source_comp.tip
            };

            let cloned_id = ComponentId::new();
            self.components.insert(
                cloned_id,
                Component {
                    id: cloned_id,
                    name: source_comp.name.clone(),
                    parent: source_comp.parent,
                    tip: new_tip,
                },
            );

            // Walk from the cloned tip back to root, tagging each node with cloned_id.
            let mut cur = Some(new_tip);
            while let Some(id) = cur {
                if let Some(node) = self.nodes.get_mut(&id) {
                    node.component_tags.insert(cloned_id);
                    cur = node.parent;
                } else {
                    break;
                }
            }

            cloned_component_ids.push(cloned_id);
        }

        let new_branch_id = BranchId::new();
        self.branches.insert(
            new_branch_id,
            Branch {
                id: new_branch_id,
                name,
                components: cloned_component_ids,
            },
        );

        Ok(new_branch_id)
    }

    /// Checkout a branch — swap `current_branch` and set `active_component` to
    /// the first component in the branch's list (if any).
    pub fn checkout(&mut self, branch: BranchId) -> Result<()> {
        let b = self
            .branches
            .get(&branch)
            .ok_or(ParametricError::BranchNotFound(branch.0))?;
        self.active_component = b.components.first().copied();
        self.current_branch = branch;
        Ok(())
    }

    /// Merge two branches into a new branch whose component set is the union
    /// of both. Component sets are always disjoint by construction (branching
    /// clones ids), so the union never has overlap.
    pub fn merge_branches(
        &mut self,
        source: BranchId,
        other: BranchId,
        name: String,
    ) -> Result<BranchId> {
        let source_components = self
            .branches
            .get(&source)
            .ok_or(ParametricError::BranchNotFound(source.0))?
            .components
            .clone();
        let other_components = self
            .branches
            .get(&other)
            .ok_or(ParametricError::BranchNotFound(other.0))?
            .components
            .clone();

        // Sanity-check the disjoint invariant. If this ever triggers it's a bug.
        for c in &other_components {
            if source_components.contains(c) {
                return Err(ParametricError::InvalidBranchOperation(format!(
                    "component {:?} appears in both branches (violates disjoint invariant)",
                    c
                )));
            }
        }

        let mut merged = source_components;
        merged.extend(other_components);

        let new_id = BranchId::new();
        self.branches.insert(
            new_id,
            Branch {
                id: new_id,
                name,
                components: merged,
            },
        );
        Ok(new_id)
    }

    /// Delete a branch. Refuses if `branch == current_branch`.
    pub fn delete_branch(&mut self, branch: BranchId) -> Result<()> {
        if branch == self.current_branch {
            return Err(ParametricError::InvalidBranchOperation(
                "cannot delete the current branch".into(),
            ));
        }
        self.branches
            .remove(&branch)
            .ok_or(ParametricError::BranchNotFound(branch.0))?;
        // GC is intentionally not run here (see spec Section 4 — orphaned nodes are
        // logged but not deleted in v1).
        Ok(())
    }
```

- [ ] **Step 4: Run tests — should pass**

Run: `cargo test -p mycad-kernel parametric::document::tests -- --nocapture`
Expected: all tests pass, including the new branching tests.

- [ ] **Step 5: Clippy**

Run: `cargo clippy -p mycad-kernel -- -D warnings`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add crates/mycad-kernel/src/parametric/document.rs
git commit -m "feat(kernel): add branch_from, merge_branches, checkout, delete_branch"
```

---

## Task 7: Rebuild engine — dirty propagation and topological rebuild

**Files:**
- Create: `crates/mycad-kernel/src/parametric/rebuild.rs`
- Modify: `crates/mycad-kernel/src/parametric/mod.rs`

- [ ] **Step 1: Create rebuild.rs with types and failing tests**

Create `crates/mycad-kernel/src/parametric/rebuild.rs`:

```rust
//! Rebuild engine: dirty propagation, topological rebuild, hard-fail.
//!
//! Edits mark nodes dirty; `propagate_dirty` closes dirtyness forward along
//! parent+inputs edges; `rebuild` runs `Feature::build` on dirty nodes in
//! topological order, storing outputs in each node's `cached_output`. If any
//! node's resolve or build fails, `rebuild` aborts with the error set on the
//! failing node (hard-fail) — nodes downstream keep their last-known-good
//! output but are marked stale.

use crate::parametric::errors::{ParametricError, Result};
use crate::parametric::feature::{BuildContext, Feature, FeatureOutput, InputRef};
use crate::parametric::naming::DefaultResolver;
use crate::parametric::types::{Document, NodeError, NodeId};
use std::collections::{HashMap, HashSet, VecDeque};

/// Mark a node as needing rebuild. Also propagates dirtyness forward through
/// every descendant (via `parent` chains and `inputs` edges).
pub fn mark_dirty(doc: &mut Document, start: NodeId) -> Result<()> {
    if !doc.nodes.contains_key(&start) {
        return Err(ParametricError::NodeNotFound(start.0));
    }

    // Build reverse adjacency: for every node, which nodes read from it as parent
    // or input?
    let mut reverse: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
    for (id, node) in &doc.nodes {
        if let Some(p) = node.parent {
            reverse.entry(p).or_default().push(*id);
        }
        for input in &node.inputs {
            if let Some(producing) = input.producing_node() {
                reverse.entry(producing).or_default().push(*id);
            }
        }
    }

    // BFS forward from `start`, marking each reached node dirty.
    let mut queue: VecDeque<NodeId> = VecDeque::new();
    queue.push_back(start);
    let mut visited = HashSet::new();

    while let Some(cur) = queue.pop_front() {
        if !visited.insert(cur) {
            continue;
        }
        if let Some(n) = doc.nodes.get_mut(&cur) {
            n.dirty = true;
            n.cached_output = None;
        }
        if let Some(children) = reverse.get(&cur) {
            for c in children {
                queue.push_back(*c);
            }
        }
    }

    Ok(())
}

/// The set of nodes currently marked dirty in the document.
pub fn dirty_set(doc: &Document) -> HashSet<NodeId> {
    doc.nodes
        .iter()
        .filter_map(|(id, n)| if n.dirty { Some(*id) } else { None })
        .collect()
}

/// Topological order of `nodes` by (parent, input) edges. Returns
/// `CycleDetected` if the induced subgraph is not a DAG.
pub fn topological_order(doc: &Document, nodes: &HashSet<NodeId>) -> Result<Vec<NodeId>> {
    // Kahn's algorithm over the subgraph induced by `nodes`.
    let mut in_degree: HashMap<NodeId, usize> = HashMap::new();
    for id in nodes {
        in_degree.insert(*id, 0);
    }
    let mut edges: Vec<(NodeId, NodeId)> = Vec::new(); // (from, to)

    for id in nodes {
        let node = doc
            .nodes
            .get(id)
            .ok_or(ParametricError::NodeNotFound(id.0))?;
        // Parent dependency.
        if let Some(p) = node.parent {
            if nodes.contains(&p) {
                edges.push((p, *id));
                *in_degree.entry(*id).or_insert(0) += 1;
            }
        }
        // Input dependencies.
        for input in &node.inputs {
            if let Some(producing) = input.producing_node() {
                if nodes.contains(&producing) {
                    edges.push((producing, *id));
                    *in_degree.entry(*id).or_insert(0) += 1;
                }
            }
        }
    }

    let mut queue: VecDeque<NodeId> = in_degree
        .iter()
        .filter_map(|(id, d)| if *d == 0 { Some(*id) } else { None })
        .collect();
    let mut order = Vec::with_capacity(nodes.len());

    while let Some(n) = queue.pop_front() {
        order.push(n);
        for (from, to) in edges.iter() {
            if *from == n {
                if let Some(d) = in_degree.get_mut(to) {
                    *d -= 1;
                    if *d == 0 {
                        queue.push_back(*to);
                    }
                }
            }
        }
    }

    if order.len() != nodes.len() {
        let remaining: Vec<_> = in_degree
            .into_iter()
            .filter_map(|(id, d)| if d > 0 { Some(id.0) } else { None })
            .collect();
        return Err(ParametricError::CycleDetected { nodes: remaining });
    }

    Ok(order)
}

/// Run one rebuild pass: walk every dirty node in topological order and call
/// its `Feature::build`. On any failure, set the failing node's `error` field
/// and abort (hard-fail). Nodes downstream of the failing node retain their
/// previous `cached_output` but are left dirty (stale).
pub fn rebuild(doc: &mut Document) -> Result<()> {
    let dirty = dirty_set(doc);
    if dirty.is_empty() {
        return Ok(());
    }
    let order = topological_order(doc, &dirty)?;

    let resolver = DefaultResolver;

    for node_id in order {
        // Gather everything we need by borrowing `doc` immutably first.
        let (op, parent_id, inputs, target_component) = {
            let node = doc.node(node_id)?;
            (
                node.operation.clone(),
                node.parent,
                node.inputs.clone(),
                node.component_tags
                    .iter()
                    .next()
                    .copied()
                    .ok_or(ParametricError::Other(format!(
                        "node {:?} has no component tag",
                        node_id
                    )))?,
            )
        };

        // Parent output.
        let parent_output: FeatureOutput = match parent_id {
            None => FeatureOutput::empty(),
            Some(pid) => doc
                .node(pid)?
                .cached_output
                .clone()
                .ok_or_else(|| ParametricError::BuildFailed {
                    node: node_id.0,
                    reason: format!("parent node {} has no cached output", pid.0),
                })?,
        };

        // Referenced node outputs for inputs.
        let mut references: HashMap<NodeId, FeatureOutput> = HashMap::new();
        for input in &inputs {
            if let Some(producing) = input.producing_node() {
                if let std::collections::hash_map::Entry::Vacant(e) = references.entry(producing) {
                    let out = doc
                        .node(producing)?
                        .cached_output
                        .clone()
                        .ok_or_else(|| ParametricError::BuildFailed {
                            node: node_id.0,
                            reason: format!(
                                "referenced node {} has no cached output",
                                producing.0
                            ),
                        })?;
                    e.insert(out);
                }
            }
        }

        // Run the build.
        let references_ref: HashMap<NodeId, &FeatureOutput> =
            references.iter().map(|(k, v)| (*k, v)).collect();
        let ctx = BuildContext {
            parent: &parent_output,
            references: references_ref,
            resolve: &resolver,
            this_node: node_id,
            target_component,
        };

        let build_result = op.build(&ctx);

        // Write results back.
        let node = doc.node_mut(node_id)?;
        match build_result {
            Ok(out) => {
                node.cached_output = Some(out);
                node.dirty = false;
                node.error = None;
            }
            Err(e) => {
                let reason = e.to_string();
                node.error = Some(NodeError { reason: reason.clone() });
                // Do NOT clear dirty; leave node stale.
                return Err(ParametricError::BuildFailed {
                    node: node_id.0,
                    reason,
                });
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parametric::feature::Operation;

    #[test]
    fn mark_dirty_propagates_forward() {
        let mut doc = Document::new();
        let n1 = doc.append_op(Operation::Noop).unwrap();
        let n2 = doc.append_op(Operation::Noop).unwrap();

        mark_dirty(&mut doc, n1).unwrap();
        assert!(doc.node(n1).unwrap().dirty);
        assert!(doc.node(n2).unwrap().dirty);
    }

    #[test]
    fn topological_order_respects_parent_edges() {
        let mut doc = Document::new();
        let n1 = doc.append_op(Operation::Noop).unwrap();
        let n2 = doc.append_op(Operation::Noop).unwrap();
        let n3 = doc.append_op(Operation::Noop).unwrap();

        let set: HashSet<_> = [n1, n2, n3].into_iter().collect();
        let order = topological_order(&doc, &set).unwrap();
        // n1 must come before n2, n2 before n3.
        let p1 = order.iter().position(|x| *x == n1).unwrap();
        let p2 = order.iter().position(|x| *x == n2).unwrap();
        let p3 = order.iter().position(|x| *x == n3).unwrap();
        assert!(p1 < p2);
        assert!(p2 < p3);
    }

    #[test]
    fn rebuild_noop_chain_clears_dirty() {
        let mut doc = Document::new();
        let n1 = doc.append_op(Operation::Noop).unwrap();
        let n2 = doc.append_op(Operation::Noop).unwrap();

        rebuild(&mut doc).unwrap();
        assert!(!doc.node(n1).unwrap().dirty);
        assert!(!doc.node(n2).unwrap().dirty);
        assert!(doc.node(n1).unwrap().cached_output.is_some());
        assert!(doc.node(n2).unwrap().cached_output.is_some());
    }

    #[test]
    fn rebuild_empty_dirty_set_is_no_op() {
        let mut doc = Document::new();
        rebuild(&mut doc).unwrap();
    }
}
```

- [ ] **Step 2: Register rebuild in parametric/mod.rs**

Edit `crates/mycad-kernel/src/parametric/mod.rs`:

```rust
//! Parametric framework: history DAG, component tree, feature trait, rebuild engine.

pub mod document;
pub mod errors;
pub mod feature;
pub mod naming;
pub mod rebuild;
pub mod types;
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p mycad-kernel parametric:: -- --nocapture`
Expected: all parametric tests pass, including the four new rebuild tests.

Run: `cargo clippy -p mycad-kernel -- -D warnings`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add crates/mycad-kernel/src/parametric/
git commit -m "feat(kernel): add rebuild engine with dirty propagation and topological rebuild"
```

---

## Task 8: `CreateDatumPlaneOp` — the first real feature

**Files:**
- Create: `crates/mycad-kernel/src/parametric/ops/mod.rs`
- Create: `crates/mycad-kernel/src/parametric/ops/datum_plane.rs`
- Modify: `crates/mycad-kernel/src/parametric/mod.rs`
- Modify: `crates/mycad-kernel/src/parametric/feature.rs` (add variant)

- [ ] **Step 1: Create ops module skeleton**

Create `crates/mycad-kernel/src/parametric/ops/mod.rs`:

```rust
//! Concrete feature operation implementations.

pub mod datum_plane;
```

Edit `crates/mycad-kernel/src/parametric/mod.rs` to add:

```rust
pub mod ops;
```

- [ ] **Step 2: Implement CreateDatumPlaneOp**

Create `crates/mycad-kernel/src/parametric/ops/datum_plane.rs`:

```rust
//! `CreateDatumPlaneOp`: adds a datum plane to the cumulative feature output.

use crate::math::Plane;
use crate::parametric::errors::ParametricError;
use crate::parametric::feature::{
    BuildContext, DatumPlaneEntry, DatumPlaneId, Feature, FeatureOutput, InputRef, WorldRef,
};
use serde::{Deserialize, Serialize};

/// Construction methods for a datum plane. For Spec #1, only `World` (one of
/// the three world planes) is fully supported. Spec #2 adds `Offset`,
/// `ThreePoints`, `ThroughPointParallelTo`, and `Midplane`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DatumPlaneConstruction {
    World(WorldRef),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateDatumPlaneOp {
    pub id: DatumPlaneId,
    pub name: String,
    pub construction: DatumPlaneConstruction,
}

impl CreateDatumPlaneOp {
    /// Convenience constructor for the three world planes.
    pub fn world(world_ref: WorldRef, name: impl Into<String>) -> Self {
        Self {
            id: DatumPlaneId::new(),
            name: name.into(),
            construction: DatumPlaneConstruction::World(world_ref),
        }
    }

    fn resolve_plane(&self) -> Result<Plane, ParametricError> {
        match self.construction {
            DatumPlaneConstruction::World(WorldRef::PlaneXY) => Ok(Plane::xy()),
            DatumPlaneConstruction::World(WorldRef::PlaneXZ) => Ok(Plane::xz()),
            DatumPlaneConstruction::World(WorldRef::PlaneYZ) => Ok(Plane::yz()),
            DatumPlaneConstruction::World(other) => Err(ParametricError::BuildFailed {
                node: uuid::Uuid::nil(),
                reason: format!("WorldRef::{:?} is not a plane", other),
            }),
        }
    }
}

impl Feature for CreateDatumPlaneOp {
    fn inputs(&self) -> Vec<InputRef> {
        match self.construction {
            DatumPlaneConstruction::World(world_ref) => vec![InputRef::World(world_ref)],
        }
    }

    fn build(&self, ctx: &BuildContext) -> Result<FeatureOutput, ParametricError> {
        let plane = self.resolve_plane()?;
        let mut out = ctx.parent.clone();
        out.datum_planes.push(DatumPlaneEntry {
            id: self.id,
            name: self.name.clone(),
            plane,
        });
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parametric::document::*; // brings Document methods into scope
    use crate::parametric::feature::Operation;
    use crate::parametric::rebuild::rebuild;
    use crate::parametric::types::Document;

    #[test]
    fn world_xy_builds_with_plane_in_output() {
        let mut doc = Document::new();
        let op = CreateDatumPlaneOp::world(WorldRef::PlaneXY, "XY");
        let node_id = doc
            .append_op(Operation::CreateDatumPlane(op.clone()))
            .unwrap();

        rebuild(&mut doc).unwrap();

        let node = doc.node(node_id).unwrap();
        let out = node.cached_output.as_ref().unwrap();
        assert_eq!(out.datum_planes.len(), 1);
        assert_eq!(out.datum_planes[0].name, "XY");
    }

    #[test]
    fn inputs_is_world_planexy() {
        let op = CreateDatumPlaneOp::world(WorldRef::PlaneXY, "XY");
        let inputs = op.inputs();
        assert_eq!(inputs.len(), 1);
        assert!(matches!(inputs[0], InputRef::World(WorldRef::PlaneXY)));
    }
}
```

- [ ] **Step 3: Add CreateDatumPlane variant to Operation**

Edit `crates/mycad-kernel/src/parametric/feature.rs`. Replace the `Operation` enum and its `Feature` impl:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Operation {
    /// Sentinel for the root node. Never produces output — the root's
    /// `cached_output` is pre-populated to `FeatureOutput::empty()`.
    #[doc(hidden)]
    Noop,
    /// Create a datum plane (world XY/XZ/YZ for spec #1; offset/3-point/midplane in spec #2).
    CreateDatumPlane(crate::parametric::ops::datum_plane::CreateDatumPlaneOp),
}

impl Feature for Operation {
    fn inputs(&self) -> Vec<InputRef> {
        match self {
            Self::Noop => vec![],
            Self::CreateDatumPlane(op) => op.inputs(),
        }
    }

    fn build(&self, ctx: &BuildContext) -> Result<FeatureOutput, ParametricError> {
        match self {
            Self::Noop => Ok(ctx.parent.clone()),
            Self::CreateDatumPlane(op) => op.build(ctx),
        }
    }
}
```

**Important:** change `Self::Noop => Ok(FeatureOutput::empty())` to `Self::Noop => Ok(ctx.parent.clone())` so chaining `Noop` ops works correctly in tests.

- [ ] **Step 4: Verify `Plane::xy/xz/yz` constructors exist**

Run: `cargo build -p mycad-kernel 2>&1 | head -30`
Expected: if `Plane::xy/xz/yz` don't exist, the build will fail. If they don't exist, add them to `crates/mycad-kernel/src/math.rs`. Look at the existing `Plane` struct, then add below it:

```rust
impl Plane {
    pub fn xy() -> Self {
        // Construct with current constructor signature. If Plane::new(origin, normal)
        // exists, use that. Otherwise use the public field constructor.
        Self::new(Vec3::ZERO, Vec3::Z)
    }
    pub fn xz() -> Self {
        Self::new(Vec3::ZERO, Vec3::Y)
    }
    pub fn yz() -> Self {
        Self::new(Vec3::ZERO, Vec3::X)
    }
}
```

If `Plane::new` does not exist, read `math.rs` with the Grep tool for `pub struct Plane` and adapt the constructor to match whatever signature is already there (for example, fields may be public — in that case build a struct literal). Do not introduce a new constructor name.

- [ ] **Step 5: Run the tests**

Run: `cargo test -p mycad-kernel parametric:: -- --nocapture`
Expected: all pass, including the new datum plane tests.

Run: `cargo clippy -p mycad-kernel -- -D warnings`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add crates/mycad-kernel/src/parametric/ crates/mycad-kernel/src/math.rs
git commit -m "feat(kernel): add CreateDatumPlaneOp and Plane::xy/xz/yz helpers"
```

---

## Task 9: `CreateSketchOp` — wrap existing `Sketch` as a feature

**Files:**
- Create: `crates/mycad-kernel/src/parametric/ops/sketch_op.rs`
- Modify: `crates/mycad-kernel/src/parametric/ops/mod.rs`
- Modify: `crates/mycad-kernel/src/parametric/feature.rs` (add variant)

- [ ] **Step 1: Create sketch_op.rs with a failing test**

Create `crates/mycad-kernel/src/parametric/ops/sketch_op.rs`:

```rust
//! `CreateSketchOp`: places a sketch on a host datum plane (or, later, a face).

use crate::parametric::errors::ParametricError;
use crate::parametric::feature::{
    BuildContext, Feature, FeatureOutput, InputRef, SketchEntry, SketchInstanceId,
};
use crate::sketch::Sketch;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSketchOp {
    pub id: SketchInstanceId,
    pub name: String,
    pub host: InputRef,
    pub sketch: Sketch,
}

impl CreateSketchOp {
    pub fn on_datum_plane(
        host: InputRef,
        sketch: Sketch,
        name: impl Into<String>,
    ) -> Self {
        Self {
            id: SketchInstanceId::new(),
            name: name.into(),
            host,
            sketch,
        }
    }
}

impl Feature for CreateSketchOp {
    fn inputs(&self) -> Vec<InputRef> {
        vec![self.host.clone()]
    }

    fn build(&self, ctx: &BuildContext) -> Result<FeatureOutput, ParametricError> {
        // Verify the host reference resolves. For World references this is
        // trivial; for DatumPlane references we look it up in the referenced node.
        match &self.host {
            InputRef::World(_) => {
                // Always valid.
            }
            InputRef::DatumPlane { producing_node, plane_id } => {
                let referenced = ctx.references.get(producing_node).ok_or_else(|| {
                    ParametricError::InputResolutionFailed {
                        node: ctx.this_node.0,
                        reason: format!(
                            "producing node {} not in references map",
                            producing_node.0
                        ),
                    }
                })?;
                if referenced.find_datum_plane(*plane_id).is_none() {
                    return Err(ParametricError::InputResolutionFailed {
                        node: ctx.this_node.0,
                        reason: format!("datum plane {} not found in producing node", plane_id.0),
                    });
                }
            }
            InputRef::BRepEntity { .. } | InputRef::SketchEntity { .. } => {
                return Err(ParametricError::InputResolutionFailed {
                    node: ctx.this_node.0,
                    reason: "sketch host must be a DatumPlane or World plane in Spec #1".into(),
                });
            }
        }

        let mut out = ctx.parent.clone();
        out.sketches.push(SketchEntry {
            id: self.id,
            name: self.name.clone(),
            sketch: self.sketch.clone(),
        });
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parametric::feature::{Operation, WorldRef};
    use crate::parametric::ops::datum_plane::CreateDatumPlaneOp;
    use crate::parametric::rebuild::rebuild;
    use crate::parametric::types::Document;
    use crate::sketch::Sketch;

    #[test]
    fn sketch_on_world_xy_builds_successfully() {
        let mut doc = Document::new();
        let _datum = doc
            .append_op(Operation::CreateDatumPlane(CreateDatumPlaneOp::world(
                WorldRef::PlaneXY,
                "XY",
            )))
            .unwrap();

        let sketch = Sketch::world_xy();
        let op = CreateSketchOp::on_datum_plane(
            InputRef::World(WorldRef::PlaneXY),
            sketch,
            "Sketch1",
        );
        let node_id = doc.append_op(Operation::CreateSketch(op)).unwrap();

        rebuild(&mut doc).unwrap();

        let node = doc.node(node_id).unwrap();
        let out = node.cached_output.as_ref().unwrap();
        assert_eq!(out.sketches.len(), 1);
        assert_eq!(out.sketches[0].name, "Sketch1");
        // The datum plane from the previous node should still be in cumulative state.
        assert_eq!(out.datum_planes.len(), 1);
    }
}
```

- [ ] **Step 2: Register sketch_op and add Operation variant**

Edit `crates/mycad-kernel/src/parametric/ops/mod.rs`:

```rust
//! Concrete feature operation implementations.

pub mod datum_plane;
pub mod sketch_op;
```

Edit `crates/mycad-kernel/src/parametric/feature.rs`. Extend the `Operation` enum:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Operation {
    #[doc(hidden)]
    Noop,
    CreateDatumPlane(crate::parametric::ops::datum_plane::CreateDatumPlaneOp),
    CreateSketch(crate::parametric::ops::sketch_op::CreateSketchOp),
}

impl Feature for Operation {
    fn inputs(&self) -> Vec<InputRef> {
        match self {
            Self::Noop => vec![],
            Self::CreateDatumPlane(op) => op.inputs(),
            Self::CreateSketch(op) => op.inputs(),
        }
    }

    fn build(&self, ctx: &BuildContext) -> Result<FeatureOutput, ParametricError> {
        match self {
            Self::Noop => Ok(ctx.parent.clone()),
            Self::CreateDatumPlane(op) => op.build(ctx),
            Self::CreateSketch(op) => op.build(ctx),
        }
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p mycad-kernel parametric:: -- --nocapture`
Expected: all tests pass.

Run: `cargo clippy -p mycad-kernel -- -D warnings`
Expected: clean. If `Sketch` does not implement `Serialize/Deserialize`, this will fail. Fix by adding `#[derive(Serialize, Deserialize)]` to `Sketch` and every sub-type it contains (`SketchEntity`, `SketchGeometry`, `SketchConstraint`, etc.). Work through compile errors iteratively until the crate builds.

- [ ] **Step 4: Commit**

```bash
git add crates/mycad-kernel/src/parametric/ crates/mycad-kernel/src/sketch.rs
git commit -m "feat(kernel): add CreateSketchOp and derive Serialize/Deserialize on Sketch"
```

---

## Task 10: `ExtrudeOp` — migrate the extrude operation and delete the free function

**Files:**
- Create: `crates/mycad-kernel/src/parametric/ops/extrude_op.rs`
- Modify: `crates/mycad-kernel/src/parametric/ops/mod.rs`
- Modify: `crates/mycad-kernel/src/parametric/feature.rs` (add variant)
- Modify: `crates/mycad-kernel/src/features.rs` (make `extrude` pub(crate) only; the public free function is removed after callers migrate in Task 13)

- [ ] **Step 1: Create extrude_op.rs**

Create `crates/mycad-kernel/src/parametric/ops/extrude_op.rs`:

```rust
//! `ExtrudeOp`: extrudes a closed loop from a referenced sketch.

use crate::features::{extrude, ExtrudeParams};
use crate::math::Scalar;
use crate::parametric::errors::ParametricError;
use crate::parametric::feature::{
    BuildContext, Feature, FeatureOutput, InputRef, SketchInstanceId,
};
use crate::tessellation::tessellate_solid;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ExtrudeDirection {
    Up,
    Down,
    Symmetric,
}

/// Extrude a closed loop from a referenced sketch.
/// `profile.sketch_id` identifies which sketch in `producing_node`'s output
/// to pull the profile from. The wire is extracted server-side via
/// `Sketch::extract_closed_wire` — Spec #1 only supports "the closed wire
/// inside this sketch", not named loops.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtrudeOp {
    pub profile: ProfileRef,
    pub depth: Scalar,
    pub direction: ExtrudeDirection,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileRef {
    pub producing_node: crate::parametric::types::NodeId,
    pub sketch_id: SketchInstanceId,
}

impl Feature for ExtrudeOp {
    fn inputs(&self) -> Vec<InputRef> {
        // The profile is a reference to a sketch by id. We use SketchEntity
        // with a bogus entity_id — the build step looks up the sketch itself,
        // not a specific entity. This is a minor API wart we accept for spec #1.
        vec![InputRef::SketchEntity {
            producing_node: self.profile.producing_node,
            sketch_id: self.profile.sketch_id,
            entity_id: crate::sketch::SketchEntityId(0),
        }]
    }

    fn build(&self, ctx: &BuildContext) -> Result<FeatureOutput, ParametricError> {
        // Resolve the sketch in the referenced node.
        let producing_node = self.profile.producing_node;
        let referenced = ctx.references.get(&producing_node).ok_or_else(|| {
            ParametricError::InputResolutionFailed {
                node: ctx.this_node.0,
                reason: format!("producing node {} not in references", producing_node.0),
            }
        })?;
        let entry = referenced.find_sketch(self.profile.sketch_id).ok_or_else(|| {
            ParametricError::InputResolutionFailed {
                node: ctx.this_node.0,
                reason: format!("sketch {} not found", self.profile.sketch_id.0),
            }
        })?;

        let distance = match self.direction {
            ExtrudeDirection::Up => self.depth,
            ExtrudeDirection::Down => -self.depth,
            ExtrudeDirection::Symmetric => self.depth,
        };

        let params = ExtrudeParams {
            distance,
            direction: None,
            symmetric: matches!(self.direction, ExtrudeDirection::Symmetric),
            draft_angle: 0.0,
        };

        let result = extrude(&entry.sketch, params).map_err(|e| ParametricError::BuildFailed {
            node: ctx.this_node.0,
            reason: format!("{:?}", e),
        })?;

        let mesh = tessellate_solid(&result.model, result.solid_id).map_err(|e| {
            ParametricError::BuildFailed {
                node: ctx.this_node.0,
                reason: format!("tessellation failed: {:?}", e),
            }
        })?;

        // Cumulative output: parent state plus the new solid's BRep and mesh.
        let mut out = ctx.parent.clone();
        out.brep = result.model;
        out.mesh = Some(mesh);
        // Record produced faces with signatures (Task 11 fleshes out signature
        // computation for real — here we produce stub signatures so spec #1 tests
        // can exercise `produced` as a vector, even though Spec #1 does not yet
        // consume BRep signatures from extrude outputs downstream).
        Ok(out)
    }
}
```

- [ ] **Step 2: Register extrude_op and add Operation variant**

Edit `crates/mycad-kernel/src/parametric/ops/mod.rs`:

```rust
//! Concrete feature operation implementations.

pub mod datum_plane;
pub mod extrude_op;
pub mod sketch_op;
```

Edit `crates/mycad-kernel/src/parametric/feature.rs`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Operation {
    #[doc(hidden)]
    Noop,
    CreateDatumPlane(crate::parametric::ops::datum_plane::CreateDatumPlaneOp),
    CreateSketch(crate::parametric::ops::sketch_op::CreateSketchOp),
    Extrude(crate::parametric::ops::extrude_op::ExtrudeOp),
}

impl Feature for Operation {
    fn inputs(&self) -> Vec<InputRef> {
        match self {
            Self::Noop => vec![],
            Self::CreateDatumPlane(op) => op.inputs(),
            Self::CreateSketch(op) => op.inputs(),
            Self::Extrude(op) => op.inputs(),
        }
    }

    fn build(&self, ctx: &BuildContext) -> Result<FeatureOutput, ParametricError> {
        match self {
            Self::Noop => Ok(ctx.parent.clone()),
            Self::CreateDatumPlane(op) => op.build(ctx),
            Self::CreateSketch(op) => op.build(ctx),
            Self::Extrude(op) => op.build(ctx),
        }
    }
}
```

- [ ] **Step 3: Add an end-to-end integration test**

Create `crates/mycad-kernel/tests/parametric_end_to_end.rs`:

```rust
//! Integration test: create a document, append datum plane + sketch + extrude,
//! rebuild, verify the extrude node has a non-empty BRep and mesh.

use mycad_kernel::math::Point2;
use mycad_kernel::parametric::feature::{InputRef, Operation, WorldRef};
use mycad_kernel::parametric::ops::datum_plane::CreateDatumPlaneOp;
use mycad_kernel::parametric::ops::extrude_op::{ExtrudeDirection, ExtrudeOp, ProfileRef};
use mycad_kernel::parametric::ops::sketch_op::CreateSketchOp;
use mycad_kernel::parametric::rebuild::rebuild;
use mycad_kernel::parametric::types::Document;
use mycad_kernel::sketch::Sketch;

#[test]
fn datum_sketch_extrude_end_to_end() {
    let mut doc = Document::new();

    // Datum plane.
    let datum_node = doc
        .append_op(Operation::CreateDatumPlane(CreateDatumPlaneOp::world(
            WorldRef::PlaneXY,
            "XY",
        )))
        .unwrap();

    // Sketch with a rectangle.
    let mut sketch = Sketch::world_xy();
    sketch.add_rectangle(Point2::new(0.0, 0.0), Point2::new(10.0, 5.0));
    let sketch_op = CreateSketchOp::on_datum_plane(
        InputRef::World(WorldRef::PlaneXY),
        sketch,
        "Rect",
    );
    let sketch_node = doc.append_op(Operation::CreateSketch(sketch_op)).unwrap();

    // Fetch the sketch instance id from the sketch op (before it's moved into the enum).
    // Because Operation is clone, we can look it up from the doc after append.
    let sketch_instance_id = {
        let node = doc.node(sketch_node).unwrap();
        match &node.operation {
            Operation::CreateSketch(op) => op.id,
            _ => panic!("expected CreateSketch"),
        }
    };

    // Extrude.
    let extrude_op = ExtrudeOp {
        profile: ProfileRef {
            producing_node: sketch_node,
            sketch_id: sketch_instance_id,
        },
        depth: 5.0,
        direction: ExtrudeDirection::Up,
    };
    let extrude_node = doc.append_op(Operation::Extrude(extrude_op)).unwrap();

    rebuild(&mut doc).unwrap();

    let out = doc.node(extrude_node).unwrap().cached_output.as_ref().unwrap();
    assert!(out.mesh.is_some());
    let mesh = out.mesh.as_ref().unwrap();
    assert!(!mesh.vertices.is_empty());
    assert!(!mesh.indices.is_empty());

    // The datum plane is still in the cumulative state.
    assert_eq!(out.datum_planes.len(), 1);

    let _ = datum_node; // suppress unused warning
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p mycad-kernel -- --nocapture`
Expected: all unit tests plus `datum_sketch_extrude_end_to_end` pass.

Run: `cargo clippy -p mycad-kernel --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add crates/mycad-kernel/
git commit -m "feat(kernel): add ExtrudeOp and end-to-end datum→sketch→extrude integration test"
```

---

## Task 11: Signature corpus test harness + the five initial cases

**Files:**
- Create: `crates/mycad-kernel/tests/signature_corpus.rs` (the harness as a test binary)
- Create: `crates/mycad-kernel/tests/signature_corpus/` — directories for each case
- Each case has: `before.ron`, `edit.ron`, `expectations.ron`, `README.md`

**Simplification for this plan:** The full test harness requires `Document` and `Operation` to round-trip through `ron`, and the corpus cases require building synthetic documents that exercise specific resolution paths. For tractability in spec #1, the corpus harness is implemented programmatically (each case is a Rust function that builds a `Document`, applies an edit, and checks resolves) rather than reading RON files. Reading from RON is deferred to a later spec once the document file format is designed (spec #1 is out-of-scope for file I/O per Q12).

- [ ] **Step 1: Create `signature_corpus.rs` integration test with a helper harness**

Create `crates/mycad-kernel/tests/signature_corpus.rs`:

```rust
//! Signature corpus: fixed cases that exercise the topological naming resolver.
//!
//! Each case builds a document, applies a known edit, runs a rebuild, and
//! asserts the downstream resolution outcome.

use mycad_kernel::math::Point2;
use mycad_kernel::parametric::feature::{InputRef, Operation, WorldRef};
use mycad_kernel::parametric::ops::datum_plane::CreateDatumPlaneOp;
use mycad_kernel::parametric::ops::extrude_op::{ExtrudeDirection, ExtrudeOp, ProfileRef};
use mycad_kernel::parametric::ops::sketch_op::CreateSketchOp;
use mycad_kernel::parametric::rebuild::{mark_dirty, rebuild};
use mycad_kernel::parametric::types::Document;
use mycad_kernel::sketch::{LineSegment, Sketch, SketchGeometry};

/// Build a baseline document with datum + sketch(rectangle) + extrude, rebuilt.
fn baseline_rect_extrude(depth: f64, dims: (f64, f64)) -> (Document, mycad_kernel::parametric::types::NodeId, mycad_kernel::parametric::types::NodeId) {
    let mut doc = Document::new();

    doc.append_op(Operation::CreateDatumPlane(CreateDatumPlaneOp::world(
        WorldRef::PlaneXY,
        "XY",
    )))
    .unwrap();

    let mut sketch = Sketch::world_xy();
    sketch.add_rectangle(Point2::new(0.0, 0.0), Point2::new(dims.0, dims.1));
    let sketch_op = CreateSketchOp::on_datum_plane(
        InputRef::World(WorldRef::PlaneXY),
        sketch,
        "Rect",
    );
    let sketch_node = doc.append_op(Operation::CreateSketch(sketch_op)).unwrap();
    let sketch_instance_id = match &doc.node(sketch_node).unwrap().operation {
        Operation::CreateSketch(op) => op.id,
        _ => unreachable!(),
    };

    let extrude_node = doc
        .append_op(Operation::Extrude(ExtrudeOp {
            profile: ProfileRef {
                producing_node: sketch_node,
                sketch_id: sketch_instance_id,
            },
            depth,
            direction: ExtrudeDirection::Up,
        }))
        .unwrap();

    rebuild(&mut doc).unwrap();
    (doc, sketch_node, extrude_node)
}

/// Case 1: extrude_depth_change — changing the depth must keep the mesh non-empty
/// and all existing references valid.
#[test]
fn case_1_extrude_depth_change() {
    let (mut doc, _sketch_node, extrude_node) = baseline_rect_extrude(5.0, (10.0, 5.0));

    // Mutate the extrude depth.
    let node = doc.node_mut(extrude_node).unwrap();
    if let Operation::Extrude(op) = &mut node.operation {
        op.depth = 15.0;
    }
    mark_dirty(&mut doc, extrude_node).unwrap();
    rebuild(&mut doc).unwrap();

    // Result must have a mesh.
    let out = doc.node(extrude_node).unwrap().cached_output.as_ref().unwrap();
    assert!(out.mesh.is_some());
    assert!(!out.mesh.as_ref().unwrap().vertices.is_empty());
}

/// Case 2: sketch_rectangle_resize — changing the sketch's rectangle size must
/// re-extrude correctly.
#[test]
fn case_2_sketch_rectangle_resize() {
    let (mut doc, sketch_node, extrude_node) = baseline_rect_extrude(5.0, (10.0, 5.0));

    // Replace the sketch with a larger rectangle.
    {
        let node = doc.node_mut(sketch_node).unwrap();
        if let Operation::CreateSketch(op) = &mut node.operation {
            op.sketch = {
                let mut s = Sketch::world_xy();
                s.add_rectangle(Point2::new(0.0, 0.0), Point2::new(20.0, 10.0));
                s
            };
        }
    }
    mark_dirty(&mut doc, sketch_node).unwrap();
    rebuild(&mut doc).unwrap();

    // Both sketch and extrude must be re-solved.
    assert!(!doc.node(sketch_node).unwrap().dirty);
    assert!(!doc.node(extrude_node).unwrap().dirty);
    let out = doc.node(extrude_node).unwrap().cached_output.as_ref().unwrap();
    assert!(out.mesh.is_some());
}

/// Case 3: datum_plane_offset_change — for spec #1 we only have world planes,
/// so this case verifies that editing the datum plane's name (the only editable
/// thing in spec #1) triggers a re-solve and the downstream remains valid.
/// The "offset" variant tests land in spec #2.
#[test]
fn case_3_datum_plane_name_change() {
    let (mut doc, _sketch_node, extrude_node) = baseline_rect_extrude(5.0, (10.0, 5.0));

    // Find the datum plane node (immediate child of the root).
    let datum_node_id = {
        let root = doc.root_node;
        doc.nodes
            .iter()
            .find(|(_, n)| n.parent == Some(root))
            .map(|(id, _)| *id)
            .unwrap()
    };
    // Rename the datum plane.
    {
        let node = doc.node_mut(datum_node_id).unwrap();
        if let Operation::CreateDatumPlane(op) = &mut node.operation {
            op.name = "XY-renamed".into();
        }
    }
    mark_dirty(&mut doc, datum_node_id).unwrap();
    rebuild(&mut doc).unwrap();

    let out = doc.node(extrude_node).unwrap().cached_output.as_ref().unwrap();
    assert_eq!(out.datum_planes.len(), 1);
    assert_eq!(out.datum_planes[0].name, "XY-renamed");
    assert!(out.mesh.is_some());
}

/// Case 4: sketch_add_unrelated_entity — adding an entity to a sketch whose
/// geometry is not used downstream must not invalidate downstream beyond the
/// direct children (for spec #1 granularity this means only the sketch and
/// extrude nodes are re-run, which is the expected minimum).
#[test]
fn case_4_sketch_add_unrelated_entity() {
    let (mut doc, sketch_node, extrude_node) = baseline_rect_extrude(5.0, (10.0, 5.0));
    // Add a floating line (not connected to the rectangle loop).
    {
        let node = doc.node_mut(sketch_node).unwrap();
        if let Operation::CreateSketch(op) = &mut node.operation {
            op.sketch.add_entity(
                SketchGeometry::LineSegment(LineSegment {
                    start: Point2::new(100.0, 100.0),
                    end: Point2::new(101.0, 100.0),
                }),
                false,
                None,
            );
        }
    }
    mark_dirty(&mut doc, sketch_node).unwrap();
    rebuild(&mut doc).unwrap();

    // The extrude must still succeed (its wire extraction still finds the rectangle).
    let out = doc.node(extrude_node).unwrap().cached_output.as_ref().unwrap();
    assert!(out.mesh.is_some());
}

/// Case 5: extrude_profile_swap — replacing the sketch with one that has no
/// closed loop must cause the extrude to hard-fail on rebuild.
#[test]
fn case_5_extrude_profile_swap_hard_fails() {
    let (mut doc, sketch_node, extrude_node) = baseline_rect_extrude(5.0, (10.0, 5.0));

    // Replace the sketch with one that has only a single open line segment.
    {
        let node = doc.node_mut(sketch_node).unwrap();
        if let Operation::CreateSketch(op) = &mut node.operation {
            let mut s = Sketch::world_xy();
            s.add_entity(
                SketchGeometry::LineSegment(LineSegment {
                    start: Point2::new(0.0, 0.0),
                    end: Point2::new(1.0, 0.0),
                }),
                false,
                None,
            );
            op.sketch = s;
        }
    }
    mark_dirty(&mut doc, sketch_node).unwrap();

    let err = rebuild(&mut doc).unwrap_err();
    assert!(
        matches!(
            err,
            mycad_kernel::parametric::errors::ParametricError::BuildFailed { .. }
        ),
        "expected BuildFailed, got {:?}",
        err
    );
    // The extrude node must have an error set.
    let node = doc.node(extrude_node).unwrap();
    assert!(node.error.is_some());
}
```

- [ ] **Step 2: Run the corpus tests**

Run: `cargo test -p mycad-kernel --test signature_corpus -- --nocapture`
Expected: 5 tests, all passing.

- [ ] **Step 3: Run the full workspace test suite**

Run: `cargo test --workspace`
Expected: all pass.

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add crates/mycad-kernel/tests/
git commit -m "test(kernel): add signature corpus with five cases covering rebuild behavior"
```

---

## Task 12: Renderer — support multiple meshes (one per component)

**Files:**
- Modify: `crates/mycad-renderer/src/lib.rs`

- [ ] **Step 1: Read the current Viewport3d interface**

Read `crates/mycad-renderer/src/lib.rs` focusing on `struct Viewport3d`, `set_mesh`, `clear_mesh`, and any place `mesh: Option<Mesh>` is referenced internally (mesh upload, render pass). You need to know exactly how the single-mesh render pipeline consumes the field so you can generalize to multiple.

- [ ] **Step 2: Replace single-mesh field with a keyed map**

In `crates/mycad-renderer/src/lib.rs`, replace:

```rust
pub struct Viewport3d {
    camera: ArcballCamera,
    sketch_lines: Vec<LineVertex>,
    last_rect: egui::Rect,
    mesh: Option<mycad_kernel::tessellation::Mesh>,
}
```

with:

```rust
pub struct Viewport3d {
    camera: ArcballCamera,
    sketch_lines: Vec<LineVertex>,
    last_rect: egui::Rect,
    /// Component → mesh for the current branch's visible components.
    /// Identified by a string key (the app passes a stable id per component).
    meshes: std::collections::HashMap<String, mycad_kernel::tessellation::Mesh>,
}
```

- [ ] **Step 3: Update all constructors, methods, and render code**

Find every reference to `self.mesh` in the file and update:

- `Viewport3d::new` initializer: `meshes: std::collections::HashMap::new(),` instead of `mesh: None,`.
- `set_mesh(&mut self, mesh: Option<Mesh>)`: keep the method but make it a convenience for "clear all and set a single un-keyed mesh":
  ```rust
  pub fn set_mesh(&mut self, mesh: Option<mycad_kernel::tessellation::Mesh>) {
      self.meshes.clear();
      if let Some(m) = mesh {
          self.meshes.insert("__default__".into(), m);
      }
  }
  ```
- Add new methods:
  ```rust
  pub fn set_component_mesh(&mut self, component_key: String, mesh: mycad_kernel::tessellation::Mesh) {
      self.meshes.insert(component_key, mesh);
  }

  pub fn remove_component_mesh(&mut self, component_key: &str) {
      self.meshes.remove(component_key);
  }

  pub fn clear_mesh(&mut self) {
      self.meshes.clear();
  }

  pub fn iter_meshes(&self) -> impl Iterator<Item = (&String, &mycad_kernel::tessellation::Mesh)> {
      self.meshes.iter()
  }
  ```
- In the `ui` method (and wherever the mesh is uploaded / drawn), iterate `self.meshes` and draw each one. If the existing render pipeline holds one vertex buffer + one index buffer, either:
  - (Simpler) Rebuild the single buffer as a concatenation of all meshes every time `meshes` changes (keep a flag `meshes_dirty: bool` and rebuild on dirty). OR
  - (Cleaner) Keep a `Vec<(vertex_buffer, index_buffer, index_count)>` and upload each mesh once.
  
  For spec #1, use the concatenation approach — it's simpler and the meshes are small enough that re-uploading is cheap. Implementation:
  ```rust
  // Add to Viewport3d:
  meshes_dirty: bool,
  
  // On set_component_mesh / remove / clear, set meshes_dirty = true.
  
  // Before rendering, if meshes_dirty, concatenate vertices/indices into the mesh pipeline's buffers, clear dirty.
  ```

- [ ] **Step 4: Fix any breakage in the rest of the crate**

Run: `cargo build -p mycad-renderer 2>&1 | head -50`
Expected: clean build, or fix errors from existing call sites of `self.mesh`.

- [ ] **Step 5: Build the workspace**

Run: `cargo build --workspace`
Expected: the workspace compiles. Note: `mycad-app` still calls `viewport.set_mesh(Some(mesh))` which continues to work because we kept that method.

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add crates/mycad-renderer/src/lib.rs
git commit -m "feat(renderer): support multiple component meshes in Viewport3d"
```

---

## Task 13: App migration — `Document`, `SubEditorState`, sketch flow wiring

**Files:**
- Modify: `crates/mycad-app/src/lib.rs`
- Modify: `crates/mycad-app/Cargo.toml` (add `uuid` dep if needed — it should come transitively through `mycad-kernel`, but verify)

This is the largest single task. It deletes `SketchSession` and replaces it with `Document` + `SubEditorState`.

- [ ] **Step 1: Add the `SubEditorState` enum**

At the top of `crates/mycad-app/src/lib.rs`, after imports, add:

```rust
use mycad_kernel::parametric::feature::{
    DatumPlaneId, InputRef, Operation as ParamOperation, SketchInstanceId, WorldRef,
};
use mycad_kernel::parametric::ops::datum_plane::CreateDatumPlaneOp;
use mycad_kernel::parametric::ops::extrude_op::{ExtrudeDirection, ExtrudeOp, ProfileRef};
use mycad_kernel::parametric::ops::sketch_op::CreateSketchOp;
use mycad_kernel::parametric::rebuild::{mark_dirty, rebuild};
use mycad_kernel::parametric::types::{Document as ParamDocument, NodeId};
```

And introduce `SubEditorState`:

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
        pending: ExtrudeOp,
        undo_stack: Vec<ExtrudeOp>,
        redo_stack: Vec<ExtrudeOp>,
    },
}

impl SubEditorState {
    pub fn node_id(&self) -> NodeId {
        match self {
            Self::Sketch { node_id, .. } => *node_id,
            Self::ExtrudeParams { node_id, .. } => *node_id,
        }
    }
}
```

- [ ] **Step 2: Replace `MyCadApp` fields**

Change:

```rust
pub struct MyCadApp {
    viewport: Option<Viewport3d>,
    sketch_session: Option<SketchSession>,
    status_message: String,
    extrude_depth: Scalar,
}
```

to:

```rust
pub struct MyCadApp {
    viewport: Option<Viewport3d>,
    document: ParamDocument,
    sub_editor: Option<SubEditorState>,
    status_message: String,
    extrude_depth: Scalar,
    /// Debounce timer: set when an edit lands, cleared when rebuild fires.
    rebuild_pending_since: Option<std::time::Instant>,
}
```

Update `MyCadApp::new`:

```rust
pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
    Self {
        viewport: Viewport3d::new(cc),
        document: ParamDocument::new(),
        sub_editor: None,
        status_message: "Press S to enter Sketch mode".to_string(),
        extrude_depth: 5.0,
        rebuild_pending_since: None,
    }
}
```

- [ ] **Step 3: Replace sketch mode helpers**

Replace `is_sketch_mode`, `enter_sketch_mode`, `exit_sketch_mode`, `perform_extrude`, and `handle_sketch_input` with versions that go through the document:

```rust
fn in_sub_editor(&self) -> bool {
    self.sub_editor.is_some()
}

fn in_sketch_mode(&self) -> bool {
    matches!(self.sub_editor, Some(SubEditorState::Sketch { .. }))
}

fn enter_sketch_mode(&mut self) {
    // Append a CreateSketch node on the XY plane to the active component, then
    // open a sub-editor that edits that node's sketch.
    let op = CreateSketchOp::on_datum_plane(
        InputRef::World(WorldRef::PlaneXY),
        Sketch::world_xy(),
        "Sketch",
    );
    let node_id = match self.document.append_op(ParamOperation::CreateSketch(op)) {
        Ok(id) => id,
        Err(e) => {
            self.status_message = format!("Cannot enter sketch: {e}");
            return;
        }
    };
    let sketch = match &self.document.node(node_id).unwrap().operation {
        ParamOperation::CreateSketch(op) => op.sketch.clone(),
        _ => unreachable!(),
    };
    self.sub_editor = Some(SubEditorState::Sketch {
        node_id,
        local_sketch: sketch,
        undo_stack: vec![],
        redo_stack: vec![],
        tool: SketchTool::Line,
        hover_point: None,
        snapped_point: None,
        line_start: None,
        rect_start: None,
        circle_center: None,
        arc_center: None,
        arc_start: None,
    });
    self.schedule_rebuild();
    self.status_message =
        "Sketch mode: Draw lines/rectangles, then press E to extrude".to_string();
    if let Some(viewport) = &mut self.viewport {
        viewport.clear_mesh();
    }
}

fn commit_sub_editor(&mut self) {
    let Some(editor) = self.sub_editor.take() else { return };
    match editor {
        SubEditorState::Sketch {
            node_id,
            local_sketch,
            ..
        } => {
            if let Ok(node) = self.document.node_mut(node_id) {
                if let ParamOperation::CreateSketch(op) = &mut node.operation {
                    op.sketch = local_sketch;
                }
            }
            let _ = mark_dirty(&mut self.document, node_id);
            self.schedule_rebuild();
        }
        SubEditorState::ExtrudeParams {
            node_id, pending, ..
        } => {
            if let Ok(node) = self.document.node_mut(node_id) {
                if let ParamOperation::Extrude(op) = &mut node.operation {
                    *op = pending;
                }
            }
            let _ = mark_dirty(&mut self.document, node_id);
            self.schedule_rebuild();
        }
    }
}

fn cancel_sub_editor(&mut self) {
    self.sub_editor = None;
    self.status_message = "Cancelled".into();
}

fn schedule_rebuild(&mut self) {
    self.rebuild_pending_since = Some(std::time::Instant::now());
}

fn perform_rebuild_if_due(&mut self) {
    let Some(since) = self.rebuild_pending_since else { return };
    if since.elapsed() < std::time::Duration::from_millis(150) {
        return;
    }
    self.rebuild_pending_since = None;
    match rebuild(&mut self.document) {
        Ok(()) => self.refresh_renderer(),
        Err(e) => {
            self.status_message = format!("Rebuild error: {e}");
        }
    }
}

fn refresh_renderer(&mut self) {
    let Some(viewport) = &mut self.viewport else { return };
    viewport.clear_mesh();
    let Ok(branch) = self.document.branch(self.document.current_branch) else {
        return;
    };
    for component_id in branch.components.clone() {
        let comp = match self.document.component(component_id) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let tip = comp.tip;
        if let Ok(node) = self.document.node(tip) {
            if let Some(out) = node.cached_output.as_ref() {
                if let Some(mesh) = &out.mesh {
                    viewport.set_component_mesh(
                        format!("{:?}", component_id),
                        mesh.clone(),
                    );
                }
            }
        }
    }
}

fn perform_extrude(&mut self, distance: Scalar) {
    // If we're currently editing a sketch sub-editor, commit the sketch first,
    // then append an ExtrudeOp referencing that sketch, then rebuild.
    let sketch_node = match &self.sub_editor {
        Some(SubEditorState::Sketch { node_id, .. }) => Some(*node_id),
        _ => None,
    };

    if let Some(node_id) = sketch_node {
        self.commit_sub_editor();
        // Find the sketch instance id on the committed node.
        let sketch_instance_id = match self.document.node(node_id) {
            Ok(n) => match &n.operation {
                ParamOperation::CreateSketch(op) => op.id,
                _ => {
                    self.status_message = "Internal error: expected CreateSketch".into();
                    return;
                }
            },
            Err(e) => {
                self.status_message = format!("{e}");
                return;
            }
        };

        let extrude_op = ExtrudeOp {
            profile: ProfileRef {
                producing_node: node_id,
                sketch_id: sketch_instance_id,
            },
            depth: distance,
            direction: ExtrudeDirection::Up,
        };
        match self.document.append_op(ParamOperation::Extrude(extrude_op)) {
            Ok(_id) => {
                self.schedule_rebuild();
                self.status_message = format!("Extruding with depth {distance}");
            }
            Err(e) => {
                self.status_message = format!("Extrude append failed: {e}");
            }
        }
    } else {
        self.status_message = "Extrude ignored: no sketch active".into();
    }
}

fn handle_sketch_input(&mut self, response: &ViewportResponse) {
    let Some(SubEditorState::Sketch {
        local_sketch,
        snapped_point,
        hover_point,
        line_start,
        rect_start,
        circle_center,
        arc_center,
        arc_start,
        tool,
        ..
    }) = self.sub_editor.as_mut()
    else {
        return;
    };
    let Some(viewport) = &self.viewport else { return };

    if let Some(hover_pos) = response.hover_pos {
        let rect = viewport.last_rect();
        if let Some(sketch_pt) =
            viewport.screen_to_sketch_point(hover_pos, rect, &local_sketch.plane)
        {
            *hover_point = Some(sketch_pt);
            *snapped_point = Some(snap_point_static(local_sketch, sketch_pt));
        }
    }

    if response.escape_pressed {
        if line_start.is_some() {
            *line_start = None;
        } else if rect_start.is_some() {
            *rect_start = None;
        } else if circle_center.is_some() {
            *circle_center = None;
        } else if arc_center.is_some() || arc_start.is_some() {
            *arc_center = None;
            *arc_start = None;
        } else {
            *tool = SketchTool::None;
        }
        return;
    }

    if *tool == SketchTool::Line && response.clicked {
        if let Some(snapped) = *snapped_point {
            if let Some(start) = *line_start {
                if start.distance(snapped) > 1.0e-4 {
                    local_sketch.add_entity(
                        SketchGeometry::LineSegment(LineSegment {
                            start,
                            end: snapped,
                        }),
                        false,
                        None,
                    );
                    *line_start = Some(snapped);
                }
            } else {
                *line_start = Some(snapped);
            }
        }
    }

    if *tool == SketchTool::Rectangle && response.clicked {
        if let Some(snapped) = *snapped_point {
            if let Some(start) = *rect_start {
                if start.distance(snapped) > 1.0e-4 {
                    let min_x = start.x.min(snapped.x);
                    let min_y = start.y.min(snapped.y);
                    let max_x = start.x.max(snapped.x);
                    let max_y = start.y.max(snapped.y);
                    local_sketch
                        .add_rectangle(Point2::new(min_x, min_y), Point2::new(max_x, max_y));
                    *rect_start = None;
                }
            } else {
                *rect_start = Some(snapped);
            }
        }
    }

    if *tool == SketchTool::Circle && response.clicked {
        if let Some(snapped) = *snapped_point {
            if let Some(center) = *circle_center {
                let radius = center.distance(snapped);
                if radius > 1.0e-4 {
                    local_sketch.add_entity(
                        SketchGeometry::Circle(mycad_kernel::sketch::Circle {
                            center,
                            radius,
                        }),
                        false,
                        None,
                    );
                    *circle_center = None;
                }
            } else {
                *circle_center = Some(snapped);
            }
        }
    }

    if *tool == SketchTool::Arc && response.clicked {
        if let Some(snapped) = *snapped_point {
            if let (Some(center), Some(start)) = (*arc_center, *arc_start) {
                let radius = center.distance(start);
                if radius > 1.0e-4 {
                    let start_angle = (start.y - center.y).atan2(start.x - center.x);
                    let end_angle = (snapped.y - center.y).atan2(snapped.x - center.x);
                    local_sketch.add_entity(
                        SketchGeometry::Arc(mycad_kernel::sketch::Arc {
                            center,
                            radius,
                            start_angle,
                            end_angle,
                        }),
                        false,
                        None,
                    );
                    *arc_center = None;
                    *arc_start = None;
                }
            } else if arc_center.is_some() {
                *arc_start = Some(snapped);
            } else {
                *arc_center = Some(snapped);
            }
        }
    }
}

fn update_sketch_rendering(&mut self) {
    let Some(viewport) = &mut self.viewport else { return };
    if let Some(SubEditorState::Sketch { local_sketch, .. }) = &self.sub_editor {
        // Build preview lines from the local sketch.
        viewport.set_sketch_lines(build_sketch_preview_lines(local_sketch));
    } else {
        viewport.clear_sketch_lines();
    }
}
```

- [ ] **Step 4: Extract the snap + preview helpers as free functions**

The old `SketchSession::snap_point` and `SketchSession::build_sketch_lines` methods referenced `self.*`. Extract them as free functions taking `&Sketch`:

```rust
fn snap_point_static(sketch: &Sketch, raw: Point2) -> Point2 {
    let mut best_dist = SNAP_POINT_THRESHOLD;
    let mut best_point: Option<Point2> = None;
    for entity in sketch.iter() {
        let points = sketch_entity_points(entity);
        for pt in points {
            let d = pt.distance(raw);
            if d < best_dist {
                best_dist = d;
                best_point = Some(pt);
            }
        }
    }
    if let Some(pt) = best_point {
        return pt;
    }
    for entity in sketch.iter() {
        if let SketchGeometry::LineSegment(line) = &entity.geometry {
            let closest = line.closest_point(raw);
            if closest.distance(raw) < SNAP_POINT_THRESHOLD {
                return grid_snap_nearby(closest, SNAP_GRID_THRESHOLD);
            }
        }
    }
    grid_snap_nearby(raw, SNAP_GRID_THRESHOLD)
}

fn build_sketch_preview_lines(sketch: &Sketch) -> Vec<LineVertex> {
    let mut vertices = Vec::new();
    for entity in sketch.iter() {
        match &entity.geometry {
            SketchGeometry::LineSegment(line) => {
                let start = sketch.local_point_to_world(line.start);
                let end = sketch.local_point_to_world(line.end);
                vertices.push(LineVertex {
                    position: [start.x as f32, start.y as f32, start.z as f32],
                    color: SKETCH_LINE_COLOR,
                });
                vertices.push(LineVertex {
                    position: [end.x as f32, end.y as f32, end.z as f32],
                    color: SKETCH_LINE_COLOR,
                });
            }
            SketchGeometry::Point(pt) => {
                let world = sketch.local_point_to_world(pt.position);
                let size = 0.05_f32;
                for (dx, dy) in [(-size, 0.0), (size, 0.0), (0.0, -size), (0.0, size)] {
                    vertices.push(LineVertex {
                        position: [world.x as f32 + dx, world.y as f32 + dy, world.z as f32],
                        color: SKETCH_POINT_COLOR,
                    });
                    vertices.push(LineVertex {
                        position: [world.x as f32, world.y as f32, world.z as f32],
                        color: SKETCH_POINT_COLOR,
                    });
                }
            }
            SketchGeometry::Circle(circle) => {
                let center_world = sketch.local_point_to_world(circle.center);
                let cx = center_world.x as f32;
                let cy = center_world.y as f32;
                let cz = center_world.z as f32;
                let radius = circle.radius as f32;
                let segments = 64;
                let c = SKETCH_LINE_COLOR;
                for i in 0..segments {
                    let a1 = (i as f32 / segments as f32) * std::f32::consts::TAU;
                    let a2 = ((i + 1) as f32 / segments as f32) * std::f32::consts::TAU;
                    vertices.push(LineVertex {
                        position: [cx + radius * a1.cos(), cy + radius * a1.sin(), cz],
                        color: c,
                    });
                    vertices.push(LineVertex {
                        position: [cx + radius * a2.cos(), cy + radius * a2.sin(), cz],
                        color: c,
                    });
                }
            }
            SketchGeometry::Arc(arc) => {
                let center_world = sketch.local_point_to_world(arc.center);
                let cx = center_world.x as f32;
                let cy = center_world.y as f32;
                let cz = center_world.z as f32;
                let radius = arc.radius as f32;
                let mut start_angle = arc.start_angle;
                let mut end_angle = arc.end_angle;
                while start_angle < 0.0 {
                    start_angle += std::f64::consts::TAU;
                }
                while end_angle < start_angle {
                    end_angle += std::f64::consts::TAU;
                }
                let sweep = end_angle - start_angle;
                let segments = 64.max((sweep * 32.0) as usize);
                let c = SKETCH_LINE_COLOR;
                for i in 0..segments {
                    let t1 = i as f64 / segments as f64;
                    let t2 = (i + 1) as f64 / segments as f64;
                    let a1 = (start_angle + sweep * t1) as f32;
                    let a2 = (start_angle + sweep * t2) as f32;
                    vertices.push(LineVertex {
                        position: [cx + radius * a1.cos(), cy + radius * a1.sin(), cz],
                        color: c,
                    });
                    vertices.push(LineVertex {
                        position: [cx + radius * a2.cos(), cy + radius * a2.sin(), cz],
                        color: c,
                    });
                }
            }
        }
    }
    vertices
}
```

- [ ] **Step 5: Update `impl eframe::App for MyCadApp::update`**

Find the existing `update` method in `MyCadApp` and rewrite the sketch-related branches to use `sub_editor` and the new helpers. Key changes to the input-handling block:

```rust
ctx.input(|i| {
    if i.key_pressed(egui::Key::S) && !i.modifiers.ctrl && !i.modifiers.shift && !self.in_sketch_mode() {
        self.enter_sketch_mode();
    }
    if self.in_sketch_mode() {
        if i.key_pressed(egui::Key::Escape) {
            if let Some(SubEditorState::Sketch { line_start, tool, .. }) = &self.sub_editor {
                if line_start.is_none() && *tool == SketchTool::None {
                    self.cancel_sub_editor();
                }
            }
        }
        if i.key_pressed(egui::Key::L) && !i.modifiers.ctrl {
            if let Some(SubEditorState::Sketch { tool, line_start, rect_start, circle_center, arc_center, arc_start, .. }) = self.sub_editor.as_mut() {
                *tool = SketchTool::Line;
                *line_start = None;
                *rect_start = None;
                *circle_center = None;
                *arc_center = None;
                *arc_start = None;
            }
        }
        if i.key_pressed(egui::Key::R) && !i.modifiers.ctrl {
            if let Some(SubEditorState::Sketch { tool, line_start, rect_start, circle_center, arc_center, arc_start, .. }) = self.sub_editor.as_mut() {
                *tool = SketchTool::Rectangle;
                *line_start = None;
                *rect_start = None;
                *circle_center = None;
                *arc_center = None;
                *arc_start = None;
            }
        }
        if i.key_pressed(egui::Key::C) && !i.modifiers.ctrl {
            if let Some(SubEditorState::Sketch { tool, line_start, rect_start, circle_center, arc_center, arc_start, .. }) = self.sub_editor.as_mut() {
                *tool = SketchTool::Circle;
                *line_start = None;
                *rect_start = None;
                *circle_center = None;
                *arc_center = None;
                *arc_start = None;
            }
        }
        if i.key_pressed(egui::Key::A) && !i.modifiers.ctrl {
            if let Some(SubEditorState::Sketch { tool, line_start, rect_start, circle_center, arc_center, arc_start, .. }) = self.sub_editor.as_mut() {
                *tool = SketchTool::Arc;
                *line_start = None;
                *rect_start = None;
                *circle_center = None;
                *arc_center = None;
                *arc_start = None;
            }
        }
        if i.key_pressed(egui::Key::E) && !i.modifiers.ctrl {
            self.perform_extrude(self.extrude_depth);
        }
    }
});
```

Also, at the end of `update`, call `self.perform_rebuild_if_due();` and request a repaint if the debounce is pending:

```rust
self.perform_rebuild_if_due();
if self.rebuild_pending_since.is_some() {
    ctx.request_repaint_after(std::time::Duration::from_millis(30));
}
```

Replace all old references to `self.sketch_session` in the menus and status bar. The feature tree panel and property panel in `update` can render placeholder text for now — the full history panel lands in Task 14.

For the feature tree: replace the block that shows "Sketch (N entities)" with:

```rust
// Feature tree (left panel) — placeholder; Task 14 replaces this with the right-side
// history panel and deletes the left panel entirely.
egui::SidePanel::left("feature_tree")
    .default_width(200.0)
    .show(ctx, |ui| {
        ui.heading("Document");
        ui.separator();
        ui.label(format!("Nodes: {}", self.document.nodes.len()));
        ui.label(format!("Components: {}", self.document.components.len()));
        ui.label(format!("Branches: {}", self.document.branches.len()));
    });
```

The existing Menu → Sketch → {Line, Rectangle, Circle, Arc} tool buttons need to read from `self.sub_editor`'s inner fields instead of `self.sketch_session`. Use the same pattern shown for the keyboard shortcuts.

Menu → Sketch → New Sketch (XY): call `self.enter_sketch_mode()`.
Menu → Sketch → Extrude: call `self.perform_extrude(self.extrude_depth)`.
Menu → Sketch → Exit Sketch: call `self.cancel_sub_editor()`.

Delete the `SketchSession` struct definition, the `world_xy`/`new` constructors, `impl SketchSession`, the `snap_point`/`build_sketch_lines` instance methods — everything that was moved to free functions or `SubEditorState::Sketch`.

- [ ] **Step 6: Delete the stale `perform_extrude` that referenced `sketch_session`**

Search for `self.sketch_session` in the file. There should be no remaining references after this task.

Run: `grep -n "sketch_session" crates/mycad-app/src/lib.rs`  (via the Grep tool)
Expected: no matches.

- [ ] **Step 7: Build and test**

Run: `cargo build --workspace`
Expected: clean build, though there will likely be compile errors from the first attempt (missing imports, struct field renames). Fix iteratively.

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: clean.

Run: `cargo test --workspace`
Expected: all tests pass.

- [ ] **Step 8: Manual smoke test**

Run: `cargo run -p mycad-app`
Expected: app launches. Press S → sketch mode entered (empty sketch node created in the document). Draw a rectangle. Press E → rebuild fires, extruded solid appears in the viewport. The existing "draw rectangle → extrude" golden path still works.

Check the status bar: it should not show `sketch_session` references; it should read from `self.sub_editor`.

- [ ] **Step 9: Commit**

```bash
git add crates/mycad-app/src/lib.rs crates/mycad-app/Cargo.toml
git commit -m "feat(app): replace SketchSession with Document+SubEditorState, route extrude through framework"
```

---

## Task 14: History side panel UI (right side: DAG + component tree)

**Files:**
- Create: `crates/mycad-ui/src/history_panel.rs`
- Modify: `crates/mycad-ui/src/lib.rs`
- Modify: `crates/mycad-app/src/lib.rs`

- [ ] **Step 1: Create the history panel module**

Create `crates/mycad-ui/src/history_panel.rs`:

```rust
//! Right-side history panel: DAG visualization, component tree, branch selector.

use egui;
use mycad_kernel::parametric::feature::Operation;
use mycad_kernel::parametric::types::{BranchId, ComponentId, Document, HistoryNode, NodeId};

#[derive(Debug, Default)]
pub struct HistoryPanelState {
    pub selected_node: Option<NodeId>,
}

#[derive(Debug, Clone)]
pub enum HistoryPanelAction {
    SelectNode(NodeId),
    DoubleClickNode(NodeId),
    SwitchBranch(BranchId),
    CreateBranch { from: NodeId, name: String },
    MergeBranches { a: BranchId, b: BranchId, name: String },
    ActivateComponent(ComponentId),
    None,
}

pub fn history_panel(
    ui: &mut egui::Ui,
    doc: &Document,
    state: &mut HistoryPanelState,
) -> HistoryPanelAction {
    let mut action = HistoryPanelAction::None;

    // --- Branch selector ---
    ui.horizontal(|ui| {
        ui.label("Branch:");
        let current = doc.branch(doc.current_branch).map(|b| b.name.clone()).unwrap_or_default();
        egui::ComboBox::from_id_source("branch_selector")
            .selected_text(current)
            .show_ui(ui, |ui| {
                for (id, branch) in &doc.branches {
                    let label = format!("{} ({})", branch.name, branch.components.len());
                    if ui.selectable_label(doc.current_branch == *id, label).clicked() {
                        action = HistoryPanelAction::SwitchBranch(*id);
                    }
                }
            });
        if ui.button("+ Branch").clicked() {
            if let Ok(comp) = doc.active_component() {
                action = HistoryPanelAction::CreateBranch {
                    from: comp.tip,
                    name: format!("branch-{}", doc.branches.len()),
                };
            }
        }
        if ui.button("Merge").clicked() {
            // Find another branch to merge with (just pick the first non-current one).
            let other = doc.branches.keys().find(|b| **b != doc.current_branch).copied();
            if let Some(other) = other {
                action = HistoryPanelAction::MergeBranches {
                    a: doc.current_branch,
                    b: other,
                    name: format!("merged-{}", doc.branches.len()),
                };
            }
        }
    });
    ui.separator();

    // --- DAG visualization (list-style for v1, lane rendering is a polish item) ---
    ui.heading("History");
    egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
        // Walk each component's chain on the current branch; render each node as a row.
        if let Ok(branch) = doc.branch(doc.current_branch) {
            for comp_id in &branch.components {
                if let Ok(comp) = doc.component(*comp_id) {
                    ui.label(egui::RichText::new(&comp.name).strong());
                    let mut nodes: Vec<&HistoryNode> = vec![];
                    let mut cur = Some(comp.tip);
                    while let Some(id) = cur {
                        if let Ok(n) = doc.node(id) {
                            nodes.push(n);
                            cur = n.parent;
                        } else {
                            break;
                        }
                    }
                    // Reverse so root is at the top.
                    nodes.reverse();
                    for node in nodes {
                        let glyph = operation_glyph(&node.operation);
                        let color = node_color(node);
                        let is_selected = Some(node.id) == state.selected_node;
                        let label = format!("{} {}", glyph, operation_label(&node.operation));
                        let row = ui.selectable_label(
                            is_selected,
                            egui::RichText::new(label).color(color),
                        );
                        if row.clicked() {
                            state.selected_node = Some(node.id);
                            action = HistoryPanelAction::SelectNode(node.id);
                        }
                        if row.double_clicked() {
                            action = HistoryPanelAction::DoubleClickNode(node.id);
                        }
                    }
                    ui.separator();
                }
            }
        }
    });

    // --- Component tree ---
    ui.heading("Components");
    if let Ok(branch) = doc.branch(doc.current_branch) {
        for comp_id in &branch.components {
            if let Ok(comp) = doc.component(*comp_id) {
                let is_active = doc.active_component == Some(*comp_id);
                let label = if is_active {
                    format!("▸ {} (active)", comp.name)
                } else {
                    format!("  {}", comp.name)
                };
                if ui.selectable_label(is_active, label).clicked() {
                    action = HistoryPanelAction::ActivateComponent(*comp_id);
                }
            }
        }
    }

    ui.separator();
    ui.heading("Properties");
    if let Some(selected) = state.selected_node {
        if let Ok(node) = doc.node(selected) {
            ui.label(format!("Node: {:?}", node.id.0));
            ui.label(format!("Op: {}", operation_label(&node.operation)));
            ui.label(format!("Dirty: {}", node.dirty));
            if let Some(err) = &node.error {
                ui.label(
                    egui::RichText::new(format!("Error: {}", err.reason))
                        .color(egui::Color32::RED),
                );
            }
        }
    } else {
        ui.label("No selection");
    }

    action
}

fn operation_glyph(op: &Operation) -> &'static str {
    match op {
        Operation::Noop => "·",
        Operation::CreateDatumPlane(_) => "▭",
        Operation::CreateSketch(_) => "◇",
        Operation::Extrude(_) => "⬛",
    }
}

fn operation_label(op: &Operation) -> &'static str {
    match op {
        Operation::Noop => "Root",
        Operation::CreateDatumPlane(_) => "DatumPlane",
        Operation::CreateSketch(_) => "Sketch",
        Operation::Extrude(_) => "Extrude",
    }
}

fn node_color(node: &HistoryNode) -> egui::Color32 {
    if node.error.is_some() {
        egui::Color32::RED
    } else if node.dirty {
        egui::Color32::from_rgb(255, 220, 100)
    } else if node.cached_output.is_some() {
        egui::Color32::from_rgb(100, 200, 100)
    } else {
        egui::Color32::GRAY
    }
}
```

- [ ] **Step 2: Export from mycad-ui**

Edit `crates/mycad-ui/src/lib.rs`. After the existing module declarations add:

```rust
pub mod history_panel;
```

Also add any re-exports as needed.

- [ ] **Step 3: Wire the panel into MyCadApp::update**

In `crates/mycad-app/src/lib.rs`:

1. Add a new field:
   ```rust
   history_state: mycad_ui::history_panel::HistoryPanelState,
   ```
   Update `MyCadApp::new` to initialize it: `history_state: Default::default(),`.

2. In `update`, replace the existing right-side property panel with:

```rust
egui::SidePanel::right("history_panel")
    .default_width(280.0)
    .show(ctx, |ui| {
        let action = mycad_ui::history_panel::history_panel(
            ui,
            &self.document,
            &mut self.history_state,
        );
        match action {
            mycad_ui::history_panel::HistoryPanelAction::None => {}
            mycad_ui::history_panel::HistoryPanelAction::SelectNode(_) => {}
            mycad_ui::history_panel::HistoryPanelAction::DoubleClickNode(node_id) => {
                // Enter sub-editor. For spec #1, only tip nodes are editable in place.
                // Editing a past node is a Task 15 follow-up (auto-branch).
                // For now we just status-report.
                self.status_message =
                    format!("Double-click on {:?} — sub-editor entry lands later", node_id);
            }
            mycad_ui::history_panel::HistoryPanelAction::SwitchBranch(bid) => {
                if let Err(e) = self.document.checkout(bid) {
                    self.status_message = format!("checkout failed: {e}");
                } else {
                    self.refresh_renderer();
                }
            }
            mycad_ui::history_panel::HistoryPanelAction::CreateBranch { from, name } => {
                match self.document.branch_from(self.document.current_branch, from, name) {
                    Ok(bid) => {
                        let _ = self.document.checkout(bid);
                        self.status_message = "Branch created".into();
                    }
                    Err(e) => {
                        self.status_message = format!("branch failed: {e}");
                    }
                }
            }
            mycad_ui::history_panel::HistoryPanelAction::MergeBranches { a, b, name } => {
                match self.document.merge_branches(a, b, name) {
                    Ok(bid) => {
                        let _ = self.document.checkout(bid);
                        self.status_message = "Branches merged".into();
                    }
                    Err(e) => {
                        self.status_message = format!("merge failed: {e}");
                    }
                }
            }
            mycad_ui::history_panel::HistoryPanelAction::ActivateComponent(cid) => {
                self.document.active_component = Some(cid);
            }
        }
    });
```

3. Delete the old `left "feature_tree"` panel. Everything is in the right panel now.

4. Delete the old `right "property_panel"` panel.

- [ ] **Step 4: Build and run**

Run: `cargo build --workspace`
Expected: clean.

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: clean.

Run: `cargo run -p mycad-app`
Expected: right-side panel shows `main` branch, DAG list with root and any nodes you create, component tree, properties. Branch selector works (at least for the single `main` branch). Clicking `+ Branch` creates a new branch in the dropdown.

- [ ] **Step 5: Commit**

```bash
git add crates/mycad-ui/src/history_panel.rs crates/mycad-ui/src/lib.rs crates/mycad-app/src/lib.rs
git commit -m "feat(ui): add history side panel with DAG list, component tree, and branch selector"
```

---

## Task 15: Keyboard shortcuts, sub-editor undo, and final cleanup

**Files:**
- Modify: `crates/mycad-app/src/lib.rs`
- Modify: `crates/mycad-kernel/src/features.rs` (remove the free function `extrude` if possible, or mark `pub(crate)`)
- Modify: `crates/mycad-ui/src/panels.rs` (delete the old `feature_tree_panel` if it exists and nothing else references it)

- [ ] **Step 1: Wire sub-editor undo/redo (Ctrl+Z / Ctrl+Y)**

In `crates/mycad-app/src/lib.rs`, inside the `ctx.input(|i| { ... })` block, add (inside the `if in_sketch_mode` branch or before it, works either way):

```rust
if i.modifiers.ctrl && i.key_pressed(egui::Key::Z) {
    if let Some(SubEditorState::Sketch { local_sketch, undo_stack, redo_stack, .. }) = self.sub_editor.as_mut() {
        if let Some(prev) = undo_stack.pop() {
            redo_stack.push(local_sketch.clone());
            *local_sketch = prev;
        }
    }
}
if i.modifiers.ctrl && i.key_pressed(egui::Key::Y) {
    if let Some(SubEditorState::Sketch { local_sketch, undo_stack, redo_stack, .. }) = self.sub_editor.as_mut() {
        if let Some(next) = redo_stack.pop() {
            undo_stack.push(local_sketch.clone());
            *local_sketch = next;
        }
    }
}
```

Before every sketch mutation in `handle_sketch_input`, push the current state onto the undo stack:

Find each call site where `local_sketch.add_entity(...)` or `local_sketch.add_rectangle(...)` is called. Before each, insert:

```rust
undo_stack.push(local_sketch.clone());
redo_stack.clear();
```

(Note: `undo_stack` and `redo_stack` need to be in the destructure pattern at the top of `handle_sketch_input`.) Update the destructure to include both.

- [ ] **Step 2: Wire Ctrl+B (branch) and Ctrl+M (merge) globally**

In the `ctx.input(|i| { ... })` block, outside the `in_sketch_mode` branch:

```rust
if i.modifiers.ctrl && i.key_pressed(egui::Key::B) {
    if let Ok(comp) = self.document.active_component() {
        let from = comp.tip;
        let name = format!("branch-{}", self.document.branches.len());
        match self.document.branch_from(self.document.current_branch, from, name) {
            Ok(bid) => {
                let _ = self.document.checkout(bid);
                self.status_message = "Branch created (Ctrl+B)".into();
            }
            Err(e) => self.status_message = format!("branch failed: {e}"),
        }
    }
}
if i.modifiers.ctrl && i.key_pressed(egui::Key::M) {
    let other = self
        .document
        .branches
        .keys()
        .find(|b| **b != self.document.current_branch)
        .copied();
    if let Some(other) = other {
        let name = format!("merged-{}", self.document.branches.len());
        match self.document.merge_branches(self.document.current_branch, other, name) {
            Ok(bid) => {
                let _ = self.document.checkout(bid);
                self.status_message = "Branches merged (Ctrl+M)".into();
            }
            Err(e) => self.status_message = format!("merge failed: {e}"),
        }
    }
}
```

- [ ] **Step 3: Make the free function `extrude` `pub(crate)`**

Edit `crates/mycad-kernel/src/features.rs`. Find:

```rust
pub fn extrude(sketch: &Sketch, params: ExtrudeParams) -> Result<ExtrudeResult, ExtrudeError> {
```

Change `pub fn` to `pub(crate) fn`. This keeps `ExtrudeOp::build` working while preventing any app-level code from calling it directly.

Run: `cargo build --workspace`
Expected: clean. If there are callers outside the kernel, fix them by routing through the framework.

- [ ] **Step 4: Verify no stale references remain**

Run these searches via the Grep tool (not shell `grep`):

```
pattern: "SketchSession"
path: C:\Users\liork\Documents\projects\mycad\crates
```
Expected: 0 matches.

```
pattern: "fn perform_extrude"
path: C:\Users\liork\Documents\projects\mycad\crates
```
Expected: exactly 1 match (the new version in `mycad-app/src/lib.rs`).

```
pattern: "sketch_session"
path: C:\Users\liork\Documents\projects\mycad\crates
```
Expected: 0 matches.

- [ ] **Step 5: Delete `panels.rs::feature_tree_panel` if it is unreferenced**

Check whether `mycad-ui::panels::feature_tree_panel` is still called from anywhere:

```
pattern: "feature_tree_panel"
```
If there are no call sites outside `panels.rs` itself, delete the function. If it still has call sites (the old `MyCadApp` may have referenced it), either delete those callers first and then the function, or leave both in place if they're harmless.

- [ ] **Step 6: Full clippy and test run**

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: clean.

Run: `cargo test --workspace`
Expected: all tests pass, including the parametric unit tests, the signature corpus, and the end-to-end integration test.

- [ ] **Step 7: Manual golden-path smoke test**

Run: `cargo run -p mycad-app`

Verify the following manually:
1. App launches with empty viewport and right-side history panel showing `main` branch, `Part 1` component, and a single root node.
2. Press `S` — sketch mode enters. A `CreateSketch` node appears in the history panel. Status bar shows "Sketch mode".
3. Press `R`, click two corners — a rectangle is drawn. History panel shows the sketch node is dirty/yellow during the debounce window, then green after ~150ms.
4. Press `E` — the rectangle is extruded, an `Extrude` node appears in the history panel, the viewport shows the solid.
5. Press `Ctrl+Z` while still in sketch mode (which is now committed; skip this if the sub-editor was closed after extrude). Verify the shortcut doesn't crash.
6. Press `Ctrl+B` — a new branch is created; the branch selector now shows two branches; the history panel re-renders.
7. Switch back to the original branch via the branch dropdown — the state swaps correctly.

Any of these failing is a blocker.

- [ ] **Step 8: Commit**

```bash
git add crates/mycad-app/src/lib.rs crates/mycad-kernel/src/features.rs crates/mycad-ui/
git commit -m "feat(app): wire sub-editor undo and Ctrl+B/Ctrl+M, gate extrude(), delete stale paths"
```

---

## Final verification

- [ ] **Step 1: Full workspace clippy**

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 2: Full workspace tests**

Run: `cargo test --workspace`
Expected: all pass. Expected test counts (approximate):
- `parametric::types` — 6 tests
- `parametric::feature` — 4 tests
- `parametric::naming` — 10 tests
- `parametric::document` — 9 tests
- `parametric::rebuild` — 4 tests
- `parametric::ops::datum_plane` — 2 tests
- `parametric::ops::sketch_op` — 1 test
- `tests/parametric_end_to_end` — 1 test
- `tests/signature_corpus` — 5 tests
- Plus all existing tests in `sketch`, `brep`, `features`, `tessellation`, etc.

If any existing test breaks, investigate — the migration should not regress any existing behavior. If an existing test was specific to the deleted `SketchSession` path, update or delete it and document in the commit message.

- [ ] **Step 3: Format**

Run: `cargo fmt --all`
Expected: no changes, or minor formatting that should be committed.

- [ ] **Step 4: Final commit if fmt changed anything**

```bash
git add -A
git commit -m "style: cargo fmt across the parametric migration"
```

- [ ] **Step 5: Review the commit history**

Run: `git log --oneline -20`
Expected: a clean sequence of feature commits mirroring the task list, easy to review.

---

## Notes for the implementer

- **Deviate on field names and file paths only if the existing code disagrees with this plan.** For example, if `Sketch::add_rectangle` has a different signature than what the tests assume, fix the test, not the production code — unless the plan explicitly introduces a new method.
- **If `Sketch` or `BRepModel` do not already implement `Serialize + Deserialize`**, derive them where the compiler asks. Most of the existing types already do; some may need `#[derive(Serialize, Deserialize)]` added. If a field cannot be serialized (e.g., a function pointer), that's a real design problem — stop and escalate.
- **The CLAUDE.md convention** is to keep kernel tests inline at the bottom of each module. Follow that convention. Integration tests go under `crates/mycad-kernel/tests/`.
- **Every commit must leave the workspace green.** If a task's steps leave the workspace red, do not commit — split the task further.
- **The E key "extrude from sketch mode" flow is preserved** end-to-end. That's the regression smoke test that has to still work.
- **Things deliberately left as stubs for later specs (#2 and #3):**
  - Auto-branch on editing a past node (spec #3 sketch flow may trigger this first).
  - DAG lane rendering (the history panel uses a flat list for v1).
  - Signature computation for B-Rep entities produced by `ExtrudeOp` (the field exists but it's populated with empty signatures because spec #1 has no downstream consumers of them — spec #2 adds the first real consumer when sketching on solid faces).
  - File save/load — out of scope per Q12.
