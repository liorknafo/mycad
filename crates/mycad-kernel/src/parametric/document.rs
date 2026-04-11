//! Behavior for [`crate::parametric::types::Document`]: construction, lookup,
//! append_op, new_component, branching, merging, checkout, deletion.

use crate::parametric::errors::{ParametricError, Result};
use crate::parametric::feature::{Feature, FeatureOutput, Operation};
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
        self.nodes
            .get(&id)
            .ok_or(ParametricError::NodeNotFound(id.0))
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

        let inputs = op.inputs();

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

        let current_branch_id = self.current_branch;
        let branch = self
            .branches
            .get_mut(&current_branch_id)
            .ok_or(ParametricError::BranchNotFound(current_branch_id.0))?;
        branch.components.push(id);

        if let Some(root) = self.nodes.get_mut(&self.root_node) {
            root.component_tags.insert(id);
        }

        Ok(id)
    }

    // --- Branching ----------------------------------------------------------

    /// Create a new branch by cloning components from a source branch at `split_at`.
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

            // Does source_comp's chain pass through split_at?
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

            // Walk from the cloned tip back to root, tagging each node.
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

    /// Merge two branches into a new branch whose component set is the union.
    /// Component sets are always disjoint by construction.
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
        // GC intentionally not run here (spec Section 4).
        Ok(())
    }
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
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
        assert!(matches!(
            doc.node(bogus),
            Err(ParametricError::NodeNotFound(_))
        ));
    }

    #[test]
    fn branch_from_clones_components_with_new_ids() {
        let mut doc = Document::new();
        let source_branch = doc.current_branch;
        let _n1 = doc.append_op(Operation::Noop).unwrap();
        let split_at = doc.active_component().unwrap().tip;

        let new_branch = doc
            .branch_from(source_branch, split_at, "experiment".into())
            .unwrap();

        let source_comp_id = doc.branches[&source_branch].components[0];
        let new_branch_struct = doc.branch(new_branch).unwrap();
        assert_eq!(new_branch_struct.name, "experiment");
        assert_eq!(new_branch_struct.components.len(), 1);
        let new_comp_id = new_branch_struct.components[0];
        assert_ne!(new_comp_id, source_comp_id);

        assert_eq!(doc.component(new_comp_id).unwrap().tip, split_at);

        let node = doc.node(split_at).unwrap();
        assert!(node.component_tags.contains(&source_comp_id));
        assert!(node.component_tags.contains(&new_comp_id));

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
        let new_branch = doc
            .branch_from(source_branch, split_at, "exp".into())
            .unwrap();

        doc.checkout(new_branch).unwrap();
        assert_eq!(doc.current_branch, new_branch);
        let expected_active = doc.branches[&new_branch].components[0];
        assert_eq!(doc.active_component, Some(expected_active));
    }

    #[test]
    fn merge_branches_unions_component_sets() {
        let mut doc = Document::new();
        let main_id = doc.current_branch;
        let split_at = doc.root_node;
        let other = doc.branch_from(main_id, split_at, "alt".into()).unwrap();
        doc.checkout(other).unwrap();
        doc.new_component("Part 2".into(), None).unwrap();
        doc.checkout(main_id).unwrap();

        let merged = doc
            .merge_branches(main_id, other, "combined".into())
            .unwrap();
        let mb = doc.branch(merged).unwrap();
        let main_len = doc.branch(main_id).unwrap().components.len();
        let other_len = doc.branch(other).unwrap().components.len();
        assert_eq!(mb.components.len(), main_len + other_len);
    }

    #[test]
    fn delete_branch_removes_from_branches_table() {
        let mut doc = Document::new();
        let main_id = doc.current_branch;
        let extra = doc
            .branch_from(main_id, doc.root_node, "extra".into())
            .unwrap();
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
}
