//! Right-side history panel: DAG list, component tree, branch selector, properties.

use mycad_kernel::parametric::feature::Operation;
use mycad_kernel::parametric::types::{BranchId, ComponentId, Document, HistoryNode, NodeId};

#[derive(Debug, Default)]
pub struct HistoryPanelState {
    pub selected_node: Option<NodeId>,
}

#[derive(Debug, Clone)]
pub enum HistoryPanelAction {
    None,
    SelectNode(NodeId),
    DoubleClickNode(NodeId),
    SwitchBranch(BranchId),
    CreateBranch { from: NodeId, name: String },
    MergeBranches { a: BranchId, b: BranchId, name: String },
    ActivateComponent(ComponentId),
}

pub fn history_panel(
    ui: &mut egui::Ui,
    doc: &Document,
    state: &mut HistoryPanelState,
) -> HistoryPanelAction {
    let mut action = HistoryPanelAction::None;

    ui.horizontal(|ui| {
        ui.label("Branch:");
        let current = doc
            .branch(doc.current_branch)
            .map(|b| b.name.clone())
            .unwrap_or_default();
        egui::ComboBox::from_id_salt("branch_selector")
            .selected_text(current)
            .show_ui(ui, |ui| {
                for (id, branch) in &doc.branches {
                    let label = format!("{} ({})", branch.name, branch.components.len());
                    if ui
                        .selectable_label(doc.current_branch == *id, label)
                        .clicked()
                    {
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
            let other = doc
                .branches
                .keys()
                .find(|b| **b != doc.current_branch)
                .copied();
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

    ui.heading("History");
    egui::ScrollArea::vertical()
        .max_height(220.0)
        .id_salt("history_scroll")
        .show(ui, |ui| {
            if let Ok(branch) = doc.branch(doc.current_branch) {
                for comp_id in &branch.components {
                    if let Ok(comp) = doc.component(*comp_id) {
                        ui.label(egui::RichText::new(&comp.name).strong());
                        let mut nodes: Vec<&HistoryNode> = Vec::new();
                        let mut cur = Some(comp.tip);
                        while let Some(id) = cur {
                            if let Ok(n) = doc.node(id) {
                                nodes.push(n);
                                cur = n.parent;
                            } else {
                                break;
                            }
                        }
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
            ui.label(format!("Op: {}", operation_label(&node.operation)));
            ui.label(format!("Dirty: {}", node.dirty));
            if let Some(err) = &node.error {
                ui.label(
                    egui::RichText::new(format!("Error: {}", err.reason))
                        .color(egui::Color32::RED),
                );
            }
            if let Operation::Extrude(op) = &node.operation {
                ui.label(format!("Depth: {:.2}", op.depth));
            }
            if let Operation::CreateSketch(op) = &node.operation {
                ui.label(format!("Sketch name: {}", op.name));
                ui.label(format!("Entities: {}", op.sketch.entities.len()));
                ui.label(format!("Constraints: {}", op.sketch.constraints.len()));
            }
            if let Operation::CreateDatumPlane(op) = &node.operation {
                ui.label(format!("Plane name: {}", op.name));
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
        Operation::Extrude(_) => "■",
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
