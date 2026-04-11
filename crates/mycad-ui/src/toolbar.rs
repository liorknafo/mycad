//! Toolbar: mode buttons, sketch tools, constraint tools, view tools.

use egui;

/// Available sketch tools for drawing and editing
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SketchTool {
    Select,
    Line,
    Rectangle,
    Circle,
    Arc,
}

/// Available constraint tools for applying geometric relationships
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConstraintTool {
    #[default]
    None,
    Coincident,
    Horizontal,
    Vertical,
    Distance,
}

/// State for constraint creation (accumulating selections)
#[derive(Debug, Clone, Default)]
pub struct ConstraintBuilder {
    pub active_tool: ConstraintTool,
    pub first_selection: Option<crate::sketch_mode::SelectableItem>,
    pub prompt_message: String,
}

impl ConstraintBuilder {
    pub fn new() -> Self {
        Self {
            active_tool: ConstraintTool::None,
            first_selection: None,
            prompt_message: String::new(),
        }
    }

    pub fn start_tool(&mut self, tool: ConstraintTool) {
        self.active_tool = tool;
        self.first_selection = None;
        self.prompt_message = match tool {
            ConstraintTool::Coincident => "Select first point".to_string(),
            ConstraintTool::Horizontal => "Select a line".to_string(),
            ConstraintTool::Vertical => "Select a line".to_string(),
            ConstraintTool::Distance => "Select first point".to_string(),
            ConstraintTool::None => String::new(),
        };
    }

    pub fn cancel(&mut self) {
        self.active_tool = ConstraintTool::None;
        self.first_selection = None;
        self.prompt_message.clear();
    }

    pub fn is_active(&self) -> bool {
        self.active_tool != ConstraintTool::None
    }
}

/// Response from toolbar interaction
#[derive(Debug, Clone)]
pub struct ToolbarResponse {
    pub selected_sketch_tool: Option<SketchTool>,
    pub constraint_tool_changed: bool,
}

/// Show the sketch toolbar with drawing and constraint tools
pub fn sketch_toolbar(
    ui: &mut egui::Ui,
    current_sketch_tool: SketchTool,
    constraint_builder: &mut ConstraintBuilder,
) -> ToolbarResponse {
    let mut response = ToolbarResponse {
        selected_sketch_tool: None,
        constraint_tool_changed: false,
    };

    ui.horizontal(|ui| {
        ui.label("Draw:");
        
        let tools = [
            (SketchTool::Select, "Select", "S"),
            (SketchTool::Line, "Line", "L"),
            (SketchTool::Rectangle, "Rect", "R"),
            (SketchTool::Circle, "Circle", "C"),
            (SketchTool::Arc, "Arc", "A"),
        ];

        for (tool, label, _shortcut) in tools {
            let selected = current_sketch_tool == tool;
            if ui.selectable_label(selected, label).clicked() {
                response.selected_sketch_tool = Some(tool);
            }
        }

        ui.separator();
        ui.label("Constraints:");

        let constraint_selected = constraint_builder.is_active();
        
        let c_tools = [
            (ConstraintTool::Coincident, "Coincident"),
            (ConstraintTool::Horizontal, "Horizontal"),
            (ConstraintTool::Vertical, "Vertical"),
            (ConstraintTool::Distance, "Distance"),
        ];

        for (tool, label) in c_tools {
            let selected = constraint_builder.active_tool == tool;
            if ui.selectable_label(selected, label).clicked() {
                constraint_builder.start_tool(tool);
                response.constraint_tool_changed = true;
            }
        }

        if constraint_selected && ui.button("Cancel").clicked() {
            constraint_builder.cancel();
            response.constraint_tool_changed = true;
        }

        if !constraint_builder.prompt_message.is_empty() {
            ui.separator();
            ui.label(egui::RichText::new(&constraint_builder.prompt_message).italics());
        }
    });

    response
}

/// Show constraint prompt message in status area
pub fn constraint_status_message(constraint_builder: &ConstraintBuilder) -> Option<&str> {
    if constraint_builder.prompt_message.is_empty() {
        None
    } else {
        Some(&constraint_builder.prompt_message)
    }
}
