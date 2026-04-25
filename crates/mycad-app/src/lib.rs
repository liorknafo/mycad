use eframe::egui;
use mycad_kernel::math::{Point2, Scalar};
use mycad_kernel::sketch::{Sketch, SketchEntityId};
use mycad_kernel::parametric::types::{Document as ParamDocument, NodeId};
use mycad_kernel::parametric::ops::extrude_op::ExtrudeOp;
use mycad_renderer::Viewport3d;

mod document_ops;
mod file_io;
mod sub_editor;
mod ui_chrome;

pub(crate) const SNAP_GRID_SIZE: Scalar = 1.0;
pub(crate) const SNAP_POINT_THRESHOLD: Scalar = 0.3;

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
        solver_result: Option<Box<mycad_kernel::sketch::SketchSolveResult>>,
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


#[allow(dead_code)]
pub struct MyCadApp {
    pub(crate) viewport: Option<Viewport3d>,
    pub(crate) status_message: String,
    pub(crate) extrude_depth: Scalar,
    pub(crate) document: ParamDocument,
    pub(crate) sub_editor: Option<SubEditorState>,
    pub(crate) rebuild_pending_since: Option<std::time::Instant>,
    /// Document-level undo stack (snapshots taken before each mutating op).
    pub(crate) doc_undo: Vec<ParamDocument>,
    pub(crate) doc_redo: Vec<ParamDocument>,
    /// Right-side history panel state.
    pub(crate) history_state: mycad_ui::history_panel::HistoryPanelState,
}

pub(crate) const DOC_UNDO_LIMIT: usize = 64;

impl MyCadApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        Self {
            viewport: Viewport3d::new(cc),
            status_message: "Press Shift+P to start a parametric sketch".to_string(),
            extrude_depth: 5.0,
            document: ParamDocument::new(),
            sub_editor: None,
            rebuild_pending_since: None,
            doc_undo: Vec::new(),
            doc_redo: Vec::new(),
            history_state: mycad_ui::history_panel::HistoryPanelState::default(),
        }
    }
}

impl eframe::App for MyCadApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.render(ctx, frame);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mycad_kernel::parametric::feature::{Operation, WorldRef};

    fn create_test_app() -> MyCadApp {
        MyCadApp {
            viewport: None,
            status_message: "Test app".to_string(),
            extrude_depth: 5.0,
            document: ParamDocument::new(),
            sub_editor: None,
            rebuild_pending_since: None,
            doc_undo: Vec::new(),
            doc_redo: Vec::new(),
            history_state: mycad_ui::history_panel::HistoryPanelState::default(),
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
    fn doc_undo_restores_prior_document() {
        let mut app = create_test_app();
        let initial_node_count = app.document.nodes.len();

        // Snapshot then mutate
        app.snapshot_doc();
        app.document
            .append_op(Operation::CreateDatumPlane(
                mycad_kernel::parametric::ops::datum_plane::CreateDatumPlaneOp::world(
                    WorldRef::PlaneXY,
                    "XY",
                ),
            ))
            .unwrap();
        assert_eq!(app.document.nodes.len(), initial_node_count + 1);
        assert_eq!(app.doc_undo.len(), 1);

        app.undo_doc();
        assert_eq!(app.document.nodes.len(), initial_node_count);
        assert_eq!(app.doc_undo.len(), 0);
        assert_eq!(app.doc_redo.len(), 1);

        app.redo_doc();
        assert_eq!(app.document.nodes.len(), initial_node_count + 1);
        assert_eq!(app.doc_undo.len(), 1);
        assert_eq!(app.doc_redo.len(), 0);
    }

    #[test]
    fn doc_undo_limit_caps_stack() {
        let mut app = create_test_app();
        for _ in 0..(DOC_UNDO_LIMIT + 10) {
            app.snapshot_doc();
        }
        assert_eq!(app.doc_undo.len(), DOC_UNDO_LIMIT);
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
