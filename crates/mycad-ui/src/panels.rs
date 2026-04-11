//! UI panels: feature tree, property panel, status bar.

use egui;
use mycad_kernel::sketch::{
    ConstraintState, Sketch, SketchConstraintKind,
    SketchSolveResult,
};
use mycad_kernel::parametric::types::Document;
use mycad_kernel::parametric::feature::Operation;

/// Show the feature tree panel with entities and constraints
pub fn feature_tree_panel(ui: &mut egui::Ui, sketch: &Sketch, solve_result: &SketchSolveResult) {
    ui.heading("Features");
    ui.separator();

    // Show DOF count from solver
    if let Some(dof) = solve_result.dof {
        let status_color = match solve_result.status {
            mycad_kernel::sketch::SketchSolveStatus::Converged => egui::Color32::from_rgb(100, 255, 100),
            _ => egui::Color32::from_rgb(255, 200, 100),
        };
        ui.horizontal(|ui| {
            ui.label("DOF:");
            ui.label(egui::RichText::new(format!("{}", dof)).color(status_color));
        });
    }

    ui.separator();

    // List entities
    ui.collapsing("Entities", |ui| {
        for entity in sketch.iter() {
            let label = format!("{}: {}", entity_type_name(&entity.geometry), entity.id.0);
            ui.label(label);
        }
    });

    // List constraints with visual state
    ui.collapsing("Constraints", |ui| {
        for constraint in sketch.constraints() {
            let visual_state = if !constraint.enabled {
                ConstraintState::Normal
            } else if solve_result.error.is_some() {
                ConstraintState::Conflicting
            } else if solve_result.status == mycad_kernel::sketch::SketchSolveStatus::Converged {
                ConstraintState::Satisfied
            } else {
                ConstraintState::Normal
            };

            let color = match visual_state {
                ConstraintState::Satisfied => egui::Color32::from_rgb(100, 255, 100),
                ConstraintState::Conflicting => egui::Color32::from_rgb(255, 100, 100),
                ConstraintState::OverConstrained => egui::Color32::from_rgb(255, 180, 100),
                ConstraintState::Normal => egui::Color32::from_rgb(200, 200, 200),
            };

            let name = constraint
                .name
                .as_deref()
                .unwrap_or_else(|| constraint_type_name(&constraint.kind).leak());
            let enabled_marker = if constraint.enabled { "" } else { " [off]" };
            ui.horizontal(|ui| {
                ui.colored_label(color, "●");
                ui.label(format!("{}{}", name, enabled_marker));
            });
        }
    });
}

/// Show the property panel for selected items
pub fn property_panel(
    ui: &mut egui::Ui,
    sketch: &mut Sketch,
    selected_entity: Option<mycad_kernel::sketch::SketchEntityId>,
) {
    ui.heading("Properties");
    ui.separator();

    if let Some(entity_id) = selected_entity {
        if let Some(entity) = sketch.entity(entity_id) {
            ui.label(format!("Type: {}", entity_type_name(&entity.geometry)));
            ui.label(format!("ID: {}", entity_id.0));
            
            if let Some(name) = &entity.name {
                ui.label(format!("Name: {}", name));
            }

            ui.separator();
            
            // Show geometry-specific properties
            match &entity.geometry {
                mycad_kernel::sketch::SketchGeometry::LineSegment(line) => {
                    ui.label(format!("Start: ({:.2}, {:.2})", line.start.x, line.start.y));
                    ui.label(format!("End: ({:.2}, {:.2})", line.end.x, line.end.y));
                    ui.label(format!("Length: {:.2}", line.length()));
                }
                mycad_kernel::sketch::SketchGeometry::Point(pt) => {
                    ui.label(format!("Position: ({:.2}, {:.2})", pt.position.x, pt.position.y));
                }
                mycad_kernel::sketch::SketchGeometry::Circle(circle) => {
                    ui.label(format!("Center: ({:.2}, {:.2})", circle.center.x, circle.center.y));
                    ui.label(format!("Radius: {:.2}", circle.radius));
                }
                mycad_kernel::sketch::SketchGeometry::Arc(arc) => {
                    ui.label(format!("Center: ({:.2}, {:.2})", arc.center.x, arc.center.y));
                    ui.label(format!("Radius: {:.2}", arc.radius));
                }
            }
        } else {
            ui.label("No selection");
        }
    } else {
        ui.label("No selection");
    }
}

