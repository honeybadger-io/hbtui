use serde::Deserialize;
use std::collections::HashMap;

// ============================================
// Dashboard YAML Parsing Structures
// ============================================

#[derive(Debug, Clone, Deserialize)]
pub struct Dashboard {
    pub title: String,
    pub widgets: Vec<Widget>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Widget {
    pub id: String,
    #[serde(rename = "type")]
    pub widget_type: String,
    pub grid: GridPosition,
    pub presentation: WidgetPresentation,
    #[serde(default)]
    pub config: WidgetConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GridPosition {
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct WidgetPresentation {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub subtitle: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct WidgetConfig {
    #[serde(default)]
    pub query: String,
    #[serde(default)]
    pub vis: VisualizationConfig,
    #[serde(default)]
    pub streams: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct VisualizationConfig {
    #[serde(default)]
    pub view: String, // "line", "table", "histogram"
    #[serde(default)]
    pub chart_config: ChartConfig,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ChartConfig {
    #[serde(default)]
    pub groups: HashMap<String, GroupConfig>,
    #[serde(rename = "xField")]
    pub x_field: Option<String>,
    #[serde(rename = "yField")]
    pub y_field: Option<String>,
    #[serde(rename = "zField")]
    pub z_field: Option<String>,
    #[serde(rename = "groupType")]
    pub group_type: Option<String>,
    #[serde(rename = "xFieldUnit")]
    pub x_field_unit: Option<String>,
    #[serde(rename = "yFieldUnit")]
    pub y_field_unit: Option<String>,
    // Billboard specific fields
    #[serde(rename = "titleField")]
    pub title_field: Option<String>,
    #[serde(rename = "valueField")]
    pub value_field: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct GroupConfig {
    pub color: Option<String>,
}

// ============================================
// Insights API Response Structures
// ============================================

#[derive(Debug, Clone, Deserialize)]
pub struct InsightsResponse {
    pub results: Vec<HashMap<String, serde_json::Value>>,
    #[serde(default, alias = "metadata")]
    pub meta: InsightsMetadata,
    #[serde(default)]
    pub error: Option<InsightsError>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct InsightsError {
    pub message: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct InsightsMetadata {
    #[serde(default)]
    pub fields: Vec<String>,
    #[serde(default)]
    pub schema: Vec<SchemaField>,
    #[serde(default)]
    pub rows: RowInfo,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SchemaField {
    pub name: String,
    #[serde(rename = "type")]
    pub field_type: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RowInfo {
    #[serde(default)]
    pub returned: u64,
    #[serde(default)]
    pub total: u64,
}

// ============================================
// Widget Runtime State
// ============================================

#[derive(Debug, Clone)]
pub enum WidgetState {
    Loading,
    Loaded(InsightsResponse),
    Error(String),
}

#[derive(Debug, Clone)]
pub struct WidgetRuntime {
    pub widget: Widget,
    pub state: WidgetState,
}

// ============================================
// Dashboard Runtime State
// ============================================

#[derive(Debug, Clone)]
pub struct DashboardState {
    pub dashboard: Dashboard,
    pub widgets: Vec<WidgetRuntime>,
    pub project_id: u64,
}

impl DashboardState {
    pub fn new(dashboard: Dashboard, project_id: u64) -> Self {
        let widgets = dashboard
            .widgets
            .iter()
            .map(|w| WidgetRuntime {
                widget: w.clone(),
                state: WidgetState::Loading,
            })
            .collect();

        Self {
            dashboard,
            widgets,
            project_id,
        }
    }

    pub fn update_widget(&mut self, widget_id: &str, state: WidgetState) {
        if let Some(widget) = self.widgets.iter_mut().find(|w| w.widget.id == widget_id) {
            widget.state = state;
        }
    }

    pub fn reset_all_to_loading(&mut self) {
        for widget in &mut self.widgets {
            widget.state = WidgetState::Loading;
        }
    }
}
