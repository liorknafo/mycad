use eframe::egui;
use mycad_kernel::parametric::feature::WorldRef;
use mycad_kernel::parametric::types::Document as ParamDocument;
use mycad_kernel::sketch::SketchGeometry;
use mycad_renderer::StandardView;

use crate::{MyCadApp, SketchTool, SubEditorState};

impl MyCadApp {
    pub(crate) fn render(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let in_param_sketch = self.is_param_sketch_mode();

        // Deferred rebuild check
        self.perform_param_rebuild_if_due();

        // Global keybindings
        ctx.input(|i| {
            // Sketch mode: S enters parametric sketch on XY plane
            if i.key_pressed(egui::Key::S) && !i.modifiers.ctrl && !i.modifiers.shift && !in_param_sketch {
                self.start_param_sketch();
            }
            // Parametric sketch mode: Shift+P (alias)
            if i.key_pressed(egui::Key::P) && i.modifiers.shift && !in_param_sketch {
                self.start_param_sketch();
            }

            // Document-level undo/redo (outside any sub-editor)
            if !in_param_sketch {
                if i.key_pressed(egui::Key::Z) && i.modifiers.ctrl && !i.modifiers.shift {
                    self.undo_doc();
                }
                if (i.key_pressed(egui::Key::Y) && i.modifiers.ctrl)
                    || (i.key_pressed(egui::Key::Z) && i.modifiers.ctrl && i.modifiers.shift)
                {
                    self.redo_doc();
                }

                // Branch ops
                if i.modifiers.ctrl && i.key_pressed(egui::Key::B) {
                    self.do_branch_from_active();
                }
                if i.modifiers.ctrl && i.key_pressed(egui::Key::M) {
                    self.do_merge_other_branch();
                }
            }

            if in_param_sketch {
                if i.key_pressed(egui::Key::Escape) {
                    // Exit parametric sketch without committing
                    self.exit_param_sketch_mode(false);
                }
                if i.key_pressed(egui::Key::P) && !i.modifiers.ctrl && !i.modifiers.shift {
                    // Point tool
                    if let Some(SubEditorState::Sketch { tool, .. }) = &mut self.sub_editor {
                        *tool = SketchTool::Point;
                    }
                }
                if i.key_pressed(egui::Key::L) && !i.modifiers.ctrl {
                    // Line tool
                    if let Some(SubEditorState::Sketch { tool, line_start, .. }) = &mut self.sub_editor {
                        *tool = SketchTool::Line;
                        *line_start = None;
                    }
                }
                if i.key_pressed(egui::Key::R) && !i.modifiers.ctrl {
                    // Rectangle tool
                    if let Some(SubEditorState::Sketch { tool, rect_start, .. }) = &mut self.sub_editor {
                        *tool = SketchTool::Rectangle;
                        *rect_start = None;
                    }
                }
                if i.key_pressed(egui::Key::C) && !i.modifiers.ctrl {
                    // Circle tool
                    if let Some(SubEditorState::Sketch { tool, circle_center, .. }) = &mut self.sub_editor {
                        *tool = SketchTool::Circle;
                        *circle_center = None;
                    }
                }
                if i.key_pressed(egui::Key::A) && !i.modifiers.ctrl {
                    // Arc tool
                    if let Some(SubEditorState::Sketch { tool, arc_center, arc_start, .. }) = &mut self.sub_editor {
                        *tool = SketchTool::Arc;
                        *arc_center = None;
                        *arc_start = None;
                    }
                }
                if i.key_pressed(egui::Key::Z) && i.modifiers.ctrl {
                    // Ctrl+Z = Undo
                    if let Some(SubEditorState::Sketch { local_sketch, undo_stack, redo_stack, .. }) = &mut self.sub_editor {
                        if let Some(prev_sketch) = undo_stack.pop() {
                            redo_stack.push(local_sketch.clone());
                            *local_sketch = prev_sketch;
                            self.request_param_rebuild_soon();
                        }
                    }
                }
                if i.key_pressed(egui::Key::Y) && i.modifiers.ctrl {
                    // Ctrl+Y = Redo
                    if let Some(SubEditorState::Sketch { local_sketch, undo_stack, redo_stack, .. }) = &mut self.sub_editor {
                        if let Some(next_sketch) = redo_stack.pop() {
                            undo_stack.push(local_sketch.clone());
                            *local_sketch = next_sketch;
                            self.request_param_rebuild_soon();
                        }
                    }
                }
                if i.key_pressed(egui::Key::H) && !i.modifiers.ctrl {
                    // H = Apply horizontal constraint
                    if let Some(SubEditorState::Sketch {
                        local_sketch,
                        selected_entity,
                        ..
                    }) = &mut self.sub_editor {
                        if let Some(entity_id) = selected_entity {
                            if let Some(entity) = local_sketch.entity(*entity_id) {
                                if matches!(entity.geometry, SketchGeometry::LineSegment(_)) {
                                    local_sketch.add_constraint(
                                        mycad_kernel::sketch::SketchConstraintKind::Horizontal {
                                            line: *entity_id,
                                        },
                                        None,
                                    );
                                    *selected_entity = None;
                                }
                            }
                        }
                    }
                }
                if i.key_pressed(egui::Key::V) && !i.modifiers.ctrl {
                    // V = Apply vertical constraint
                    if let Some(SubEditorState::Sketch {
                        local_sketch,
                        selected_entity,
                        ..
                    }) = &mut self.sub_editor {
                        if let Some(entity_id) = selected_entity {
                            if let Some(entity) = local_sketch.entity(*entity_id) {
                                if matches!(entity.geometry, SketchGeometry::LineSegment(_)) {
                                    local_sketch.add_constraint(
                                        mycad_kernel::sketch::SketchConstraintKind::Vertical {
                                            line: *entity_id,
                                        },
                                        None,
                                    );
                                    *selected_entity = None;
                                }
                            }
                        }
                    }
                }
                if i.key_pressed(egui::Key::Delete) {
                    // Delete = Remove selected entity
                    if let Some(SubEditorState::Sketch {
                        local_sketch,
                        selected_entity,
                        undo_stack,
                        redo_stack,
                        ..
                    }) = &mut self.sub_editor {
                        if let Some(entity_id) = selected_entity {
                            undo_stack.push(local_sketch.clone());
                            redo_stack.clear();
                            local_sketch.remove_entity(*entity_id);
                            *selected_entity = None;
                            self.request_param_rebuild_soon();
                        }
                    }
                }
                if i.key_pressed(egui::Key::G) && !i.modifiers.ctrl {
                    // G = Toggle construction geometry
                    if let Some(SubEditorState::Sketch {
                        local_sketch,
                        selected_entity,
                        undo_stack,
                        redo_stack,
                        ..
                    }) = &mut self.sub_editor {
                        if let Some(&entity_id) = selected_entity.as_ref() {
                            if let Some(entity) = local_sketch.entity_mut(entity_id) {
                                entity.construction = !entity.construction;
                            }
                            undo_stack.push(local_sketch.clone());
                            redo_stack.clear();
                            self.request_param_rebuild_soon();
                        }
                    }
                }
                if i.key_pressed(egui::Key::Enter) {
                    // Enter = Commit parametric sketch
                    self.exit_param_sketch_mode(true);
                }
                if i.key_pressed(egui::Key::E) && !i.modifiers.ctrl {
                    // E = Commit sketch and create extrude
                    self.exit_param_sketch_mode(true);
                    self.perform_param_extrude(self.extrude_depth);
                }
            }
        });

        // Menu bar
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("New").clicked() {
                        self.snapshot_doc();
                        self.document = ParamDocument::new();
                        self.sub_editor = None;
                        self.rebuild_pending_since = None;
                        if let Some(viewport) = &mut self.viewport {
                            viewport.clear_mesh();
                            viewport.clear_sketch_lines();
                        }
                        self.status_message = "New document created".to_string();
                        ui.close_menu();
                    }
                    if ui.button("Open...").clicked() {
                        match self.load_param_document() {
                            Ok(_) => {
                                self.request_param_rebuild_soon();
                            }
                            Err(e) => {
                                self.status_message = e;
                            }
                        }
                        ui.close_menu();
                    }
                    if ui.button("Save...").clicked() {
                        if let Err(e) = self.save_param_document() {
                            self.status_message = e;
                        }
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Export STL").clicked() {
                        if let Err(e) = self.export_stl() {
                            self.status_message = e;
                        }
                        ui.close_menu();
                    }
                });
                ui.menu_button("Edit", |ui| {
                    let undo_label = format!("Undo  (Ctrl+Z)  [{}]", self.doc_undo.len());
                    if ui.add_enabled(!self.doc_undo.is_empty(), egui::Button::new(undo_label))
                        .clicked()
                    {
                        self.undo_doc();
                        ui.close_menu();
                    }
                    let redo_label = format!("Redo  (Ctrl+Y)  [{}]", self.doc_redo.len());
                    if ui.add_enabled(!self.doc_redo.is_empty(), egui::Button::new(redo_label))
                        .clicked()
                    {
                        self.redo_doc();
                        ui.close_menu();
                    }
                });
                ui.menu_button("Sketch", |ui| {
                    if !in_param_sketch {
                        ui.menu_button("New Parametric Sketch", |ui| {
                            if ui.button("On XY plane        S / Shift+P").clicked() {
                                self.start_param_sketch_on_plane(WorldRef::PlaneXY);
                                ui.close_menu();
                            }
                            if ui.button("On XZ plane").clicked() {
                                self.start_param_sketch_on_plane(WorldRef::PlaneXZ);
                                ui.close_menu();
                            }
                            if ui.button("On YZ plane").clicked() {
                                self.start_param_sketch_on_plane(WorldRef::PlaneYZ);
                                ui.close_menu();
                            }
                        });
                    } else if in_param_sketch {
                        if ui.button("Point Tool            P").clicked() {
                            if let Some(SubEditorState::Sketch { tool, .. }) = &mut self.sub_editor {
                                *tool = SketchTool::Point;
                            }
                            ui.close_menu();
                        }
                        if ui.button("Line Tool              L").clicked() {
                            if let Some(SubEditorState::Sketch { tool, line_start, .. }) = &mut self.sub_editor {
                                *tool = SketchTool::Line;
                                *line_start = None;
                            }
                            ui.close_menu();
                        }
                        if ui.button("Rectangle Tool    R").clicked() {
                            if let Some(SubEditorState::Sketch { tool, rect_start, .. }) = &mut self.sub_editor {
                                *tool = SketchTool::Rectangle;
                                *rect_start = None;
                            }
                            ui.close_menu();
                        }
                        if ui.button("Circle Tool        C").clicked() {
                            if let Some(SubEditorState::Sketch { tool, circle_center, .. }) = &mut self.sub_editor {
                                *tool = SketchTool::Circle;
                                *circle_center = None;
                            }
                            ui.close_menu();
                        }
                        if ui.button("Arc Tool              A").clicked() {
                            if let Some(SubEditorState::Sketch { tool, arc_center, arc_start, .. }) = &mut self.sub_editor {
                                *tool = SketchTool::Arc;
                                *arc_center = None;
                                *arc_start = None;
                            }
                            ui.close_menu();
                        }
                        ui.separator();
                        ui.horizontal(|ui| {
                            ui.label("Depth: ");
                            ui.add(egui::DragValue::new(&mut self.extrude_depth).speed(0.5).range(0.1..=100.0));
                        });
                        if ui.button("Extrude & Commit  E").clicked() {
                            self.exit_param_sketch_mode(true);
                            self.perform_param_extrude(self.extrude_depth);
                            ui.close_menu();
                        }
                        ui.separator();
                        ui.separator();
                        if ui.button("Clear Sketch").clicked() {
                            if let Some(SubEditorState::Sketch {
                                local_sketch,
                                undo_stack,
                                redo_stack,
                                ..
                            }) = &mut self.sub_editor {
                                undo_stack.push(local_sketch.clone());
                                redo_stack.clear();
                                local_sketch.entities.clear();
                                local_sketch.constraints.clear();
                                self.request_param_rebuild_soon();
                            }
                            ui.close_menu();
                        }
                        ui.menu_button("Constraints", |ui| {
                            if ui.button("Horizontal          H").clicked() {
                                if let Some(SubEditorState::Sketch {
                                    local_sketch,
                                    selected_entity,
                                    ..
                                }) = &mut self.sub_editor {
                                    if let Some(entity_id) = selected_entity {
                                        if let Some(entity) = local_sketch.entity(*entity_id) {
                                            if matches!(entity.geometry, SketchGeometry::LineSegment(_)) {
                                                local_sketch.add_constraint(
                                                    mycad_kernel::sketch::SketchConstraintKind::Horizontal {
                                                        line: *entity_id,
                                                    },
                                                    None,
                                                );
                                                *selected_entity = None;
                                            }
                                        }
                                    }
                                }
                                ui.close_menu();
                            }
                            if ui.button("Vertical              V").clicked() {
                                if let Some(SubEditorState::Sketch {
                                    local_sketch,
                                    selected_entity,
                                    ..
                                }) = &mut self.sub_editor {
                                    if let Some(entity_id) = selected_entity {
                                        if let Some(entity) = local_sketch.entity(*entity_id) {
                                            if matches!(entity.geometry, SketchGeometry::LineSegment(_)) {
                                                local_sketch.add_constraint(
                                                    mycad_kernel::sketch::SketchConstraintKind::Vertical {
                                                        line: *entity_id,
                                                    },
                                                    None,
                                                );
                                                *selected_entity = None;
                                            }
                                        }
                                    }
                                }
                                ui.close_menu();
                            }
                        });
                        ui.separator();
                        if ui.button("Commit              Return").clicked() {
                            self.exit_param_sketch_mode(true);
                            ui.close_menu();
                        }
                        if ui.button("Cancel              Esc").clicked() {
                            self.exit_param_sketch_mode(false);
                            ui.close_menu();
                        }
                    }
                });
                ui.menu_button("View", |ui| {
                    if let Some(viewport) = self.viewport.as_mut() {
                        if ui.button("Fit All").clicked() {
                            viewport.fit_all();
                            ui.close_menu();
                        }
                        if ui.button("Fit Mesh").clicked() {
                            viewport.fit_mesh();
                            ui.close_menu();
                        }
                        if ui.button("Reset View").clicked() {
                            viewport.set_standard_view(StandardView::Isometric);
                            ui.close_menu();
                        }
                        ui.separator();
                        for (label, view) in [
                            ("Front", StandardView::Front),
                            ("Back", StandardView::Back),
                            ("Left", StandardView::Left),
                            ("Right", StandardView::Right),
                            ("Top", StandardView::Top),
                            ("Bottom", StandardView::Bottom),
                            ("Isometric", StandardView::Isometric),
                        ] {
                            if ui.button(label).clicked() {
                                viewport.set_standard_view(view);
                                ui.close_menu();
                            }
                        }
                        ui.separator();
                        if ui.button("Toggle Projection").clicked() {
                            viewport.toggle_projection();
                            ui.close_menu();
                        }
                    } else {
                        ui.label("Viewport unavailable");
                    }
                });
            });
        });

        // Status bar
        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if in_param_sketch {
                    ui.label(egui::RichText::new("PARAMETRIC SKETCH").color(egui::Color32::from_rgb(100, 255, 100)));
                    if let Some(SubEditorState::Sketch { local_sketch, .. }) = &self.sub_editor {
                        ui.separator();
                        let entity_count = local_sketch.entities.len();
                        let constraint_count = local_sketch.constraints().count();
                        ui.label(format!("{} entities, {} constraints", entity_count, constraint_count));
                    }
                } else {
                    ui.label(&self.status_message);
                }
                ui.separator();
                ui.label("mm");
                if let Some(viewport) = self.viewport.as_ref() {
                    let camera = viewport.camera();
                    let pos = camera.position();
                    ui.separator();
                    ui.label(self.projection_label());
                    ui.separator();
                    ui.label(format!("dist {:.2}", camera.distance));
                    ui.separator();
                    ui.label(format!("cam {:.2}, {:.2}, {:.2}", pos.x, pos.y, pos.z));
                }
            });
        });

        // Right side panel: history DAG + components when not sketching;
        // selected-entity properties while sub-editor is active.
        egui::SidePanel::right("right_panel")
            .default_width(280.0)
            .show(ctx, |ui| {
                if in_param_sketch {
                    ui.heading("Sketch Properties");
                    ui.separator();
                    Self::draw_sub_editor_properties(ui, &self.sub_editor);
                } else {
                    let action = mycad_ui::history_panel::history_panel(
                        ui,
                        &self.document,
                        &mut self.history_state,
                    );
                    self.handle_history_panel_action(action);
                }
            });

        // Central viewport
        let viewport_response = egui::CentralPanel::default()
            .show(ctx, |ui| {
                egui::Frame::canvas(ui.style()).show(ui, |ui| {
                    if let Some(viewport) = self.viewport.as_mut() {
                        Some(viewport.ui(ui, in_param_sketch))
                    } else {
                        ui.centered_and_justified(|ui| {
                            ui.heading("3D Viewport unavailable");
                        });
                        None
                    }
                })
                .inner
            })
            .inner;

        if let Some(response) = viewport_response {
            if in_param_sketch {
                self.handle_param_sketch_input(&response);
            } else {
                self.handle_viewport_pick(&response);
            }
        }
        self.update_sketch_rendering();
    }
}
