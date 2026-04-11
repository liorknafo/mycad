use eframe::egui;
use mycad_kernel::math::{Plane, Point2, Scalar};
use mycad_kernel::sketch::{
    LineSegment, Sketch, SketchEntity, SketchEntityId, SketchGeometry,
};
use mycad_kernel::features::{ExtrudeParams, extrude};
use mycad_kernel::tessellation::tessellate_solid;
use mycad_renderer::overlay::LineVertex;
use mycad_renderer::{ProjectionMode, StandardView, Viewport3d, ViewportResponse};
use mycad_kernel::parametric::types::{Document as ParamDocument, NodeId};
use mycad_kernel::parametric::ops::extrude_op::{ExtrudeOp, ExtrudeDirection, ProfileRef};
use mycad_kernel::parametric::ops::datum_plane::CreateDatumPlaneOp;
use mycad_kernel::parametric::ops::sketch_op::CreateSketchOp;
use mycad_kernel::parametric::feature::{InputRef, Operation, WorldRef};
use mycad_kernel::parametric::rebuild::{rebuild, mark_dirty};

const SNAP_GRID_SIZE: Scalar = 1.0;
const SNAP_POINT_THRESHOLD: Scalar = 0.3;
const SNAP_GRID_THRESHOLD: Scalar = 0.15;
const SKETCH_LINE_COLOR: [f32; 4] = [0.2, 0.8, 1.0, 1.0];
const SKETCH_PREVIEW_COLOR: [f32; 4] = [0.5, 0.5, 1.0, 0.6];
const SKETCH_POINT_COLOR: [f32; 4] = [1.0, 0.8, 0.2, 1.0];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SketchTool {
    None,
    Point,
    Line,
    Rectangle,
    Circle,
    Arc,
}

