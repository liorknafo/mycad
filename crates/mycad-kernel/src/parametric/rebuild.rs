//! Rebuild engine: dirty propagation, topological rebuild, hard-fail.
//!
//! Edits mark nodes dirty; [`mark_dirty`] closes dirtyness forward along
//! parent+inputs edges; [`rebuild`] runs `Feature::build` on dirty nodes in
//! topological order, storing outputs in each node's `cached_output`. If any
//! node's resolve or build fails, [`rebuild`] aborts with the error set on the
//! failing node (hard-fail) — nodes downstream keep their last-known-good
//! output but are left dirty (stale).

use crate::parametric::errors::{ParametricError, Result};
use crate::parametric::feature::{BuildContext, Feature, FeatureOutput};
use crate::parametric::naming::DefaultResolver;
use crate::parametric::types::{Document, NodeError, NodeId};
use std::collections::{HashMap, HashSet, VecDeque};

/// Mark a node as needing rebuild. Also propagates dirtyness forward through
/// every descendant (via `parent` chains and `inputs` edges).
pub fn mark_dirty(doc: &mut Document, start: NodeId) -> Result<()> {
    if !doc.nodes.contains_key(&start) {
        return Err(ParametricError::NodeNotFound(start.0));
    }

    // Build reverse adjacency: for every node, which nodes read from it as
    // parent or input?
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
    let mut in_degree: HashMap<NodeId, usize> = HashMap::new();
    for id in nodes {
        in_degree.insert(*id, 0);
    }
    let mut edges: Vec<(NodeId, NodeId)> = Vec::new();

    for id in nodes {
        let node = doc
            .nodes
            .get(id)
            .ok_or(ParametricError::NodeNotFound(id.0))?;
        if let Some(p) = node.parent {
            if nodes.contains(&p) {
                edges.push((p, *id));
                *in_degree.entry(*id).or_insert(0) += 1;
            }
        }
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
/// and abort (hard-fail).
pub fn rebuild(doc: &mut Document) -> Result<()> {
    let dirty = dirty_set(doc);
    if dirty.is_empty() {
        return Ok(());
    }
    let order = topological_order(doc, &dirty)?;

    let resolver = DefaultResolver;

    for node_id in order {
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
                    .ok_or_else(|| {
                        ParametricError::Other(format!("node {:?} has no component tag", node_id))
                    })?,
            )
        };

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

        let node = doc.node_mut(node_id)?;
        match build_result {
            Ok(out) => {
                node.cached_output = Some(out);
                node.dirty = false;
                node.error = None;
            }
            Err(e) => {
                let reason = e.to_string();
                node.error = Some(NodeError {
                    reason: reason.clone(),
                });
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
