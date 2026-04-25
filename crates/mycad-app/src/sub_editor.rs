use mycad_kernel::math::{Plane, Point2};
use mycad_kernel::parametric::feature::{InputRef, Operation, WorldRef};
use mycad_kernel::parametric::ops::datum_plane::CreateDatumPlaneOp;
use mycad_kernel::parametric::ops::sketch_op::CreateSketchOp;
use mycad_kernel::parametric::rebuild::mark_dirty;
use mycad_kernel::sketch::{LineSegment, Sketch, SketchEntityId, SketchGeometry};
use mycad_renderer::overlay::LineVertex;
use mycad_renderer::{StandardView, ViewportResponse};

use crate::{
    MyCadApp, SketchTool, SubEditorState, SNAP_GRID_SIZE, SNAP_POINT_THRESHOLD,
};

pub(crate) fn entity_type_name(geometry: &SketchGeometry) -> &'static str {
    match geometry {
        SketchGeometry::Point(_) => "Point",
        SketchGeometry::LineSegment(_) => "Line",
        SketchGeometry::Circle(_) => "Circle",
        SketchGeometry::Arc(_) => "Arc",
    }
}

impl MyCadApp {
    pub(crate) fn handle_param_sketch_input(&mut self, response: &ViewportResponse) {
        let mut should_rebuild = false;

        {
            let Some(SubEditorState::Sketch {
                local_sketch,
                tool,
                hover_point,
                snapped_point,
                line_start,
                rect_start,
                circle_center,
                arc_center,
                arc_start,
                undo_stack,
                redo_stack,
                selected_entity,
                solver_result,
                ..
            }) = &mut self.sub_editor
            else {
                return;
            };

            let Some(viewport) = &self.viewport else { return };

            if let Some(hover_pos) = response.hover_pos {
                let rect = viewport.last_rect();
                if let Some(sketch_pt) = viewport.screen_to_sketch_point(hover_pos, rect, &local_sketch.plane) {
                    *hover_point = Some(sketch_pt);

                    // Snap to existing points first
                    let mut snapped = None;
                    for entity in &local_sketch.entities {
                        match &entity.geometry {
                            SketchGeometry::Point(pt) => {
                                if sketch_pt.distance(pt.position) < SNAP_POINT_THRESHOLD {
                                    snapped = Some(pt.position);
                                    break;
                                }
                            }
                            SketchGeometry::LineSegment(line) => {
                                if sketch_pt.distance(line.start) < SNAP_POINT_THRESHOLD {
                                    snapped = Some(line.start);
                                    break;
                                }
                                if sketch_pt.distance(line.end) < SNAP_POINT_THRESHOLD {
                                    snapped = Some(line.end);
                                    break;
                                }
                            }
                            _ => {}
                        }
                    }

                    // Fall back to grid snapping
                    if snapped.is_none() {
                        snapped = Some(Point2::new(
                            (sketch_pt.x / SNAP_GRID_SIZE).round() * SNAP_GRID_SIZE,
                            (sketch_pt.y / SNAP_GRID_SIZE).round() * SNAP_GRID_SIZE,
                        ));
                    }

                    *snapped_point = snapped;
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

            // Point tool: single click to place point
            if *tool == SketchTool::Point && response.clicked {
                if let Some(snapped) = *snapped_point {
                    undo_stack.push(local_sketch.clone());
                    redo_stack.clear();
                    local_sketch.add_entity(
                        SketchGeometry::Point(mycad_kernel::sketch::SketchPoint {
                            position: snapped,
                        }),
                        false,
                        None,
                    );
                    should_rebuild = true;
                }
            }

            if *tool == SketchTool::Line && response.clicked {
                if let Some(snapped) = *snapped_point {
                    if let Some(start) = *line_start {
                        if start.distance(snapped) > 1.0e-4 {
                            undo_stack.push(local_sketch.clone());
                            redo_stack.clear();
                            local_sketch.add_entity(
                                SketchGeometry::LineSegment(LineSegment {
                                    start,
                                    end: snapped,
                                }),
                                false,
                                None,
                            );
                            *line_start = Some(snapped);
                            should_rebuild = true;
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
                            undo_stack.push(local_sketch.clone());
                            redo_stack.clear();
                            let min_x = start.x.min(snapped.x);
                            let min_y = start.y.min(snapped.y);
                            let max_x = start.x.max(snapped.x);
                            let max_y = start.y.max(snapped.y);

                            local_sketch.add_rectangle(
                                Point2::new(min_x, min_y),
                                Point2::new(max_x, max_y),
                            );

                            *rect_start = None;
                            should_rebuild = true;
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
                            undo_stack.push(local_sketch.clone());
                            redo_stack.clear();
                            local_sketch.add_entity(
                                SketchGeometry::Circle(mycad_kernel::sketch::Circle {
                                    center,
                                    radius,
                                }),
                                false,
                                None,
                            );

                            *circle_center = None;
                            should_rebuild = true;
                        }
                    } else {
                        *circle_center = Some(snapped);
                    }
                }
            }

            // Arc tool: three clicks - center, start point, end point
            if *tool == SketchTool::Arc && response.clicked {
                if let Some(snapped) = *snapped_point {
                    if let (Some(center), Some(start)) = (*arc_center, *arc_start) {
                        let radius = center.distance(start);
                        if radius > 1.0e-4 {
                            undo_stack.push(local_sketch.clone());
                            redo_stack.clear();
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
                            should_rebuild = true;
                        }
                    } else if arc_center.is_some() {
                        *arc_start = Some(snapped);
                    } else {
                        *arc_center = Some(snapped);
                    }
                }
            }

            // Entity selection: if clicking without an active tool, try to select an entity
            if response.clicked && *tool == SketchTool::None {
                if let Some(hover) = *hover_point {
                    // Try to find an entity near the hover point
                    let mut closest_entity: Option<SketchEntityId> = None;
                    let mut closest_distance = SNAP_POINT_THRESHOLD;

                    for entity in &local_sketch.entities {
                        let dist = match &entity.geometry {
                            SketchGeometry::Point(pt) => hover.distance(pt.position),
                            SketchGeometry::LineSegment(line) => {
                                let closest_pt = line.closest_point(hover);
                                hover.distance(closest_pt)
                            }
                            SketchGeometry::Circle(circle) => {
                                let radius_dist = hover.distance(circle.center).abs() - circle.radius;
                                radius_dist.abs()
                            }
                            SketchGeometry::Arc(arc) => {
                                let radius_dist = hover.distance(arc.center).abs() - arc.radius;
                                radius_dist.abs()
                            }
                        };

                        if dist < closest_distance {
                            closest_distance = dist;
                            closest_entity = Some(entity.id);
                        }
                    }

                    *selected_entity = closest_entity;
                }
            }

            // Run sketch solver if sketch was modified
            if should_rebuild {
                *solver_result = Some(Box::new(local_sketch.solve()));
            }
        }

        if should_rebuild {
            self.request_param_rebuild_soon();
        }
    }

    pub(crate) fn update_sketch_rendering(&mut self) {
        let Some(viewport) = &mut self.viewport else { return };
        if let Some(SubEditorState::Sketch {
            local_sketch,
            line_start,
            snapped_point,
            ..
        }) = &self.sub_editor {
            // Render parametric sketch entities
            let mut lines = Vec::new();

            // Render origin axis markers (prominent X and Y axes)
            const ORIGIN_AXIS_LENGTH: f32 = 2.5;
            // X axis (positive direction only)
            lines.extend(LineVertex::new(
                [0.0, 0.0, 0.0],
                [ORIGIN_AXIS_LENGTH, 0.0, 0.0],
            ));
            // Y axis (positive direction only)
            lines.extend(LineVertex::new(
                [0.0, 0.0, 0.0],
                [0.0, ORIGIN_AXIS_LENGTH, 0.0],
            ));
            // Small crosshair at origin for visibility
            lines.extend(LineVertex::new(
                [-0.3, 0.0, 0.0],
                [0.3, 0.0, 0.0],
            ));
            lines.extend(LineVertex::new(
                [0.0, -0.3, 0.0],
                [0.0, 0.3, 0.0],
            ));

            // Render grid (background grid to aid alignment)
            const GRID_RANGE: f64 = 20.0;
            for i in -20..=20 {
                let pos = (i as f64) * SNAP_GRID_SIZE;
                // Vertical grid lines
                lines.extend(LineVertex::new(
                    [pos as f32, -GRID_RANGE as f32, 0.0],
                    [pos as f32, GRID_RANGE as f32, 0.0],
                ));
                // Horizontal grid lines
                lines.extend(LineVertex::new(
                    [-GRID_RANGE as f32, pos as f32, 0.0],
                    [GRID_RANGE as f32, pos as f32, 0.0],
                ));
            }

            for entity in &local_sketch.entities {
                match &entity.geometry {
                    SketchGeometry::LineSegment(line) => {
                        lines.extend(LineVertex::new(
                            [line.start.x as f32, line.start.y as f32, 0.0],
                            [line.end.x as f32, line.end.y as f32, 0.0],
                        ));
                    }
                    SketchGeometry::Circle(circle) => {
                        // Draw circle as line segments (approximation)
                        const CIRCLE_SEGMENTS: usize = 32;
                        for i in 0..CIRCLE_SEGMENTS {
                            let angle1 = 2.0 * std::f64::consts::PI * (i as f64) / (CIRCLE_SEGMENTS as f64);
                            let angle2 = 2.0 * std::f64::consts::PI * ((i + 1) as f64) / (CIRCLE_SEGMENTS as f64);
                            let p1 = Point2::new(
                                circle.center.x + circle.radius * angle1.cos(),
                                circle.center.y + circle.radius * angle1.sin(),
                            );
                            let p2 = Point2::new(
                                circle.center.x + circle.radius * angle2.cos(),
                                circle.center.y + circle.radius * angle2.sin(),
                            );
                            lines.extend(LineVertex::new(
                                [p1.x as f32, p1.y as f32, 0.0],
                                [p2.x as f32, p2.y as f32, 0.0],
                            ));
                        }
                    }
                    SketchGeometry::Arc(arc) => {
                        // Draw arc as line segments (approximation)
                        const ARC_SEGMENTS: usize = 16;
                        let angle_diff = arc.end_angle - arc.start_angle;
                        for i in 0..ARC_SEGMENTS {
                            let t1 = i as f64 / ARC_SEGMENTS as f64;
                            let t2 = (i + 1) as f64 / ARC_SEGMENTS as f64;
                            let angle1 = arc.start_angle + angle_diff * t1;
                            let angle2 = arc.start_angle + angle_diff * t2;
                            let p1 = Point2::new(
                                arc.center.x + arc.radius * angle1.cos(),
                                arc.center.y + arc.radius * angle1.sin(),
                            );
                            let p2 = Point2::new(
                                arc.center.x + arc.radius * angle2.cos(),
                                arc.center.y + arc.radius * angle2.sin(),
                            );
                            lines.extend(LineVertex::new(
                                [p1.x as f32, p1.y as f32, 0.0],
                                [p2.x as f32, p2.y as f32, 0.0],
                            ));
                        }
                    }
                    _ => {}
                }
            }

            // Show preview line if in line mode
            if let Some(start) = line_start {
                if let Some(snapped) = snapped_point {
                    lines.extend(LineVertex::new(
                        [start.x as f32, start.y as f32, 0.0],
                        [snapped.x as f32, snapped.y as f32, 0.0],
                    ));
                }
            }

            // Show snap indicator circle at snapped point
            if let Some(snapped) = snapped_point {
                const SNAP_INDICATOR_RADIUS: f64 = 0.3;
                const SNAP_INDICATOR_SEGMENTS: usize = 16;
                for i in 0..SNAP_INDICATOR_SEGMENTS {
                    let angle1 = 2.0 * std::f64::consts::PI * (i as f64) / (SNAP_INDICATOR_SEGMENTS as f64);
                    let angle2 = 2.0 * std::f64::consts::PI * ((i + 1) as f64) / (SNAP_INDICATOR_SEGMENTS as f64);
                    let p1 = Point2::new(
                        snapped.x + SNAP_INDICATOR_RADIUS * angle1.cos(),
                        snapped.y + SNAP_INDICATOR_RADIUS * angle1.sin(),
                    );
                    let p2 = Point2::new(
                        snapped.x + SNAP_INDICATOR_RADIUS * angle2.cos(),
                        snapped.y + SNAP_INDICATOR_RADIUS * angle2.sin(),
                    );
                    lines.extend(LineVertex::new(
                        [p1.x as f32, p1.y as f32, 0.0],
                        [p2.x as f32, p2.y as f32, 0.0],
                    ));
                }
            }

            viewport.set_sketch_lines(lines);
        } else {
            viewport.clear_sketch_lines();
        }
    }

    /// Start a parametric sketch on the XY world plane.
    ///
    /// Creates a CreateDatumPlaneOp for the world XY plane and a CreateSketchOp on that plane.
    /// Initializes SubEditorState::Sketch for local editing with undo/redo support.
    /// Schedules a deferred rebuild via rebuild_pending_since.
    ///
    /// This is part of the staged migration to parametric architecture. Currently, the legacy
    /// sketch_session path is the primary UI flow; this method provides the parametric pathway
    /// that will eventually replace it.
    #[allow(dead_code)]
    pub(crate) fn start_param_sketch(&mut self) {
        self.start_param_sketch_on_plane(WorldRef::PlaneXY);
    }

    pub(crate) fn start_param_sketch_on_plane(&mut self, world_ref: WorldRef) {
        let (plane_name, plane) = match world_ref {
            WorldRef::PlaneXY => ("XY", Plane::xy()),
            WorldRef::PlaneXZ => ("XZ", Plane::xz()),
            WorldRef::PlaneYZ => ("YZ", Plane::yz()),
            _ => ("XY", Plane::xy()),
        };

        self.snapshot_doc();

        // Create a datum plane operation on the specified world plane.
        let datum_result = self.document.append_op(Operation::CreateDatumPlane(
            CreateDatumPlaneOp::world(world_ref, plane_name),
        ));

        if datum_result.is_err() {
            self.status_message = "Failed to create datum plane".to_string();
            return;
        }

        // Create a sketch on that datum plane.
        let sketch = Sketch::new(plane);
        let sketch_op = CreateSketchOp::on_datum_plane(
            InputRef::World(world_ref),
            sketch.clone(),
            "Sketch",
        );
        let sketch_result = self.document.append_op(Operation::CreateSketch(Box::new(sketch_op)));

        if let Ok(sketch_node) = sketch_result {
            let mut local_sketch = Box::new(sketch);
            let initial_solve = Some(Box::new(local_sketch.solve()));
            self.sub_editor = Some(SubEditorState::Sketch {
                node_id: sketch_node,
                local_sketch,
                undo_stack: Vec::new(),
                redo_stack: Vec::new(),
                tool: SketchTool::None,
                hover_point: None,
                snapped_point: None,
                line_start: None,
                rect_start: None,
                circle_center: None,
                arc_center: None,
                arc_start: None,
                selected_entity: None,
                solver_result: initial_solve,
            });
            self.rebuild_pending_since = Some(std::time::Instant::now());
            self.status_message = format!("Parametric sketch started on {} plane", plane_name);

            // Auto-switch viewport to appropriate view for the sketch plane
            if let Some(viewport) = &mut self.viewport {
                let view = match world_ref {
                    WorldRef::PlaneXY => StandardView::Top,
                    WorldRef::PlaneXZ => StandardView::Front,
                    WorldRef::PlaneYZ => StandardView::Right,
                    _ => StandardView::Top,
                };
                viewport.set_standard_view(view);
            }
        } else {
            self.status_message = "Failed to create sketch".to_string();
        }
    }

    /// Commit the current parametric sketch edits back to the document.
    ///
    /// Updates the CreateSketchOp in the document with the local_sketch modifications.
    /// Marks the sketch node as dirty so the rebuild engine propagates the change downstream.
    /// Clears sub_editor but schedules a rebuild via request_param_rebuild_soon.
    ///
    /// Returns Err if not in sketch editing mode or if the node lookup fails.
    #[allow(dead_code)]
    pub(crate) fn commit_param_sketch_edits(&mut self) -> Result<(), String> {
        let sub_editor = match &mut self.sub_editor {
            Some(SubEditorState::Sketch { local_sketch, node_id, .. }) => {
                (*node_id, local_sketch.clone())
            }
            _ => return Err("Not in sketch editing mode".to_string()),
        };

        let (sketch_node_id, edited_sketch) = sub_editor;

        self.snapshot_doc();

        // Update the sketch in the document.
        {
            let node = self.document.node_mut(sketch_node_id)
                .map_err(|e| format!("Sketch node not found: {:?}", e))?;
            if let Operation::CreateSketch(op) = &mut node.operation {
                op.sketch = (*edited_sketch).clone();
            } else {
                return Err("Expected CreateSketch operation".to_string());
            }
        }

        mark_dirty(&mut self.document, sketch_node_id)
            .map_err(|e| format!("Failed to mark dirty: {:?}", e))?;

        self.request_param_rebuild_soon();
        Ok(())
    }

    /// Cancel parametric sketch edits without committing.
    ///
    /// Discards local_sketch modifications and clears SubEditorState.
    /// Does not mark the sketch node as dirty, so no rebuild is triggered.
    #[allow(dead_code)]
    pub(crate) fn cancel_param_sketch_edits(&mut self) {
        self.sub_editor = None;
        self.rebuild_pending_since = None;
        self.status_message = "Sketch editing cancelled".to_string();
    }

    pub(crate) fn exit_param_sketch_mode(&mut self, commit: bool) {
        if commit {
            if let Err(e) = self.commit_param_sketch_edits() {
                self.status_message = format!("Failed to commit: {}", e);
                return;
            }
            self.status_message = "Parametric sketch committed, rebuilding...".to_string();
        } else {
            self.cancel_param_sketch_edits();
            self.status_message = "Parametric sketch cancelled".to_string();
        }
        if let Some(viewport) = &mut self.viewport {
            viewport.clear_sketch_lines();
        }
    }
}