/// Local editing state for parametric feature editing within the document.
///
/// When a user enters the parametric sketch editing mode (future UI), MyCadApp
/// creates a SubEditorState that holds the local copy of the geometry being edited,
/// plus undo/redo stacks and tool state. On commit, the local edits are pushed back
/// to the document node, the node is marked dirty, and a rebuild is scheduled.
///
/// This enum exists as part of the staged migration to parametric architecture.
/// Currently the legacy sketch_session is the primary UI; SubEditorState provides
/// the parametric pathway that will eventually replace it. Both can coexist during
/// transition (sketch_session for legacy flow, SubEditorState for new flow).
pub enum SubEditorState {
    Sketch {
        node_id: NodeId,
        local_sketch: Box<Sketch>,
        undo_stack: Vec<Box<Sketch>>,
        redo_stack: Vec<Box<Sketch>>,
        tool: SketchTool,
        hover_point: Option<Point2>,
        snapped_point: Option<Point2>,
        line_start: Option<Point2>,
        rect_start: Option<Point2>,
        circle_center: Option<Point2>,
        arc_center: Option<Point2>,
        arc_start: Option<Point2>,
        selected_entity: Option<SketchEntityId>,
    },
    ExtrudeParams {
        node_id: NodeId,
        pending: Box<ExtrudeOp>,
        undo_stack: Vec<Box<ExtrudeOp>>,
        redo_stack: Vec<Box<ExtrudeOp>>,
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

pub struct SketchSession {
    pub sketch: Sketch,
    pub tool: SketchTool,
    pub hover_point: Option<Point2>,
    pub snapped_point: Option<Point2>,
    pub line_start: Option<Point2>,
    pub rect_start: Option<Point2>,
    pub circle_center: Option<Point2>,
    pub arc_center: Option<Point2>,
    pub arc_start: Option<Point2>,
}

impl SketchSession {
    pub fn new(plane: Plane) -> Self {
        Self {
            sketch: Sketch::new(plane),
            tool: SketchTool::Line,
            hover_point: None,
            snapped_point: None,
            line_start: None,
            rect_start: None,
            circle_center: None,
            arc_center: None,
            arc_start: None,
        }
    }

    pub fn world_xy() -> Self {
        Self {
            sketch: Sketch::world_xy(),
            tool: SketchTool::Line,
            hover_point: None,
            snapped_point: None,
            line_start: None,
            rect_start: None,
            circle_center: None,
            arc_center: None,
            arc_start: None,
        }
    }

    pub fn snap_point(&self, raw: Point2) -> Point2 {
        // 1. Snap to existing sketch points (highest priority)
        let mut best_dist = SNAP_POINT_THRESHOLD;
        let mut best_point: Option<Point2> = None;
        for entity in self.sketch.iter() {
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

        // 2. Snap to nearest point on existing lines
        for entity in self.sketch.iter() {
            if let SketchGeometry::LineSegment(line) = &entity.geometry {
                let closest = line.closest_point(raw);
                if closest.distance(raw) < SNAP_POINT_THRESHOLD {
                    // Snap to the closest point on the line, but also grid-align it
                    let snapped = grid_snap_nearby(closest, SNAP_GRID_THRESHOLD);
                    return snapped;
                }
            }
        }

        // 3. Grid snap: only snap axes independently when close to a grid line
        grid_snap_nearby(raw, SNAP_GRID_THRESHOLD)
    }

    pub fn build_sketch_lines(&self) -> Vec<LineVertex> {
        let mut vertices = Vec::new();
        for entity in self.sketch.iter() {
            match &entity.geometry {
                SketchGeometry::LineSegment(line) => {
                    let start = self.sketch.local_point_to_world(line.start);
                    let end = self.sketch.local_point_to_world(line.end);
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
                    let world = self.sketch.local_point_to_world(pt.position);
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
                    let center_world = self.sketch.local_point_to_world(circle.center);
                    let cx = center_world.x as f32;
                    let cy = center_world.y as f32;
                    let cz = center_world.z as f32;
                    let radius = circle.radius as f32;
                    let segments = 64;
                    let c = SKETCH_LINE_COLOR;
                    for i in 0..segments {
                        let angle1 = (i as f32 / segments as f32) * std::f32::consts::TAU;
                        let angle2 = ((i + 1) as f32 / segments as f32) * std::f32::consts::TAU;
                        vertices.push(LineVertex { position: [cx + radius * angle1.cos(), cy + radius * angle1.sin(), cz], color: c });
                        vertices.push(LineVertex { position: [cx + radius * angle2.cos(), cy + radius * angle2.sin(), cz], color: c });
                    }
                }
                SketchGeometry::Arc(arc) => {
                    let center_world = self.sketch.local_point_to_world(arc.center);
                    let cx = center_world.x as f32;
                    let cy = center_world.y as f32;
                    let cz = center_world.z as f32;
                    let radius = arc.radius as f32;
                    // Normalize angles
                    let mut start_angle = arc.start_angle;
                    let mut end_angle = arc.end_angle;
                    while start_angle < 0.0 { start_angle += std::f64::consts::TAU; }
                    while end_angle < start_angle { end_angle += std::f64::consts::TAU; }
                    let sweep = end_angle - start_angle;
                    let segments = 64.max((sweep * 32.0) as usize);
                    let c = SKETCH_LINE_COLOR;
                    for i in 0..segments {
                        let t1 = i as f64 / segments as f64;
                        let t2 = (i + 1) as f64 / segments as f64;
                        let a1 = (start_angle + sweep * t1) as f32;
                        let a2 = (start_angle + sweep * t2) as f32;
                        vertices.push(LineVertex { position: [cx + radius * a1.cos(), cy + radius * a1.sin(), cz], color: c });
                        vertices.push(LineVertex { position: [cx + radius * a2.cos(), cy + radius * a2.sin(), cz], color: c });
                    }
                }
            }
        }

        if let (Some(start), Some(end)) = (self.line_start, self.snapped_point) {
            let ws = self.sketch.local_point_to_world(start);
            let we = self.sketch.local_point_to_world(end);
            vertices.push(LineVertex {
                position: [ws.x as f32, ws.y as f32, ws.z as f32],
                color: SKETCH_PREVIEW_COLOR,
            });
            vertices.push(LineVertex {
                position: [we.x as f32, we.y as f32, we.z as f32],
                color: SKETCH_PREVIEW_COLOR,
            });
        }

        // Rectangle preview: show 4 edges when rect_start is set
        if let (Some(start), Some(end)) = (self.rect_start, self.snapped_point) {
            let min_x = start.x.min(end.x);
            let min_y = start.y.min(end.y);
            let max_x = start.x.max(end.x);
            let max_y = start.y.max(end.y);
            let corners = [
                Point2::new(min_x, min_y),
                Point2::new(max_x, min_y),
                Point2::new(max_x, max_y),
                Point2::new(min_x, max_y),
            ];
            for i in 0..4 {
                let ws = self.sketch.local_point_to_world(corners[i]);
                let we = self.sketch.local_point_to_world(corners[(i + 1) % 4]);
                vertices.push(LineVertex {
                    position: [ws.x as f32, ws.y as f32, ws.z as f32],
                    color: SKETCH_PREVIEW_COLOR,
                });
                vertices.push(LineVertex {
                    position: [we.x as f32, we.y as f32, we.z as f32],
                    color: SKETCH_PREVIEW_COLOR,
                });
            }
        }

        // Circle preview: show circle outline when center is set
        if let (Some(center), Some(end)) = (self.circle_center, self.snapped_point) {
            let radius = center.distance(end);
            if radius > 1.0e-4 {
                let center_world = self.sketch.local_point_to_world(center);
                let cx = center_world.x as f32;
                let cy = center_world.y as f32;
                let cz = center_world.z as f32;
                let r = radius as f32;
                let segments = 64;
                for i in 0..segments {
                    let angle1 = (i as f32 / segments as f32) * std::f32::consts::TAU;
                    let angle2 = ((i + 1) as f32 / segments as f32) * std::f32::consts::TAU;
                    vertices.push(LineVertex { position: [cx + r * angle1.cos(), cy + r * angle1.sin(), cz], color: SKETCH_PREVIEW_COLOR });
                    vertices.push(LineVertex { position: [cx + r * angle2.cos(), cy + r * angle2.sin(), cz], color: SKETCH_PREVIEW_COLOR });
                }
                // Radius line
                let end_world = self.sketch.local_point_to_world(end);
                vertices.push(LineVertex { position: [cx, cy, cz], color: SKETCH_PREVIEW_COLOR });
                vertices.push(LineVertex { position: [end_world.x as f32, end_world.y as f32, end_world.z as f32], color: SKETCH_PREVIEW_COLOR });
            }
        }

        // Arc preview
        if let (Some(center), Some(start)) = (self.arc_center, self.arc_start) {
            if let Some(end) = self.snapped_point {
                let radius = center.distance(start);
                if radius > 1.0e-4 {
                    let center_world = self.sketch.local_point_to_world(center);
                    let cx = center_world.x as f32;
                    let cy = center_world.y as f32;
                    let cz = center_world.z as f32;
                    let r = radius as f32;
                    let start_angle = (start.y - center.y).atan2(start.x - center.x);
                    let end_angle = (end.y - center.y).atan2(end.x - center.x);
                    let mut sweep = end_angle - start_angle;
                    while sweep < 0.0 { sweep += std::f64::consts::TAU; }
                    let segments = 64.max((sweep * 32.0) as usize);
                    for i in 0..segments {
                        let t1 = i as f64 / segments as f64;
                        let t2 = (i + 1) as f64 / segments as f64;
                        let a1 = (start_angle + sweep * t1) as f32;
                        let a2 = (start_angle + sweep * t2) as f32;
                        vertices.push(LineVertex { position: [cx + r * a1.cos(), cy + r * a1.sin(), cz], color: SKETCH_PREVIEW_COLOR });
                        vertices.push(LineVertex { position: [cx + r * a2.cos(), cy + r * a2.sin(), cz], color: SKETCH_PREVIEW_COLOR });
                    }
                    // Center-to-start and center-to-end lines
                    let start_world = self.sketch.local_point_to_world(start);
                    let end_world = self.sketch.local_point_to_world(end);
                    vertices.push(LineVertex { position: [cx, cy, cz], color: SKETCH_PREVIEW_COLOR });
                    vertices.push(LineVertex { position: [start_world.x as f32, start_world.y as f32, start_world.z as f32], color: SKETCH_PREVIEW_COLOR });
                    vertices.push(LineVertex { position: [cx, cy, cz], color: SKETCH_PREVIEW_COLOR });
                    vertices.push(LineVertex { position: [end_world.x as f32, end_world.y as f32, end_world.z as f32], color: SKETCH_PREVIEW_COLOR });
                }
            }
        }

        if let Some(snap) = self.snapped_point {
            let world = self.sketch.local_point_to_world(snap);
            let size = 0.08_f32;
            let wx = world.x as f32;
            let wy = world.y as f32;
            let wz = world.z as f32;
            let c = [1.0, 1.0, 0.3, 0.8];
            vertices.push(LineVertex { position: [wx - size, wy - size, wz], color: c });
            vertices.push(LineVertex { position: [wx + size, wy - size, wz], color: c });
            vertices.push(LineVertex { position: [wx + size, wy - size, wz], color: c });
            vertices.push(LineVertex { position: [wx + size, wy + size, wz], color: c });
            vertices.push(LineVertex { position: [wx + size, wy + size, wz], color: c });
            vertices.push(LineVertex { position: [wx - size, wy + size, wz], color: c });
            vertices.push(LineVertex { position: [wx - size, wy + size, wz], color: c });
            vertices.push(LineVertex { position: [wx - size, wy - size, wz], color: c });
        }

        vertices
    }
}

fn grid_snap_nearby(point: Point2, threshold: Scalar) -> Point2 {
    let snapped_x = (point.x / SNAP_GRID_SIZE).round() * SNAP_GRID_SIZE;
    let snapped_y = (point.y / SNAP_GRID_SIZE).round() * SNAP_GRID_SIZE;
    Point2::new(
        if (point.x - snapped_x).abs() < threshold { snapped_x } else { point.x },
        if (point.y - snapped_y).abs() < threshold { snapped_y } else { point.y },
    )
}

fn sketch_entity_points(entity: &SketchEntity) -> Vec<Point2> {
    match &entity.geometry {
        SketchGeometry::Point(pt) => vec![pt.position],
        SketchGeometry::LineSegment(line) => vec![line.start, line.end],
        SketchGeometry::Circle(circle) => vec![circle.center],
        SketchGeometry::Arc(arc) => vec![arc.center, arc.start_point(), arc.end_point()],
    }
}

#[allow(dead_code)]
pub struct MyCadApp {
    viewport: Option<Viewport3d>,
    sketch_session: Option<SketchSession>,
    status_message: String,
    extrude_depth: Scalar,
    // Parametric framework (migration in progress)
    document: ParamDocument,
    sub_editor: Option<SubEditorState>,
    rebuild_pending_since: Option<std::time::Instant>,
}

impl MyCadApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        Self {
            viewport: Viewport3d::new(cc),
            sketch_session: None,
            status_message: "Press S to enter Sketch mode".to_string(),
            extrude_depth: 5.0,
            document: ParamDocument::new(),
            sub_editor: None,
            rebuild_pending_since: None,
        }
    }

    fn is_sketch_mode(&self) -> bool {
        self.sketch_session.is_some()
    }

    fn is_param_sketch_mode(&self) -> bool {
        matches!(&self.sub_editor, Some(SubEditorState::Sketch { .. }))
    }

    fn projection_label(&self) -> &'static str {
        match self.viewport.as_ref().map(|v| v.camera().projection) {
            Some(ProjectionMode::Orthographic) => "ortho",
            _ => "perspective",
        }
    }

