use eframe::egui;
use mycad_kernel::math::Scalar;
use mycad_kernel::parametric::feature::Operation;
use mycad_kernel::parametric::ops::extrude_op::{ExtrudeDirection, ExtrudeOp, ProfileRef};
use mycad_kernel::parametric::rebuild::{mark_dirty, rebuild};
use mycad_kernel::sketch::SketchGeometry;
use mycad_renderer::{ProjectionMode, ViewportResponse};

use crate::sub_editor::entity_type_name;
use crate::{MyCadApp, SubEditorState, DOC_UNDO_LIMIT};

impl MyCadApp {
    pub(crate) fn is_param_sketch_mode(&self) -> bool {
        matches!(&self.sub_editor, Some(SubEditorState::Sketch { .. }))
    }

    pub(crate) fn projection_label(&self) -> &'static str {
        match self.viewport.as_ref().map(|v| v.camera().projection) {
            Some(ProjectionMode::Orthographic) => "ortho",
            _ => "perspective",
        }
    }

    /// Push the current document onto the undo stack and clear redo.
    /// Call before any mutating document operation.
    pub(crate) fn snapshot_doc(&mut self) {
        self.doc_undo.push(self.document.clone());
        if self.doc_undo.len() > DOC_UNDO_LIMIT {
            self.doc_undo.remove(0);
        }
        self.doc_redo.clear();
    }

    pub(crate) fn undo_doc(&mut self) {
        let Some(prev) = self.doc_undo.pop() else {
            self.status_message = "Nothing to undo".to_string();
            return;
        };
        self.doc_redo.push(std::mem::replace(&mut self.document, prev));
        self.sub_editor = None;
        self.rebuild_pending_since = None;
        self.rebuild_after_restore();
        self.status_message = "Undo".to_string();
    }

    pub(crate) fn redo_doc(&mut self) {
        let Some(next) = self.doc_redo.pop() else {
            self.status_message = "Nothing to redo".to_string();
            return;
        };
        self.doc_undo.push(std::mem::replace(&mut self.document, next));
        self.sub_editor = None;
        self.rebuild_pending_since = None;
        self.rebuild_after_restore();
        self.status_message = "Redo".to_string();
    }

    /// After a document restore, mark all nodes dirty and rebuild so the
    /// viewport mesh reflects the restored state.
    pub(crate) fn rebuild_after_restore(&mut self) {
        let node_ids: Vec<_> = self.document.nodes.keys().copied().collect();
        for id in node_ids {
            let _ = mark_dirty(&mut self.document, id);
        }
        if let Err(e) = rebuild(&mut self.document) {
            self.status_message = format!("Rebuild after restore failed: {:?}", e);
            return;
        }
        if let Some(last_node_id) = self.document.nodes.keys().last().copied() {
            if let Ok(node) = self.document.node(last_node_id) {
                if let Some(output) = &node.cached_output {
                    if let Some(viewport) = &mut self.viewport {
                        if let Some(mesh) = &output.mesh {
                            viewport.set_mesh(Some(mesh.clone()));
                        } else {
                            viewport.clear_mesh();
                        }
                    }
                }
            }
        }
    }

    pub(crate) fn latest_mesh(&self) -> Option<mycad_kernel::tessellation::Mesh> {
        let last_node_id = self.document.nodes.keys().last().copied()?;
        let node = self.document.node(last_node_id).ok()?;
        let out = node.cached_output.as_ref()?;
        out.mesh.clone()
    }

    /// Create an extrude operation on the most recent parametric sketch.
    pub(crate) fn perform_param_extrude(&mut self, depth: Scalar) {
        // Find the most recent sketch node (iterate in reverse insertion order)
        let mut node_ids: Vec<_> = self.document.nodes.keys().copied().collect();
        node_ids.reverse();

        let sketch_node = node_ids.iter()
            .find(|id| {
                if let Ok(node) = self.document.node(**id) {
                    matches!(node.operation, Operation::CreateSketch(_))
                } else {
                    false
                }
            })
            .copied();

        let Some(sketch_node_id) = sketch_node else {
            self.status_message = "No parametric sketch found to extrude".to_string();
            return;
        };

        self.snapshot_doc();

        // Get the sketch instance ID from the operation
        let sketch_instance_id = if let Ok(node) = self.document.node(sketch_node_id) {
            if let Operation::CreateSketch(op) = &node.operation {
                op.id
            } else {
                self.status_message = "Expected CreateSketch operation".to_string();
                return;
            }
        } else {
            self.status_message = "Failed to find sketch node".to_string();
            return;
        };

        // Create and append the extrude operation
        let extrude_op = ExtrudeOp {
            profile: ProfileRef {
                producing_node: sketch_node_id,
                sketch_id: sketch_instance_id,
            },
            depth,
            direction: ExtrudeDirection::Up,
        };

        if let Err(e) = self.document.append_op(Operation::Extrude(extrude_op)) {
            self.status_message = format!("Failed to create extrude: {:?}", e);
            return;
        }

        // Mark the extrude node as needing build
        let last_node = self.document.nodes.keys().last().copied();
        if let Some(node_id) = last_node {
            let _ = mark_dirty(&mut self.document, node_id);
        }

        self.request_param_rebuild_soon();
        self.status_message = format!("Parametric extrude created (depth: {:.2}), rebuilding...", depth);
    }

    /// Ctrl+B: branch from the current active component's tip.
    pub(crate) fn do_branch_from_active(&mut self) {
        let Ok(comp) = self.document.active_component() else {
            self.status_message = "No active component".to_string();
            return;
        };
        let from = comp.tip;
        let name = format!("branch-{}", self.document.branches.len());
        match self
            .document
            .branch_from(self.document.current_branch, from, name)
        {
            Ok(bid) => {
                let _ = self.document.checkout(bid);
                self.status_message = "Branch created (Ctrl+B)".to_string();
            }
            Err(e) => self.status_message = format!("branch failed: {:?}", e),
        }
    }

    /// Ctrl+M: merge the current branch with the first non-current branch found.
    pub(crate) fn do_merge_other_branch(&mut self) {
        let other = self
            .document
            .branches
            .keys()
            .find(|b| **b != self.document.current_branch)
            .copied();
        let Some(other) = other else {
            self.status_message = "No other branch to merge".to_string();
            return;
        };
        let name = format!("merged-{}", self.document.branches.len());
        match self
            .document
            .merge_branches(self.document.current_branch, other, name)
        {
            Ok(bid) => {
                let _ = self.document.checkout(bid);
                self.status_message = "Branches merged (Ctrl+M)".to_string();
            }
            Err(e) => self.status_message = format!("merge failed: {:?}", e),
        }
    }

    /// Request a deferred rebuild with debouncing.
    ///
    /// Sets rebuild_pending_since to now(). The rebuild is executed only when
    /// perform_param_rebuild_if_due() detects that >= REBUILD_DEBOUNCE_MS have elapsed.
    /// This prevents thrashing with frequent edits and allows batching multiple changes
    /// into a single rebuild cycle.
    #[allow(dead_code)]
    pub(crate) fn request_param_rebuild_soon(&mut self) {
        self.rebuild_pending_since = Some(std::time::Instant::now());
    }

    /// Perform deferred rebuild if the debounce timer has expired.
    ///
    /// Executes rebuild() on the parametric document only if >= REBUILD_DEBOUNCE_MS
    /// (100ms) have passed since the most recent call to request_param_rebuild_soon().
    ///
    /// On rebuild success:
    /// - Updates the viewport with the mesh from the final feature's output
    /// - Clears rebuild_pending_since
    /// - Sets status_message to "Parametric rebuild complete"
    ///
    /// On rebuild failure:
    /// - Sets status_message to describe the error (error nodes remain in the document)
    /// - Does NOT clear rebuild_pending_since (caller may retry)
    ///
    /// This method should be called once per update() cycle to integrate parametric
    /// rebuilds into the app's UI update loop.
    #[allow(dead_code)]
    pub(crate) fn perform_param_rebuild_if_due(&mut self) {
        const REBUILD_DEBOUNCE_MS: u128 = 100;

        let rebuild_now = if let Some(since) = self.rebuild_pending_since {
            since.elapsed().as_millis() >= REBUILD_DEBOUNCE_MS
        } else {
            false
        };

        if !rebuild_now {
            return;
        }

        self.rebuild_pending_since = None;

        if let Err(e) = rebuild(&mut self.document) {
            self.status_message = format!("Rebuild failed: {:?}", e);
            return;
        }

        // After successful rebuild, update viewport with the final mesh.
        // For now, just find the last feature's output.
        if let Some(last_node_id) = self.document.nodes.keys().last().copied() {
            if let Ok(node) = self.document.node(last_node_id) {
                if let Some(output) = &node.cached_output {
                    if let Some(mesh) = &output.mesh {
                        if let Some(viewport) = &mut self.viewport {
                            viewport.set_mesh(Some(mesh.clone()));
                        }
                    }
                }
            }
        }

        self.status_message = "Parametric rebuild complete".to_string();
    }

    /// Handle a click in the 3D viewport when no sketch is active.
    /// Ray-casts the click against the current mesh and reports the hit.
    pub(crate) fn handle_viewport_pick(&mut self, response: &ViewportResponse) {
        if !response.clicked {
            return;
        }
        let Some(pos) = response.hover_pos else { return };
        let Some(viewport) = self.viewport.as_ref() else { return };
        let rect = viewport.last_rect();
        if !rect.contains(pos) {
            return;
        }

        match viewport.pick_mesh_triangle(pos, rect) {
            Some(hit) => {
                self.status_message = format!(
                    "Picked triangle #{} at ({:.2}, {:.2}, {:.2})",
                    hit.triangle_index, hit.point.x, hit.point.y, hit.point.z
                );
            }
            None => {
                self.status_message = "No geometry under cursor".to_string();
            }
        }
    }

    pub(crate) fn handle_history_panel_action(
        &mut self,
        action: mycad_ui::history_panel::HistoryPanelAction,
    ) {
        use mycad_ui::history_panel::HistoryPanelAction as A;
        match action {
            A::None | A::SelectNode(_) => {}
            A::DoubleClickNode(node_id) => {
                self.status_message =
                    format!("Double-click on {:?} — sub-editor entry lands later", node_id);
            }
            A::SwitchBranch(bid) => match self.document.checkout(bid) {
                Ok(()) => {
                    self.status_message = "Switched branch".to_string();
                    self.request_param_rebuild_soon();
                }
                Err(e) => self.status_message = format!("checkout failed: {:?}", e),
            },
            A::CreateBranch { from, name } => {
                match self
                    .document
                    .branch_from(self.document.current_branch, from, name)
                {
                    Ok(bid) => {
                        let _ = self.document.checkout(bid);
                        self.status_message = "Branch created".to_string();
                    }
                    Err(e) => self.status_message = format!("branch failed: {:?}", e),
                }
            }
            A::MergeBranches { a, b, name } => {
                match self.document.merge_branches(a, b, name) {
                    Ok(bid) => {
                        let _ = self.document.checkout(bid);
                        self.status_message = "Branches merged".to_string();
                    }
                    Err(e) => self.status_message = format!("merge failed: {:?}", e),
                }
            }
            A::ActivateComponent(cid) => {
                self.document.active_component = Some(cid);
            }
        }
    }

    pub(crate) fn draw_sub_editor_properties(ui: &mut egui::Ui, sub_editor: &Option<SubEditorState>) {
        let Some(SubEditorState::Sketch {
            local_sketch,
            selected_entity,
            solver_result,
            ..
        }) = sub_editor
        else {
            ui.label("Not in sketch mode");
            return;
        };

        ui.label(egui::RichText::new("Sketch Statistics").strong());
        ui.label(format!("Entities: {}", local_sketch.entities.len()));
        ui.label(format!(
            "Constraints: {}",
            local_sketch.constraints().count()
        ));

        if let Some(result) = solver_result {
            let (status_text, status_color) = match result.status {
                mycad_kernel::sketch::SketchSolveStatus::Converged => {
                    ("Converged", egui::Color32::from_rgb(100, 255, 100))
                }
                mycad_kernel::sketch::SketchSolveStatus::MaxIterationsReached => {
                    ("Max iters", egui::Color32::from_rgb(255, 200, 100))
                }
                mycad_kernel::sketch::SketchSolveStatus::Failed => {
                    ("Failed", egui::Color32::from_rgb(255, 100, 100))
                }
            };
            ui.horizontal(|ui| {
                ui.label("Solver:");
                ui.colored_label(status_color, status_text);
            });
            if let Some(dof) = result.dof {
                let dof_color = if dof == 0 {
                    egui::Color32::from_rgb(100, 255, 100)
                } else {
                    egui::Color32::from_rgb(255, 200, 100)
                };
                ui.horizontal(|ui| {
                    ui.label("DOF:");
                    ui.colored_label(dof_color, format!("{}", dof));
                });
            }
        }

        ui.separator();

        let Some(entity_id) = selected_entity else {
            ui.label("(Click entity to select)");
            return;
        };
        let Some(entity) = local_sketch.entity(*entity_id) else {
            ui.label("Selected entity not found");
            return;
        };

        ui.heading("Selected Entity");
        ui.label(format!("Type: {}", entity_type_name(&entity.geometry)));
        ui.label(format!("ID: {}", entity_id.0));
        if let Some(name) = &entity.name {
            ui.label(format!("Name: {}", name));
        }

        let construction_text = if entity.construction {
            "Construction (G to toggle)"
        } else {
            "Normal (G to toggle)"
        };
        let construction_color = if entity.construction {
            egui::Color32::from_rgb(180, 150, 100)
        } else {
            egui::Color32::from_rgb(100, 200, 100)
        };
        ui.label(egui::RichText::new(construction_text).color(construction_color));

        ui.separator();

        match &entity.geometry {
            SketchGeometry::LineSegment(line) => {
                ui.label(format!(
                    "Start: ({:.2}, {:.2})",
                    line.start.x, line.start.y
                ));
                ui.label(format!("End: ({:.2}, {:.2})", line.end.x, line.end.y));
                ui.label(format!("Length: {:.2}", line.length()));
            }
            SketchGeometry::Point(pt) => {
                ui.label(format!(
                    "Position: ({:.2}, {:.2})",
                    pt.position.x, pt.position.y
                ));
            }
            SketchGeometry::Circle(circle) => {
                ui.label(format!(
                    "Center: ({:.2}, {:.2})",
                    circle.center.x, circle.center.y
                ));
                ui.label(format!("Radius: {:.2}", circle.radius));
            }
            SketchGeometry::Arc(arc) => {
                ui.label(format!(
                    "Center: ({:.2}, {:.2})",
                    arc.center.x, arc.center.y
                ));
                ui.label(format!("Radius: {:.2}", arc.radius));
                ui.label(format!(
                    "Start angle: {:.2}°",
                    arc.start_angle.to_degrees()
                ));
                ui.label(format!("End angle: {:.2}°", arc.end_angle.to_degrees()));
            }
        }
    }
}
