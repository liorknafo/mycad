//! Core types: IDs, HistoryNode, Component, Branch, Document.

use crate::parametric::feature::{FeatureOutput, InputRef, Operation};
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

/// The error state of a node after a failed rebuild.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeError {
    pub reason: String,
}

/// A single node in the history DAG. Represents one feature operation.
///
/// Mutable in its `operation` (parameter edits mutate the node in place and
/// mark it dirty). Immutable in its identity (`id`) and parent link (`parent`).
/// The cumulative cached output is stored in `cached_output`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryNode {
    pub id: NodeId,
    pub operation: Operation,
    pub parent: Option<NodeId>,
    pub inputs: Vec<InputRef>,
    pub component_tags: HashSet<ComponentId>,
    pub cached_output: Option<FeatureOutput>,
    pub dirty: bool,
    pub signature_version: u32,
    pub error: Option<NodeError>,
}

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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    pub nodes: HashMap<NodeId, HistoryNode>,
    pub components: HashMap<ComponentId, Component>,
    pub branches: HashMap<BranchId, Branch>,
    pub root_node: NodeId,
    pub current_branch: BranchId,
    pub active_component: Option<ComponentId>,
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

    #[test]
    fn history_node_fields_round_trip() {
        let id = NodeId::new();
        let component_id = ComponentId::new();
        let mut tags = HashSet::new();
        tags.insert(component_id);
        let node = HistoryNode {
            id,
            operation: Operation::Noop,
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
}