    fn enter_sketch_mode(&mut self) {
        self.sketch_session = Some(SketchSession::world_xy());
        self.status_message = "Sketch mode: Draw lines/rectangles, then press E to extrude".to_string();
        // Clear any existing mesh when entering sketch mode
        if let Some(viewport) = &mut self.viewport {
            viewport.clear_mesh();
        }
    }

    fn exit_sketch_mode(&mut self) {
        if let Some(session) = &mut self.sketch_session {
            session.line_start = None;
            session.rect_start = None;
            session.circle_center = None;
            session.arc_center = None;
            session.arc_start = None;
            session.tool = SketchTool::None;
        }
        self.sketch_session = None;
        if let Some(viewport) = &mut self.viewport {
            viewport.clear_sketch_lines();
        }
        self.status_message = "Press S to enter Sketch mode".to_string();
    }

    fn exit_param_sketch_mode(&mut self, commit: bool) {
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

    /// Save the parametric document to a JSON file.
    fn save_param_document(&mut self, path: &str) -> Result<(), String> {
        let json = serde_json::to_string_pretty(&self.document)
            .map_err(|e| format!("Serialization failed: {}", e))?;

        std::fs::write(path, json)
            .map_err(|e| format!("Failed to write file: {}", e))?;

        self.status_message = format!("Document saved to: {}", path);
        Ok(())
    }

    /// Load a parametric document from a JSON file.
    fn load_param_document(&mut self, path: &str) -> Result<(), String> {
        let contents = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read file: {}", e))?;

        let document = serde_json::from_str(&contents)
            .map_err(|e| format!("Deserialization failed: {}", e))?;

        self.document = document;
        self.sub_editor = None;
        self.rebuild_pending_since = None;

        self.status_message = format!("Document loaded from: {}", path);
        Ok(())
    }

    /// Create an extrude operation on the most recent parametric sketch.
    fn perform_param_extrude(&mut self, depth: Scalar) {
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

    fn perform_extrude(&mut self, distance: Scalar) {
        let Some(session) = &self.sketch_session else {
            self.status_message = "Cannot extrude: not in sketch mode".to_string();
            return;
        };
        let sketch = &session.sketch;
        
        if sketch.entities.is_empty() {
            self.status_message = "Cannot extrude: sketch is empty".to_string();
            return;
        }

        // Count line segments
        let line_count = sketch.entities.iter().filter(|e| matches!(&e.geometry, SketchGeometry::LineSegment(_))).count();
        if line_count == 0 {
            self.status_message = "Cannot extrude: need line segments to form a closed profile".to_string();
            return;
        }

        // Try to extract wire first for better error reporting
        match sketch.extract_closed_wire() {
            Ok(wire) => {
                self.status_message = format!("Found {} edges, extruding with depth {}...", wire.edges.len(), distance);
            }
            Err(e) => {
                self.status_message = format!("Cannot find closed profile: {:?}. Draw lines that form a closed loop (e.g. use Rectangle tool).", e);
                return;
            }
        }

        // Need to borrow again since we mutably borrowed above for status
        let Some(session) = &self.sketch_session else { return; };
        let sketch = &session.sketch;
        let params = ExtrudeParams::new(distance);
        match extrude(sketch, params) {
            Ok(result) => {
                match tessellate_solid(&result.model, result.solid_id) {
                    Ok(mesh) => {
                        let vertex_count = mesh.vertices.len();
                        let tri_count = mesh.indices.len();
                        self.status_message = format!(
                            "Extruded! {} vertices, {} triangles",
                            vertex_count, tri_count
                        );
                        if let Some(viewport) = &mut self.viewport {
                            viewport.set_mesh(Some(mesh));
                        }
                        // Exit sketch mode after successful extrusion
                        self.sketch_session = None;
                        if let Some(viewport) = &mut self.viewport {
                            viewport.clear_sketch_lines();
                        }
                    }
                    Err(e) => {
                        self.status_message = format!("Tessellation error: {}", e);
                    }
                }
            }
            Err(e) => {
                self.status_message = format!("Extrude error: {}", e);
            }
        }
    }

    fn handle_param_sketch_input(&mut self, response: &ViewportResponse) {
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
        }

        if should_rebuild {
            self.request_param_rebuild_soon();
        }
    }

    fn handle_sketch_input(&mut self, response: &ViewportResponse) {
        let Some(session) = &mut self.sketch_session else { return };
        let Some(viewport) = &self.viewport else { return };

        if let Some(hover_pos) = response.hover_pos {
            let rect = viewport.last_rect();
            if let Some(sketch_pt) = viewport.screen_to_sketch_point(hover_pos, rect, &session.sketch.plane) {
                session.hover_point = Some(sketch_pt);
                session.snapped_point = Some(session.snap_point(sketch_pt));
            }
        }

        if response.escape_pressed {
            if session.line_start.is_some() {
                session.line_start = None;
            } else if session.rect_start.is_some() {
                session.rect_start = None;
            } else if session.circle_center.is_some() {
                session.circle_center = None;
            } else if session.arc_center.is_some() || session.arc_start.is_some() {
                session.arc_center = None;
                session.arc_start = None;
            } else {
                session.tool = SketchTool::None;
            }
            return;
        }

        if session.tool == SketchTool::Line && response.clicked {
            if let Some(snapped) = session.snapped_point {
                if let Some(start) = session.line_start {
                    if start.distance(snapped) > 1.0e-4 {
                        session.sketch.add_entity(
                            SketchGeometry::LineSegment(LineSegment {
                                start,
                                end: snapped,
                            }),
                            false,
                            None,
                        );
                        session.line_start = Some(snapped);
                    }
                } else {
                    session.line_start = Some(snapped);
                }
            }
        }

        if session.tool == SketchTool::Rectangle && response.clicked {
            if let Some(snapped) = session.snapped_point {
                if let Some(start) = session.rect_start {
                    if start.distance(snapped) > 1.0e-4 {
                        // Use Sketch::add_rectangle which properly creates
                        // connected line segments with constraints
                        let min_x = start.x.min(snapped.x);
                        let min_y = start.y.min(snapped.y);
                        let max_x = start.x.max(snapped.x);
                        let max_y = start.y.max(snapped.y);
                        
                        session.sketch.add_rectangle(
                            Point2::new(min_x, min_y),
                            Point2::new(max_x, max_y),
                        );
                        
                        // Reset for next rectangle
                        session.rect_start = None;
                    }
                } else {
                    session.rect_start = Some(snapped);
                }
            }
        }

        // Circle tool: two clicks - center and radius point
        if session.tool == SketchTool::Circle && response.clicked {
            if let Some(snapped) = session.snapped_point {
                if let Some(center) = session.circle_center {
                    let radius = center.distance(snapped);
                    if radius > 1.0e-4 {
                        session.sketch.add_entity(
                            SketchGeometry::Circle(mycad_kernel::sketch::Circle {
                                center,
                                radius,
                            }),
                            false,
                            None,
                        );
                        session.circle_center = None;
                    }
                } else {
                    session.circle_center = Some(snapped);
                }
            }
        }

        // Arc tool: three clicks - center, start point, end point
        if session.tool == SketchTool::Arc && response.clicked {
            if let Some(snapped) = session.snapped_point {
                if let (Some(center), Some(start)) = (session.arc_center, session.arc_start) {
                    let radius = center.distance(start);
                    if radius > 1.0e-4 {
                        let start_angle = (start.y - center.y).atan2(start.x - center.x);
                        let end_angle = (snapped.y - center.y).atan2(snapped.x - center.x);
                        session.sketch.add_entity(
                            SketchGeometry::Arc(mycad_kernel::sketch::Arc {
                                center,
                                radius,
                                start_angle,
                                end_angle,
                            }),
                            false,
                            None,
                        );
                        session.arc_center = None;
                        session.arc_start = None;
                    }
                } else if session.arc_center.is_some() {
                    session.arc_start = Some(snapped);
                } else {
                    session.arc_center = Some(snapped);
                }
            }
        }
    }

    fn update_sketch_rendering(&mut self) {
        let Some(viewport) = &mut self.viewport else { return };
        if let Some(session) = &self.sketch_session {
            viewport.set_sketch_lines(session.build_sketch_lines());
        } else if let Some(SubEditorState::Sketch {
            local_sketch,
            line_start,
            snapped_point,
            ..
        }) = &self.sub_editor {
            // Render parametric sketch entities
            let mut lines = Vec::new();

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
    fn start_param_sketch(&mut self) {
        // Create a datum plane operation on the world XY plane.
        let datum_result = self.document.append_op(Operation::CreateDatumPlane(
            CreateDatumPlaneOp::world(WorldRef::PlaneXY, "XY"),
        ));

        if datum_result.is_err() {
            self.status_message = "Failed to create datum plane".to_string();
            return;
        }

        // Create a sketch on that datum plane.
        let sketch = Sketch::world_xy();
        let sketch_op = CreateSketchOp::on_datum_plane(
            InputRef::World(WorldRef::PlaneXY),
            sketch.clone(),
            "Sketch",
        );
        let sketch_result = self.document.append_op(Operation::CreateSketch(Box::new(sketch_op)));

        if let Ok(sketch_node) = sketch_result {
            self.sub_editor = Some(SubEditorState::Sketch {
                node_id: sketch_node,
                local_sketch: Box::new(sketch),
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
            });
            self.rebuild_pending_since = Some(std::time::Instant::now());
            self.status_message = "Parametric sketch started (experimental)".to_string();
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
    fn commit_param_sketch_edits(&mut self) -> Result<(), String> {
        let sub_editor = match &mut self.sub_editor {
            Some(SubEditorState::Sketch { local_sketch, node_id, .. }) => {
                (*node_id, local_sketch.clone())
            }
            _ => return Err("Not in sketch editing mode".to_string()),
        };

        let (sketch_node_id, edited_sketch) = sub_editor;

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
    fn cancel_param_sketch_edits(&mut self) {
        self.sub_editor = None;
        self.rebuild_pending_since = None;
        self.status_message = "Sketch editing cancelled".to_string();
    }

    /// Request a deferred rebuild with debouncing.
    ///
    /// Sets rebuild_pending_since to now(). The rebuild is executed only when
    /// perform_param_rebuild_if_due() detects that >= REBUILD_DEBOUNCE_MS have elapsed.
    /// This prevents thrashing with frequent edits and allows batching multiple changes
    /// into a single rebuild cycle.
    #[allow(dead_code)]
    fn request_param_rebuild_soon(&mut self) {
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
    fn perform_param_rebuild_if_due(&mut self) {
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
}

impl eframe::App for MyCadApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let in_sketch = self.is_sketch_mode();
        let in_param_sketch = self.is_param_sketch_mode();

        // Deferred rebuild check
        self.perform_param_rebuild_if_due();

        // Global keybindings
        ctx.input(|i| {
            // Legacy sketch mode: S
            if i.key_pressed(egui::Key::S) && !i.modifiers.ctrl && !i.modifiers.shift && !in_sketch && !in_param_sketch {
                self.enter_sketch_mode();
            }
            // Parametric sketch mode: Shift+P
            if i.key_pressed(egui::Key::P) && i.modifiers.shift && !in_sketch && !in_param_sketch {
                self.start_param_sketch();
            }

            if in_sketch {
                if i.key_pressed(egui::Key::Escape) {
                    // Escape is also handled in viewport for line cancel,
                    // but if no line is active, exit sketch mode
                    if let Some(session) = &self.sketch_session {
                        if session.line_start.is_none() && session.tool == SketchTool::None {
                            self.exit_sketch_mode();
                        }
                    }
                }
                if i.key_pressed(egui::Key::L) && !i.modifiers.ctrl {
                    if let Some(session) = &mut self.sketch_session {
                        session.tool = SketchTool::Line;
                        session.line_start = None;
                        session.rect_start = None;
                        session.circle_center = None;
                        session.arc_center = None;
                        session.arc_start = None;
                    }
                }
                if i.key_pressed(egui::Key::R) && !i.modifiers.ctrl {
                    if let Some(session) = &mut self.sketch_session {
                        session.tool = SketchTool::Rectangle;
                        session.line_start = None;
                        session.rect_start = None;
                        session.circle_center = None;
                        session.arc_center = None;
                        session.arc_start = None;
                    }
                }
                if i.key_pressed(egui::Key::C) && !i.modifiers.ctrl {
                    if let Some(session) = &mut self.sketch_session {
                        session.tool = SketchTool::Circle;
                        session.line_start = None;
                        session.rect_start = None;
                        session.circle_center = None;
                        session.arc_center = None;
                        session.arc_start = None;
                    }
                }
                if i.key_pressed(egui::Key::A) && !i.modifiers.ctrl {
                    if let Some(session) = &mut self.sketch_session {
                        session.tool = SketchTool::Arc;
                        session.line_start = None;
                        session.rect_start = None;
                        session.circle_center = None;
                        session.arc_center = None;
                        session.arc_start = None;
                    }
                }
                // E = Extrude current sketch
                if i.key_pressed(egui::Key::E) && !i.modifiers.ctrl {
                    self.perform_extrude(self.extrude_depth);
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
        let in_sketch = self.is_sketch_mode();

        // Menu bar
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("New").clicked() {
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
                        // For now, use a default filename
                        match self.load_param_document("mycad_document.json") {
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
                        // For now, use a default filename
                        if let Err(e) = self.save_param_document("mycad_document.json") {
                            self.status_message = e;
                        }
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Export STL").clicked() {
                        ui.close_menu();
                    }
                });
                ui.menu_button("Edit", |ui| {
                    if ui.button("Undo").clicked() {
                        ui.close_menu();
                    }
                    if ui.button("Redo").clicked() {
                        ui.close_menu();
                    }
                });
                ui.menu_button("Sketch", |ui| {
                    if !in_sketch && !in_param_sketch {
                        if ui.button("New Sketch (XY)    S").clicked() {
                            self.enter_sketch_mode();
                            ui.close_menu();
                        }
                        ui.separator();
                        if ui.button("New Parametric Sketch (Exp)  Shift+P").clicked() {
                            self.start_param_sketch();
                            ui.close_menu();
                        }
                    } else if in_sketch {
                        if ui.button("Line Tool              L").clicked() {
                            if let Some(session) = &mut self.sketch_session {
                                session.tool = SketchTool::Line;
                                session.line_start = None;
                                session.rect_start = None;
                            }
                            ui.close_menu();
                        }
                        if ui.button("Rectangle Tool    R").clicked() {
                            if let Some(session) = &mut self.sketch_session {
                                session.tool = SketchTool::Rectangle;
                                session.line_start = None;
                                session.rect_start = None;
                                session.circle_center = None;
                                session.arc_center = None;
                                session.arc_start = None;
                            }
                            ui.close_menu();
                        }
                        if ui.button("Circle Tool        C").clicked() {
                            if let Some(session) = &mut self.sketch_session {
                                session.tool = SketchTool::Circle;
                                session.line_start = None;
                                session.rect_start = None;
                                session.circle_center = None;
                                session.arc_center = None;
                                session.arc_start = None;
                            }
                            ui.close_menu();
                        }
                        if ui.button("Arc Tool            A").clicked() {
                            if let Some(session) = &mut self.sketch_session {
                                session.tool = SketchTool::Arc;
                                session.line_start = None;
                                session.rect_start = None;
                                session.circle_center = None;
                                session.arc_center = None;
                                session.arc_start = None;
                            }
                            ui.close_menu();
                        }
                        ui.separator();
                        ui.horizontal(|ui| {
                            ui.label("Depth: ");
                            ui.add(egui::DragValue::new(&mut self.extrude_depth).speed(0.5).range(0.1..=100.0));
                        });
                        if ui.button("Extrude              E").clicked() {
                            self.perform_extrude(self.extrude_depth);
                            ui.close_menu();
                        }
                        ui.separator();
                        if ui.button("Exit Sketch        Esc").clicked() {
                            self.exit_sketch_mode();
                            ui.close_menu();
                        }
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
                if in_sketch {
                    ui.label(egui::RichText::new("SKETCH MODE").color(egui::Color32::from_rgb(50, 200, 255)));
                    if let Some(session) = &self.sketch_session {
                        ui.separator();
                        let tool_label = match session.tool {
                            SketchTool::None => "Select",
                            SketchTool::Point => "Point",
                            SketchTool::Line => "Line",
                            SketchTool::Rectangle => "Rectangle",
                            SketchTool::Circle => "Circle",
                            SketchTool::Arc => "Arc",
                        };
                        ui.label(format!("Tool: {}", tool_label));
                        if let Some(snap) = session.snapped_point {
                            ui.separator();
                            ui.label(format!("{:.2}, {:.2}", snap.x, snap.y));
                        }
                        ui.separator();
                        let entity_count = session.sketch.entities.len();
                        ui.label(format!("{} entities", entity_count));
                        ui.separator();
                        ui.label(egui::RichText::new("Press E to extrude").color(egui::Color32::from_rgb(100, 255, 100)));
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

        // Feature tree (left panel)
        egui::SidePanel::left("feature_tree")
            .default_width(200.0)
            .show(ctx, |ui| {
                // Show parametric feature tree if document has features
                let has_features = self.document.nodes.len() > 1; // More than just root
                if has_features {
                    mycad_ui::panels::parametric_feature_tree_panel(ui, &self.document);
                } else if let Some(session) = &self.sketch_session {
                    ui.heading("Features");
                    ui.separator();
                    ui.label(format!("Sketch ({} entities)", session.sketch.entities.len()));
                } else {
                    ui.heading("Features");
                    ui.separator();
                    ui.label("(empty)");
                }
            });

        // Property panel (right panel)
        egui::SidePanel::right("property_panel")
            .default_width(250.0)
            .show(ctx, |ui| {
                ui.heading("Properties");
                ui.separator();

                // Show parametric sketch entity properties
                if in_param_sketch {
                    if let Some(SubEditorState::Sketch {
                        local_sketch,
                        selected_entity,
                        ..
                    }) = &self.sub_editor {
                        // Show sketch statistics at top
                        ui.label(egui::RichText::new("Sketch Statistics").strong());
                        ui.label(format!("Entities: {}", local_sketch.entities.len()));
                        ui.label(format!("Constraints: {}", local_sketch.constraints().count()));
                        ui.separator();

                        if let Some(entity_id) = selected_entity {
                            if let Some(entity) = local_sketch.entity(*entity_id) {
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
                                        ui.label(format!("Start: ({:.2}, {:.2})", line.start.x, line.start.y));
                                        ui.label(format!("End: ({:.2}, {:.2})", line.end.x, line.end.y));
                                        ui.label(format!("Length: {:.2}", line.length()));
                                    }
                                    SketchGeometry::Point(pt) => {
                                        ui.label(format!("Position: ({:.2}, {:.2})", pt.position.x, pt.position.y));
                                    }
                                    SketchGeometry::Circle(circle) => {
                                        ui.label(format!("Center: ({:.2}, {:.2})", circle.center.x, circle.center.y));
                                        ui.label(format!("Radius: {:.2}", circle.radius));
                                    }
                                    SketchGeometry::Arc(arc) => {
                                        ui.label(format!("Center: ({:.2}, {:.2})", arc.center.x, arc.center.y));
                                        ui.label(format!("Radius: {:.2}", arc.radius));
                                        ui.label(format!("Start angle: {:.2}°", arc.start_angle.to_degrees()));
                                        ui.label(format!("End angle: {:.2}°", arc.end_angle.to_degrees()));
                                    }
                                }
                            } else {
                                ui.label("Selected entity not found");
                            }
                        } else {
                            ui.label("(Click entity to select)");
                        }
                    } else {
                        ui.label("Not in sketch mode");
                    }
                } else {
                    ui.label("No selection");
                }
            });

        // Central viewport
        let viewport_response = egui::CentralPanel::default()
            .show(ctx, |ui| {
                egui::Frame::canvas(ui.style()).show(ui, |ui| {
                    if let Some(viewport) = self.viewport.as_mut() {
                        Some(viewport.ui(ui, in_sketch))
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
                self.handle_sketch_input(&response);
            }
        }
        self.update_sketch_rendering();
    }
}

fn entity_type_name(geometry: &SketchGeometry) -> &'static str {
    match geometry {
        SketchGeometry::Point(_) => "Point",
        SketchGeometry::LineSegment(_) => "Line",
        SketchGeometry::Circle(_) => "Circle",
        SketchGeometry::Arc(_) => "Arc",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_app() -> MyCadApp {
        MyCadApp {
            viewport: None,
            sketch_session: None,
            status_message: "Test app".to_string(),
            extrude_depth: 5.0,
            document: ParamDocument::new(),
            sub_editor: None,
            rebuild_pending_since: None,
        }
    }

    #[test]
    fn parametric_sketch_workflow() {
        let mut app = create_test_app();

        // Start a parametric sketch
        app.start_param_sketch();
        assert!(app.sub_editor.is_some(), "Should have created sub_editor");
        assert!(app.document.nodes.len() > 1, "Should have created datum + sketch nodes");

        // Get node_id before modifying
        let sketch_node_id = if let Some(SubEditorState::Sketch { node_id, .. }) = &app.sub_editor {
            *node_id
        } else {
            panic!("Expected sketch editing state");
        };

        // Modify the sketch
        if let Some(SubEditorState::Sketch { local_sketch, .. }) = &mut app.sub_editor {
            // Add a rectangle to the sketch
            local_sketch.add_rectangle(Point2::new(0.0, 0.0), Point2::new(10.0, 5.0));
            assert_eq!(
                local_sketch.entities.len(),
                4,
                "Rectangle should have 4 line segments"
            );
        }

        // Commit the changes
        let commit_result = app.commit_param_sketch_edits();
        assert!(commit_result.is_ok(), "Commit should succeed");

        // Verify the sketch was updated in the document
        let node = app.document.node(sketch_node_id);
        assert!(node.is_ok(), "Node should exist after commit");
        if let Ok(n) = node {
            if let Operation::CreateSketch(sketch_op) = &n.operation {
                assert_eq!(
                    sketch_op.sketch.entities.len(),
                    4,
                    "Sketch in document should be updated"
                );
            }
        }

        // Request rebuild
        app.request_param_rebuild_soon();
        assert!(
            app.rebuild_pending_since.is_some(),
            "Should have rebuild scheduled"
        );

        // Wait a bit for debounce to expire
        std::thread::sleep(std::time::Duration::from_millis(150));

        // Perform rebuild
        app.perform_param_rebuild_if_due();
        assert!(
            app.rebuild_pending_since.is_none(),
            "Should have cleared rebuild flag after executing"
        );
    }

    #[test]
    fn cancel_sketch_edits() {
        let mut app = create_test_app();

        // Start a parametric sketch
        app.start_param_sketch();
        assert!(app.sub_editor.is_some());

        // Modify the sketch
        if let Some(SubEditorState::Sketch { local_sketch, .. }) = &mut app.sub_editor {
            local_sketch.add_rectangle(Point2::new(0.0, 0.0), Point2::new(10.0, 5.0));
        }

        // Cancel without committing
        app.cancel_param_sketch_edits();
        assert!(app.sub_editor.is_none(), "Should have cleared sub_editor");
        assert!(
            app.rebuild_pending_since.is_none(),
            "Should have cleared rebuild pending"
        );
    }
}