fn entity_type_name(geometry: &mycad_kernel::sketch::SketchGeometry) -> &'static str {
    match geometry {
        mycad_kernel::sketch::SketchGeometry::Point(_) => "Point",
        mycad_kernel::sketch::SketchGeometry::LineSegment(_) => "Line",
        mycad_kernel::sketch::SketchGeometry::Circle(_) => "Circle",
        mycad_kernel::sketch::SketchGeometry::Arc(_) => "Arc",
    }
}

fn constraint_type_name(kind: &SketchConstraintKind) -> String {
    match kind {
        SketchConstraintKind::Coincident { .. } => "Coincident".to_string(),
        SketchConstraintKind::Horizontal { .. } => "Horizontal".to_string(),
        SketchConstraintKind::Vertical { .. } => "Vertical".to_string(),
        SketchConstraintKind::Distance { distance, .. } => format!("Distance ({:.2})", distance),
    }
}

/// Show solver status information
pub fn solver_status_panel(ui: &mut egui::Ui, result: &SketchSolveResult) {
    let (status_text, color) = match result.status {
        mycad_kernel::sketch::SketchSolveStatus::Converged => {
            ("✓ Converged", egui::Color32::from_rgb(100, 255, 100))
        }
        mycad_kernel::sketch::SketchSolveStatus::MaxIterationsReached => {
            ("⚠ Max iterations", egui::Color32::from_rgb(255, 200, 100))
        }
        mycad_kernel::sketch::SketchSolveStatus::Failed => {
            ("✗ Failed", egui::Color32::from_rgb(255, 100, 100))
        }
    };

    ui.horizontal(|ui| {
        ui.label("Solver:");
        ui.colored_label(color, status_text);
        if result.iterations > 0 {
            ui.label(format!("({} iters)", result.iterations));
        }
    });

    if let Some(ref error) = result.error {
        ui.label(egui::RichText::new(format!("Error: {:?}", error)).color(egui::Color32::RED));
    }
}

/// Show the parametric feature tree from a document.
pub fn parametric_feature_tree_panel(ui: &mut egui::Ui, document: &Document) {
    ui.heading("History");
    ui.separator();

    let mut features = Vec::new();
    for (node_id, node) in &document.nodes {
        if node.parent != Some(document.root_node) {
            continue; // Skip non-root children for now (single-component view)
        }

        let (op_type, name, status_color) = match &node.operation {
            Operation::Noop => ("Root".to_string(), "Root".to_string(), egui::Color32::GRAY),
            Operation::CreateDatumPlane(op) => {
                (
                    "Datum".to_string(),
                    op.name.clone(),
                    if node.error.is_some() {
                        egui::Color32::from_rgb(255, 100, 100)
                    } else if node.dirty {
                        egui::Color32::from_rgb(255, 200, 100)
                    } else {
                        egui::Color32::from_rgb(100, 255, 100)
                    },
                )
            }
            Operation::CreateSketch(op) => {
                let entity_count = node
                    .cached_output
                    .as_ref()
                    .and_then(|out| out.sketches.first())
                    .map(|sketch_entry| sketch_entry.sketch.entities.len())
                    .unwrap_or(0);

                let name = format!("{} ({} entities)", op.name, entity_count);
                (
                    "Sketch".to_string(),
                    name,
                    if node.error.is_some() {
                        egui::Color32::from_rgb(255, 100, 100)
                    } else if node.dirty {
                        egui::Color32::from_rgb(255, 200, 100)
                    } else {
                        egui::Color32::from_rgb(100, 255, 100)
                    },
                )
            }
            Operation::Extrude(op) => {
                let depth = op.depth;
                (
                    "Extrude".to_string(),
                    format!("Extrude (depth: {:.2})", depth),
                    if node.error.is_some() {
                        egui::Color32::from_rgb(255, 100, 100)
                    } else if node.dirty {
                        egui::Color32::from_rgb(255, 200, 100)
                    } else {
                        egui::Color32::from_rgb(100, 255, 100)
                    },
                )
            }
        };

        features.push((node_id, op_type, name, status_color, node.error.clone()));
    }

    if features.is_empty() {
        ui.label("(empty)");
    }

    for (_, op_type, name, status_color, error) in features {
        ui.horizontal(|ui| {
            ui.colored_label(status_color, "●");
            ui.label(format!("{}: {}", op_type, name));
            if error.is_some() {
                ui.label(egui::RichText::new("⚠").color(egui::Color32::RED));
            }
        });
    }
}
