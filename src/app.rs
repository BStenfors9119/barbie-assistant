use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;

use iced::alignment::Vertical;
use iced::theme::palette::Palette;
use iced::widget::{
    button, checkbox, column, container, horizontal_space, mouse_area, pick_list, row, scrollable,
    text, text_editor, text_input,
};
use iced::{Border, Color, Element, Length, Task, Theme};

use crate::builder_state::{BuilderState, WhereOperator};
use crate::commands::{generate_query, QueryParams};
use crate::settings::{Settings, ThemeColor, ThemeMode};
use crate::stm_schema;
use crate::templates::QueryTemplate;
use crate::user_templates::UserTemplate;

const FONT_MIN: u16 = 10;
const FONT_MAX: u16 = 28;

// ── Query customiser ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Default)]
enum JoinType {
    #[default]
    Inner,
    Left,
}

impl JoinType {
    fn all() -> Vec<Self> {
        vec![JoinType::Inner, JoinType::Left]
    }
}

impl fmt::Display for JoinType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JoinType::Inner => write!(f, "INNER JOIN"),
            JoinType::Left  => write!(f, "LEFT JOIN"),
        }
    }
}

#[derive(Debug, Clone, Default)]
struct JoinDef {
    join_type: JoinType,
    table: String,
    on_condition: String,
}

#[derive(Debug, Default)]
struct QueryCustomiser {
    base_table: String,
    /// Ordered list of all known columns (from schema or template).
    available_columns: Vec<String>,
    /// Subset currently included in SELECT.
    selected_fields: Vec<String>,
    /// Text input buffer for adding an ad-hoc field.
    new_field_input: String,
    /// Preserved WHERE clause (still contains `{param}` placeholders).
    where_clause: String,
    order_clause: String,
    joins: Vec<JoinDef>,
}

impl QueryCustomiser {
    fn from_template(tmpl: &QueryTemplate) -> Self {
        let p = parse_select_sql(&tmpl.sql);

        // Columns from schema (if table is known).
        let mut available: Vec<String> = stm_schema::find_table(&p.from_table)
            .map(|t| t.columns.iter().map(|c| c.name.to_string()).collect())
            .unwrap_or_default();

        // Append any template-specific fields not already in the schema list.
        for f in &p.fields {
            if !available.iter().any(|a| a.eq_ignore_ascii_case(f)) {
                available.push(f.clone());
            }
        }

        // SELECT * → all available selected; specific fields → only those.
        let selected = if p.select_star {
            available.clone()
        } else {
            p.fields.iter()
                .map(|f| available.iter().find(|a| a.eq_ignore_ascii_case(f)).cloned().unwrap_or_else(|| f.clone()))
                .collect()
        };

        QueryCustomiser {
            base_table: p.from_table,
            available_columns: available,
            selected_fields: selected,
            new_field_input: String::new(),
            where_clause: p.where_clause,
            order_clause: p.order_clause,
            joins: vec![],
        }
    }

    fn build_sql(&self) -> String {
        let select = if self.selected_fields.is_empty() {
            "*".to_string()
        } else {
            self.selected_fields.join(", ")
        };
        let mut sql = format!("SELECT {}\nFROM {}", select, self.base_table);
        for j in &self.joins {
            if !j.table.trim().is_empty() {
                if j.on_condition.trim().is_empty() {
                    sql += &format!("\n{} {}", j.join_type, j.table.trim());
                } else {
                    sql += &format!("\n{} {} ON {}", j.join_type, j.table.trim(), j.on_condition.trim());
                }
            }
        }
        if !self.where_clause.is_empty() {
            sql += &format!("\nWHERE {}", self.where_clause);
        }
        if !self.order_clause.is_empty() {
            sql += &format!("\nORDER BY {}", self.order_clause);
        }
        sql
    }
}

struct SelectParts {
    select_star: bool,
    fields: Vec<String>,
    from_table: String,
    where_clause: String,
    order_clause: String,
}

fn parse_select_sql(sql: &str) -> SelectParts {
    // Normalise whitespace so every keyword is separated by a single space.
    let norm: String = sql.split_whitespace().collect::<Vec<_>>().join(" ");
    let up = norm.to_ascii_uppercase();

    // SELECT body (between "SELECT " and " FROM ").
    let from_at = up.find(" FROM ").unwrap_or(up.len());
    let fields_str = norm.get(7..from_at).map(|s| s.trim()).unwrap_or("*");
    let (select_star, fields) = if fields_str == "*" {
        (true, vec![])
    } else {
        let v = fields_str.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
        (false, v)
    };

    // Everything after FROM.
    let rest = &norm[(from_at + 6).min(norm.len())..];
    let up_rest = rest.to_ascii_uppercase();

    // Table name ends at the first clause keyword.
    let table_end = [" WHERE ", " ORDER ", " INNER ", " LEFT ", " RIGHT ", " JOIN "]
        .iter().filter_map(|kw| up_rest.find(kw)).min().unwrap_or(rest.len());
    let from_table = rest[..table_end].trim().to_string();

    let where_clause = up_rest.find(" WHERE ")
        .map(|i| {
            let s = &rest[i + 7..];
            let e = s.to_ascii_uppercase().find(" ORDER ").unwrap_or(s.len());
            s[..e].trim().to_string()
        })
        .unwrap_or_default();

    let order_clause = up_rest.find(" ORDER BY ")
        .map(|i| rest[i + 10..].trim().to_string())
        .unwrap_or_default();

    SelectParts { select_star, fields, from_table, where_clause, order_clause }
}

// ── View state ────────────────────────────────────────────────────────────────

#[derive(Debug, Default, PartialEq)]
enum AppView {
    #[default]
    Main,
    Builder,
}

#[derive(Debug, Default, PartialEq, Clone)]
enum MainTab {
    #[default]
    Templates,
    FromScratch,
}

// ── Join graph ────────────────────────────────────────────────────────────────

struct JoinEdge {
    a: &'static str,
    b: &'static str,
    on_cond: &'static str,
}

static JOIN_EDGES: &[JoinEdge] = &[
    JoinEdge { a: "EXPENSE_REPORT", b: "EXPENSE_ENTRY",
        on_cond: "EXPENSE_ENTRY.REPORT_ID = EXPENSE_REPORT.REPORT_ID" },
    JoinEdge { a: "EXPENSE_ENTRY",  b: "EXPENSE_ALLOCATION",
        on_cond: "EXPENSE_ALLOCATION.REPORT_ID = EXPENSE_ENTRY.REPORT_ID\n    AND EXPENSE_ALLOCATION.ENTRY_ID = EXPENSE_ENTRY.ENTRY_ID" },
    JoinEdge { a: "EXPENSE_ENTRY",  b: "EXPENSE_ATTENDEE",
        on_cond: "EXPENSE_ATTENDEE.REPORT_ID = EXPENSE_ENTRY.REPORT_ID\n    AND EXPENSE_ATTENDEE.ENTRY_ID = EXPENSE_ENTRY.ENTRY_ID" },
    JoinEdge { a: "EXPENSE_REPORT", b: "PAYMENT_BATCH",
        on_cond: "PAYMENT_BATCH.REPORT_ID = EXPENSE_REPORT.REPORT_ID" },
    JoinEdge { a: "EXPENSE_REPORT", b: "EMPLOYEE",
        on_cond: "EMPLOYEE.LOGIN_ID = EXPENSE_REPORT.LOGIN_ID" },
    JoinEdge { a: "TRAVEL_REQUEST", b: "TRAVEL_REQUEST_SEGMENT",
        on_cond: "TRAVEL_REQUEST_SEGMENT.REQUEST_ID = TRAVEL_REQUEST.REQUEST_ID" },
    JoinEdge { a: "TRAVEL_REQUEST", b: "EMPLOYEE",
        on_cond: "EMPLOYEE.LOGIN_ID = TRAVEL_REQUEST.LOGIN_ID" },
    // Expense detail
    JoinEdge { a: "EXPENSE_ENTRY", b: "EXPENSE_ITEMIZATION",
        on_cond: "EXPENSE_ITEMIZATION.REPORT_ID = EXPENSE_ENTRY.REPORT_ID\n    AND EXPENSE_ITEMIZATION.ENTRY_ID = EXPENSE_ENTRY.ENTRY_ID" },
    JoinEdge { a: "EXPENSE_ENTRY", b: "CORPORATE_CARD_TRANSACTION",
        on_cond: "CORPORATE_CARD_TRANSACTION.TRANSACTION_ID = EXPENSE_ENTRY.CARD_TRANSACTION_ID" },
    JoinEdge { a: "EXPENSE_REPORT", b: "CASH_ADVANCE",
        on_cond: "CASH_ADVANCE.REPORT_ID = EXPENSE_REPORT.REPORT_ID" },
    // Travel bookings
    JoinEdge { a: "TRIP", b: "EMPLOYEE",
        on_cond: "EMPLOYEE.LOGIN_ID = TRIP.LOGIN_ID" },
    JoinEdge { a: "TRIP", b: "TRAVEL_REQUEST",
        on_cond: "TRIP.REQUEST_ID = TRAVEL_REQUEST.REQUEST_ID" },
    JoinEdge { a: "TRIP", b: "EXPENSE_REPORT",
        on_cond: "EXPENSE_REPORT.TRIP_ID = TRIP.TRIP_ID" },
    JoinEdge { a: "TRIP", b: "TRIP_SEGMENT_AIR",
        on_cond: "TRIP_SEGMENT_AIR.TRIP_ID = TRIP.TRIP_ID" },
    JoinEdge { a: "TRIP", b: "TRIP_SEGMENT_HOTEL",
        on_cond: "TRIP_SEGMENT_HOTEL.TRIP_ID = TRIP.TRIP_ID" },
    JoinEdge { a: "TRIP", b: "TRIP_SEGMENT_CAR",
        on_cond: "TRIP_SEGMENT_CAR.TRIP_ID = TRIP.TRIP_ID" },
    // Invoice
    JoinEdge { a: "INVOICE", b: "EMPLOYEE",
        on_cond: "EMPLOYEE.LOGIN_ID = INVOICE.LOGIN_ID" },
    JoinEdge { a: "INVOICE", b: "INVOICE_LINE",
        on_cond: "INVOICE_LINE.INVOICE_ID = INVOICE.INVOICE_ID" },
    JoinEdge { a: "INVOICE_LINE", b: "INVOICE_ALLOCATION",
        on_cond: "INVOICE_ALLOCATION.INVOICE_ID = INVOICE_LINE.INVOICE_ID\n    AND INVOICE_ALLOCATION.LINE_ID = INVOICE_LINE.LINE_ID" },
];

/// BFS from any node in `from_set` to `target`; returns the path of
/// (table_name, on_condition) steps needed to reach it.
fn bfs_path(from_set: &HashSet<String>, target: &str) -> Option<Vec<(String, String)>> {
    let mut visited: HashSet<String> = from_set.clone();
    let mut queue: VecDeque<(String, Vec<(String, String)>)> = VecDeque::new();
    for node in from_set {
        queue.push_back((node.clone(), vec![]));
    }
    while let Some((current, path)) = queue.pop_front() {
        for edge in JOIN_EDGES {
            let (next, on_cond) = if edge.a == current {
                (edge.b, edge.on_cond)
            } else if edge.b == current {
                (edge.a, edge.on_cond)
            } else {
                continue;
            };
            if visited.contains(next) { continue; }
            visited.insert(next.to_string());
            let mut new_path = path.clone();
            new_path.push((next.to_string(), on_cond.to_string()));
            if next == target { return Some(new_path); }
            queue.push_back((next.to_string(), new_path));
        }
    }
    None
}

/// Given user-selected tables (ordered), compute the ordered JOIN plan and
/// which tables were inserted automatically as bridges.
///
/// Returns `(joins, auto_added)` where `joins` is a list of
/// `(table_name, on_condition)` — the first entry's table is the **base** FROM
/// table (on_condition is empty), the rest are LEFT JOIN targets.
fn plan_joins(selected: &[String]) -> (Vec<(String, String)>, Vec<String>) {
    if selected.is_empty() { return (vec![], vec![]); }

    let mut joins: Vec<(String, String)> = vec![(selected[0].clone(), String::new())];
    let mut auto_added: Vec<String> = Vec::new();
    let mut included: HashSet<String> = HashSet::from([selected[0].clone()]);
    let mut to_visit: Vec<String> = selected[1..].to_vec();

    while !to_visit.is_empty() {
        // Find the nearest reachable target.
        let mut best: Option<(usize, Vec<(String, String)>)> = None;
        for (idx, target) in to_visit.iter().enumerate() {
            if included.contains(target) {
                best = Some((idx, vec![]));
                break;
            }
            if let Some(path) = bfs_path(&included, target) {
                if best.as_ref().map_or(true, |(_, p)| path.len() < p.len()) {
                    best = Some((idx, path));
                }
            }
        }
        let Some((idx, path)) = best else { break };
        let target = to_visit.remove(idx);

        for (table, on_cond) in path {
            if !included.contains(&table) {
                let is_auto = !selected.contains(&table);
                included.insert(table.clone());
                joins.push((table.clone(), on_cond));
                if is_auto { auto_added.push(table.clone()); }
                to_visit.retain(|t| *t != table);
            }
        }
        if !included.contains(&target) {
            included.insert(target.clone());
            joins.push((target, String::new()));
        }
    }
    (joins, auto_added)
}

// ── App ───────────────────────────────────────────────────────────────────────

pub struct App {
    selected_template: Option<String>,
    parameters: HashMap<String, String>,
    sql_editor: text_editor::Content,
    customiser: Option<QueryCustomiser>,
    error: Option<String>,
    saved_queries: Vec<String>,
    /// Collapsed state per group name; missing key = expanded.
    group_expanded: HashMap<String, bool>,
    /// Templates loaded from `_templates/*.json`.
    file_templates: Vec<QueryTemplate>,
    font_size: u16,
    font_size_raw: String,
    theme_color: ThemeColor,
    theme_mode: ThemeMode,
    user_templates: Vec<UserTemplate>,
    view: AppView,
    builder: BuilderState,
    // ── From-Scratch tab ──────────────────────────────────────────────────────
    main_tab: MainTab,
    /// Tables the user explicitly checked.
    scratch_tables: Vec<String>,
    /// Qualified "TABLE.COLUMN" names selected by the user.
    scratch_columns: Vec<String>,
    /// Computed join plan: [(table, on_condition)]; first entry is base FROM.
    scratch_joins: Vec<(String, String)>,
    /// Tables that were auto-added as bridges.
    scratch_auto_added: Vec<String>,
    /// Live SQL output for the From Scratch editor.
    scratch_sql: text_editor::Content,
    /// Index of the column row currently being dragged.
    scratch_drag_idx: Option<usize>,
    /// Index of the column row the cursor is hovering over during a drag.
    scratch_hover_idx: Option<usize>,
    /// Set when opening the default email client fails.
    email_error: Option<String>,
    /// Table names found in pasted SQL that don't exist in the schema.
    scratch_unknown_tables: Vec<String>,
    /// Qualified column names (TABLE.COLUMN or bare) from pasted SQL not found in schema.
    scratch_unknown_columns: Vec<String>,
}

impl Default for App {
    fn default() -> Self {
        let s = Settings::load();
        Self {
            selected_template: None,
            parameters: HashMap::new(),
            sql_editor: text_editor::Content::new(),
            customiser: None,
            error: None,
            saved_queries: Vec::new(),
            group_expanded: HashMap::new(),
            file_templates: QueryTemplate::load_from_dir(),
            font_size_raw: s.font_size.to_string(),
            font_size: s.font_size,
            theme_color: s.theme_color,
            theme_mode: s.theme_mode,
            user_templates: UserTemplate::load_all(),
            view: AppView::default(),
            builder: BuilderState::default(),
            main_tab: MainTab::default(),
            scratch_tables: Vec::new(),
            scratch_columns: Vec::new(),
            scratch_joins: Vec::new(),
            scratch_auto_added: Vec::new(),
            scratch_sql: text_editor::Content::new(),
            scratch_drag_idx: None,
            scratch_hover_idx: None,
            email_error: None,
            scratch_unknown_tables: Vec::new(),
            scratch_unknown_columns: Vec::new(),
        }
    }
}

// ── Messages ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum Message {
    TemplateSelected(String),
    ParameterChanged(String, String),
    GenerateQuery,
    CopyToClipboard,
    SaveQuery,
    SqlEditorAction(text_editor::Action),
    OpenFile,
    FileOpened(Option<String>),
    ToggleGroup(String),
    // query customiser
    FieldToggled(String, bool),
    CustomFieldInput(String),
    AddCustomField,
    AddJoin,
    RemoveJoin(usize),
    JoinTypeChanged(usize, JoinType),
    JoinTableChanged(usize, String),
    JoinConditionChanged(usize, String),
    SetTheme(ThemeColor, ThemeMode),
    FontIncrease,
    FontDecrease,
    FontSizeInput(String),
    OpenBuilder,
    BuilderBack,
    BuilderNameChanged(String),
    BuilderTableSelected(String),
    BuilderColumnToggled(String, bool),
    BuilderConditionAdded,
    BuilderConditionRemoved(usize),
    BuilderConditionFieldChanged(usize, String),
    BuilderConditionOperatorChanged(usize, WhereOperator),
    BuilderConditionParamChanged(usize, String),
    BuilderSave,
    // ── From-Scratch tab ──────────────────────────────────────────────────────
    MainTabChanged(MainTab),
    ScratchTableToggled(String, bool),
    ScratchColumnToggled(String, bool),
    ScratchDragStart(usize),
    ScratchDragHover(usize),
    ScratchDragDrop(usize),
    ScratchDragCancel,
    ScratchCopyToClipboard,
    ScratchSqlEditorAction(text_editor::Action),
    EmailSql(String),
    EmailOpened,
    EmailFailed(String),
    ResetTemplates,
    ResetScratch,
    Noop,
}

// ── App impl ──────────────────────────────────────────────────────────────────

impl App {
    pub fn title(&self) -> String {
        "STM Query Builder".to_string()
    }

    pub fn theme(&self) -> Theme {
        build_theme(&self.theme_color, &self.theme_mode)
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::TemplateSelected(id) => {
                self.selected_template = Some(id.clone());
                self.parameters.clear();
                self.error = None;
                let all = self.all_templates();
                if let Some(tmpl) = all.iter().find(|t| t.id == id) {
                    let c = QueryCustomiser::from_template(tmpl);
                    self.sql_editor = text_editor::Content::with_text(&c.build_sql());
                    self.customiser = Some(c);
                } else {
                    self.customiser = None;
                    self.sql_editor = text_editor::Content::new();
                }
            }
            Message::ParameterChanged(key, value) => {
                self.parameters.insert(key, value);
            }
            Message::GenerateQuery => {
                // Use the customiser's SQL (with param substitution) if available.
                let base = if let Some(ref c) = self.customiser {
                    c.build_sql()
                } else if let Some(ref id) = self.selected_template.clone() {
                    let all = self.all_templates();
                    match generate_query(
                        QueryParams { template_id: id.clone(), parameters: self.parameters.clone() },
                        &all,
                    ) {
                        Ok(sql) => { self.sql_editor = text_editor::Content::with_text(&sql); self.error = None; }
                        Err(e) => self.error = Some(e),
                    }
                    return Task::none();
                } else {
                    return Task::none();
                };
                let mut final_sql = base;
                for (key, val) in &self.parameters {
                    final_sql = final_sql.replace(&format!("{{{}}}", key), val);
                }
                self.sql_editor = text_editor::Content::with_text(&final_sql);
                self.error = None;
            }
            Message::CopyToClipboard => {
                return iced::clipboard::write(self.sql_editor.text());
            }
            Message::SaveQuery => {
                let t = self.sql_editor.text();
                if !t.trim().is_empty() {
                    self.saved_queries.push(t);
                }
            }
            Message::SqlEditorAction(action) => {
                self.sql_editor.perform(action);
            }
            Message::OpenFile => {
                return Task::perform(
                    async {
                        tokio::task::spawn_blocking(|| -> Option<String> {
                            rfd::FileDialog::new()
                                .pick_file()
                                .and_then(|p| std::fs::read_to_string(&p).ok())
                        })
                        .await
                        .ok()
                        .flatten()
                    },
                    Message::FileOpened,
                );
            }
            Message::FileOpened(Some(content)) => {
                self.sql_editor = text_editor::Content::with_text(&content);
                self.selected_template = None;
                self.parameters.clear();
                self.error = None;
            }
            Message::FileOpened(None) => {}
            Message::ToggleGroup(name) => {
                let expanded = self.group_expanded.entry(name).or_insert(true);
                *expanded = !*expanded;
            }
            Message::SetTheme(color, mode) => {
                self.theme_color = color;
                self.theme_mode = mode;
                self.persist_settings();
            }
            Message::FontIncrease => {
                if self.font_size < FONT_MAX {
                    self.font_size += 1;
                    self.font_size_raw = self.font_size.to_string();
                    self.persist_settings();
                }
            }
            Message::FontDecrease => {
                if self.font_size > FONT_MIN {
                    self.font_size -= 1;
                    self.font_size_raw = self.font_size.to_string();
                    self.persist_settings();
                }
            }
            Message::FontSizeInput(raw) => {
                self.font_size_raw = raw.clone();
                if let Ok(n) = raw.trim().parse::<u16>() {
                    if (FONT_MIN..=FONT_MAX).contains(&n) {
                        self.font_size = n;
                        self.persist_settings();
                    }
                }
            }
            Message::OpenBuilder => {
                self.builder = BuilderState::default();
                self.view = AppView::Builder;
            }
            Message::BuilderBack => {
                self.view = AppView::Main;
                self.builder = BuilderState::default();
            }
            Message::BuilderNameChanged(name) => {
                self.builder.template_name = name;
                self.builder.save_error = None;
            }
            Message::BuilderTableSelected(table) => {
                self.builder.selected_table = Some(table);
                self.builder.selected_columns.clear();
                self.builder.conditions.clear();
                self.builder.save_error = None;
            }
            Message::BuilderColumnToggled(col, checked) => {
                if checked {
                    if !self.builder.selected_columns.contains(&col) {
                        self.builder.selected_columns.push(col);
                    }
                } else {
                    self.builder.selected_columns.retain(|c| c != &col);
                }
            }
            Message::BuilderConditionAdded => {
                self.builder.conditions.push(Default::default());
            }
            Message::BuilderConditionRemoved(i) => {
                if i < self.builder.conditions.len() {
                    self.builder.conditions.remove(i);
                }
            }
            Message::BuilderConditionFieldChanged(i, field) => {
                if let Some(c) = self.builder.conditions.get_mut(i) {
                    c.field = Some(field);
                }
            }
            Message::BuilderConditionOperatorChanged(i, op) => {
                if let Some(c) = self.builder.conditions.get_mut(i) {
                    c.operator = op;
                }
            }
            Message::BuilderConditionParamChanged(i, name) => {
                if let Some(c) = self.builder.conditions.get_mut(i) {
                    c.param_name = name;
                }
            }
            Message::BuilderSave => match self.builder.validate() {
                Err(e) => self.builder.save_error = Some(e),
                Ok(()) => {
                    let sql = self.builder.build_sql().unwrap_or_default();
                    let table = self.builder.selected_table.clone().unwrap_or_default();
                    self.user_templates.push(UserTemplate {
                        id: self.builder.build_template_id(),
                        name: self.builder.template_name.clone(),
                        description: format!("Custom query on {table}"),
                        sql,
                    });
                    UserTemplate::save_all(&self.user_templates);
                    self.view = AppView::Main;
                    self.builder = BuilderState::default();
                }
            },
            Message::FieldToggled(field, checked) => {
                if let Some(ref mut c) = self.customiser {
                    if checked {
                        if !c.selected_fields.contains(&field) {
                            c.selected_fields.push(field);
                        }
                    } else {
                        c.selected_fields.retain(|f| f != &field);
                    }
                }
                self.regenerate_sql();
            }
            Message::CustomFieldInput(val) => {
                if let Some(ref mut c) = self.customiser {
                    c.new_field_input = val;
                }
            }
            Message::AddCustomField => {
                if let Some(ref mut c) = self.customiser {
                    let field = c.new_field_input.trim().to_string();
                    if !field.is_empty() {
                        if !c.available_columns.contains(&field) {
                            c.available_columns.push(field.clone());
                        }
                        if !c.selected_fields.contains(&field) {
                            c.selected_fields.push(field);
                        }
                        c.new_field_input.clear();
                    }
                }
                self.regenerate_sql();
            }
            Message::AddJoin => {
                if let Some(ref mut c) = self.customiser {
                    c.joins.push(JoinDef::default());
                }
            }
            Message::RemoveJoin(i) => {
                if let Some(ref mut c) = self.customiser {
                    if i < c.joins.len() {
                        c.joins.remove(i);
                    }
                }
                self.regenerate_sql();
            }
            Message::JoinTypeChanged(i, jt) => {
                if let Some(ref mut c) = self.customiser {
                    if let Some(j) = c.joins.get_mut(i) {
                        j.join_type = jt;
                    }
                }
                self.regenerate_sql();
            }
            Message::JoinTableChanged(i, val) => {
                if let Some(ref mut c) = self.customiser {
                    if let Some(j) = c.joins.get_mut(i) {
                        j.table = val;
                    }
                }
                self.regenerate_sql();
            }
            Message::JoinConditionChanged(i, val) => {
                if let Some(ref mut c) = self.customiser {
                    if let Some(j) = c.joins.get_mut(i) {
                        j.on_condition = val;
                    }
                }
                self.regenerate_sql();
            }
            Message::MainTabChanged(tab) => {
                self.main_tab = tab;
            }
            Message::ScratchTableToggled(table, checked) => {
                if checked {
                    if !self.scratch_tables.contains(&table) {
                        self.scratch_tables.push(table);
                    }
                } else {
                    self.scratch_tables.retain(|t| t != &table);
                    // Remove any columns belonging to this table.
                    let prefix = format!("{}.", table);
                    self.scratch_columns.retain(|c| !c.starts_with(&prefix));
                }
                self.scratch_recompute();
            }
            Message::ScratchColumnToggled(col, checked) => {
                if checked {
                    if !self.scratch_columns.contains(&col) {
                        self.scratch_columns.push(col);
                    }
                } else {
                    self.scratch_columns.retain(|c| c != &col);
                }
                self.scratch_recompute();
            }
            Message::ScratchDragStart(i) => {
                self.scratch_drag_idx = Some(i);
                self.scratch_hover_idx = Some(i);
            }
            Message::ScratchDragHover(j) => {
                if self.scratch_drag_idx.is_some() {
                    self.scratch_hover_idx = Some(j);
                }
            }
            Message::ScratchDragDrop(j) => {
                if let Some(i) = self.scratch_drag_idx.take() {
                    if i != j && j < self.scratch_columns.len() {
                        let item = self.scratch_columns.remove(i);
                        self.scratch_columns.insert(j.min(self.scratch_columns.len()), item);
                        self.scratch_recompute();
                    }
                }
                self.scratch_hover_idx = None;
            }
            Message::ScratchDragCancel => {
                self.scratch_drag_idx = None;
                self.scratch_hover_idx = None;
            }
            Message::ScratchCopyToClipboard => {
                return iced::clipboard::write(self.scratch_sql.text());
            }
            Message::ScratchSqlEditorAction(action) => {
                let is_paste = matches!(
                    &action,
                    text_editor::Action::Edit(text_editor::Edit::Paste(_))
                );
                self.scratch_sql.perform(action);
                if is_paste {
                    self.scratch_parse_from_sql();
                }
            }
            Message::ResetTemplates => {
                self.selected_template = None;
                self.parameters.clear();
                self.sql_editor = text_editor::Content::new();
                self.customiser = None;
                self.error = None;
            }
            Message::ResetScratch => {
                self.scratch_tables.clear();
                self.scratch_columns.clear();
                self.scratch_joins.clear();
                self.scratch_auto_added.clear();
                self.scratch_sql = text_editor::Content::new();
                self.scratch_drag_idx = None;
                self.scratch_hover_idx = None;
                self.scratch_unknown_tables.clear();
                self.scratch_unknown_columns.clear();
            }
            Message::EmailSql(sql) => {
                self.email_error = None;
                if !sql.trim().is_empty() {
                    let body = percent_encode(&sql);
                    let uri = format!("mailto:?subject=SQL%20Query&body={body}");
                    return Task::perform(
                        tokio::task::spawn_blocking(move || {
                            open::that(uri).map_err(|e| e.to_string())
                        }),
                        |res| match res {
                            Ok(Ok(())) => Message::EmailOpened,
                            Ok(Err(e)) => Message::EmailFailed(e),
                            Err(e)     => Message::EmailFailed(e.to_string()),
                        },
                    );
                }
            }
            Message::EmailOpened => {
                self.email_error = None;
            }
            Message::EmailFailed(e) => {
                self.email_error = Some(format!("Could not open email client: {e}"));
            }
            Message::Noop => {}
        }
        Task::none()
    }

    fn regenerate_sql(&mut self) {
        if let Some(ref c) = self.customiser {
            let sql = c.build_sql();
            self.sql_editor = text_editor::Content::with_text(&sql);
        }
    }

    fn scratch_recompute(&mut self) {
        let (joins, auto_added) = plan_joins(&self.scratch_tables);
        self.scratch_joins = joins;
        self.scratch_auto_added = auto_added;
        let sql = self.build_scratch_sql();
        self.scratch_sql = text_editor::Content::with_text(&sql);
    }

    fn build_scratch_sql(&self) -> String {
        if self.scratch_joins.is_empty() {
            return String::new();
        }
        let select = if self.scratch_columns.is_empty() {
            "*".to_string()
        } else {
            self.scratch_columns
                .iter()
                .enumerate()
                .map(|(i, c)| if i == 0 { c.clone() } else { format!("    {c}") })
                .collect::<Vec<_>>()
                .join(",\n")
        };
        let base = &self.scratch_joins[0].0;
        let mut sql = format!("SELECT {select}\nFROM {base}");
        for (table, on_cond) in self.scratch_joins.iter().skip(1) {
            if on_cond.is_empty() {
                sql += &format!("\nLEFT JOIN {table}");
            } else {
                sql += &format!("\nLEFT JOIN {table}\n    ON {on_cond}");
            }
        }
        sql
    }

    /// Parse the current scratch SQL editor content (after a paste) and
    /// auto-select matching tables/columns, flagging unknowns.
    fn scratch_parse_from_sql(&mut self) {
        let sql = self.scratch_sql.text();
        let norm: String = sql.split_whitespace().collect::<Vec<_>>().join(" ");
        let up = norm.to_ascii_uppercase();

        // ── Extract SELECT fields ──────────────────────────────────────────
        let from_at = up.find(" FROM ").unwrap_or(up.len());
        let raw_fields: Vec<String> = norm
            .get(7..from_at)
            .map(|s| s.trim())
            .unwrap_or("")
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty() && s != "*")
            .collect();

        // ── Extract table names from FROM + JOINs ─────────────────────────
        let rest = &norm[(from_at + 6).min(norm.len())..];
        let up_rest = rest.to_ascii_uppercase();

        let clause_kws: &[&str] = &[
            " WHERE ", " ORDER ", " INNER ", " LEFT ",
            " RIGHT ", " JOIN ", " CROSS ", " ON ",
        ];
        let first_end = clause_kws
            .iter()
            .filter_map(|kw| up_rest.find(kw))
            .min()
            .unwrap_or(rest.len());
        let mut raw_tables: Vec<String> = vec![rest[..first_end].trim().to_string()];

        // Walk through all JOIN occurrences.
        let mut search_from = 0usize;
        while let Some(rel) = up_rest[search_from..].find(" JOIN ") {
            let abs = search_from + rel + 6;
            let tail = &rest[abs..];
            let up_tail = &up_rest[abs..];
            let end = clause_kws
                .iter()
                .filter_map(|kw| up_tail.find(kw))
                .min()
                .unwrap_or(tail.len());
            let tname = tail[..end].trim().to_string();
            if !tname.is_empty() { raw_tables.push(tname); }
            search_from = abs;
        }

        // ── Match tables against schema ────────────────────────────────────
        let mut new_tables: Vec<String> = Vec::new();
        let mut new_unknown_tables: Vec<String> = Vec::new();
        for t in &raw_tables {
            let tup = t.to_ascii_uppercase();
            if stm_schema::find_table(&tup).is_some() {
                if !new_tables.contains(&tup) { new_tables.push(tup); }
            } else if !t.is_empty() && !new_unknown_tables.contains(t) {
                new_unknown_tables.push(t.clone());
            }
        }

        // ── Match SELECT fields against schema ─────────────────────────────
        let mut new_columns: Vec<String> = Vec::new();
        let mut new_unknown_columns: Vec<String> = Vec::new();

        // All effective tables (known) to search unqualified column names.
        let all_known: Vec<&str> = new_tables.iter().map(|s| s.as_str()).collect();

        for field in &raw_fields {
            let fup = field.to_ascii_uppercase();
            if let Some((tbl, col)) = fup.split_once('.') {
                // Qualified: TABLE.COLUMN
                if let Some(t) = stm_schema::find_table(tbl) {
                    if t.columns.iter().any(|c| c.name == col) {
                        let q = format!("{}.{}", tbl, col);
                        if !new_columns.contains(&q) { new_columns.push(q); }
                    } else {
                        let q = format!("{}.{}", tbl, col);
                        if !new_unknown_columns.contains(&q) { new_unknown_columns.push(q); }
                    }
                } else {
                    if !new_unknown_columns.contains(&fup) { new_unknown_columns.push(fup); }
                }
            } else {
                // Unqualified: search all known tables
                let mut found = false;
                for tname in &all_known {
                    if let Some(t) = stm_schema::find_table(tname) {
                        if t.columns.iter().any(|c| c.name == fup) {
                            let q = format!("{}.{}", tname, fup);
                            if !new_columns.contains(&q) { new_columns.push(q); }
                            found = true;
                            break;
                        }
                    }
                }
                if !found && !new_unknown_columns.contains(&fup) {
                    new_unknown_columns.push(fup);
                }
            }
        }

        self.scratch_tables = new_tables;
        self.scratch_columns = new_columns;
        self.scratch_unknown_tables = new_unknown_tables;
        self.scratch_unknown_columns = new_unknown_columns;
        self.scratch_drag_idx = None;
        self.scratch_hover_idx = None;

        // Recompute joins (but don't overwrite the SQL the user just pasted).
        let (joins, auto_added) = plan_joins(&self.scratch_tables);
        self.scratch_joins = joins;
        self.scratch_auto_added = auto_added;
    }

    fn persist_settings(&self) {
        Settings {
            font_size: self.font_size,
            theme_color: self.theme_color.clone(),
            theme_mode: self.theme_mode.clone(),
        }
        .save();
    }

    fn all_templates(&self) -> Vec<QueryTemplate> {
        let mut t = QueryTemplate::builtin();
        t.extend(self.file_templates.iter().cloned());
        t.extend(self.user_templates.iter().map(|ut| QueryTemplate {
            id: ut.id.clone(),
            group: Some("My Templates".to_string()),
            name: ut.name.clone(),
            description: ut.description.clone(),
            sql: ut.sql.clone(),
        }));
        t
    }

    pub fn view(&self) -> Element<'_, Message> {
        match self.view {
            AppView::Main => self.view_main(),
            AppView::Builder => self.view_builder(),
        }
    }

    // ── Main view ─────────────────────────────────────────────────────────────

    fn view_main(&self) -> Element<'_, Message> {
        let fs = self.font_size as f32;
        let tc = &self.theme_color;
        let tm = &self.theme_mode;

        let bar = toolbar(
            row![
                ghost_btn("Open File", fs).on_press(Message::OpenFile),
                horizontal_space(),
                font_size_control(self, fs),
                row![
                    theme_swatch(ThemeColor::Blue,  ThemeMode::Light, tc == &ThemeColor::Blue  && tm == &ThemeMode::Light),
                    theme_swatch(ThemeColor::Blue,  ThemeMode::Dark,  tc == &ThemeColor::Blue  && tm == &ThemeMode::Dark),
                ].spacing(3),
                row![
                    theme_swatch(ThemeColor::Gray,  ThemeMode::Light, tc == &ThemeColor::Gray  && tm == &ThemeMode::Light),
                    theme_swatch(ThemeColor::Gray,  ThemeMode::Dark,  tc == &ThemeColor::Gray  && tm == &ThemeMode::Dark),
                ].spacing(3),
                row![
                    theme_swatch(ThemeColor::Green, ThemeMode::Light, tc == &ThemeColor::Green && tm == &ThemeMode::Light),
                    theme_swatch(ThemeColor::Green, ThemeMode::Dark,  tc == &ThemeColor::Green && tm == &ThemeMode::Dark),
                ].spacing(3),
            ]
            .spacing(8)
            .align_y(Vertical::Center),
        );

        let tab_strip = self.view_tab_strip(fs);

        let body: Element<Message> = match self.main_tab {
            MainTab::Templates => row![
                self.view_template_tree(fs),
                self.view_editor(fs),
            ]
            .height(Length::Fill)
            .into(),
            MainTab::FromScratch => self.view_scratch(fs),
        };

        container(column![bar, tab_strip, body])
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|theme: &Theme| container::Style {
                background: Some(theme.extended_palette().background.base.color.into()),
                ..Default::default()
            })
            .into()
    }

    // ── Tab strip ─────────────────────────────────────────────────────────────

    fn view_tab_strip(&self, fs: f32) -> Element<'_, Message> {
        let active = &self.main_tab;
        let make_tab = |label: &'static str, tab: MainTab| -> Element<'static, Message> {
            let is_active = *active == tab;
            button(text(label).size(fs - 1.0))
                .padding([6, 18])
                .on_press(Message::MainTabChanged(tab))
                .style(move |theme: &Theme, status| {
                    let pal = theme.extended_palette();
                    let (bg, fg, border_color) = if is_active {
                        (
                            Some(pal.background.base.color.into()),
                            pal.primary.base.color,
                            pal.primary.base.color,
                        )
                    } else {
                        let hover = matches!(status, button::Status::Hovered | button::Status::Pressed);
                        (
                            if hover { Some(Color { a: 0.08, ..pal.background.base.text }.into()) } else { None },
                            Color { a: 0.6, ..pal.background.base.text },
                            Color::TRANSPARENT,
                        )
                    };
                    button::Style {
                        background: bg,
                        text_color: fg,
                        border: Border {
                            radius: 4.0.into(),
                            width: if is_active { 2.0 } else { 0.0 },
                            color: border_color,
                        },
                        shadow: iced::Shadow::default(),
                    }
                })
                .into()
        };

        container(
            row![
                make_tab("TEMPLATES",    MainTab::Templates),
                make_tab("FROM SCRATCH", MainTab::FromScratch),
            ]
            .spacing(2)
            .align_y(Vertical::Center),
        )
        .padding([4u16, 8])
        .width(Length::Fill)
        .style(|theme: &Theme| {
            let pal = theme.extended_palette();
            container::Style {
                background: Some(
                    mix(pal.background.base.color, pal.primary.base.color, 0.10).into(),
                ),
                ..Default::default()
            }
        })
        .into()
    }

    // ── From-Scratch view ─────────────────────────────────────────────────────

    fn view_scratch(&self, fs: f32) -> Element<'_, Message> {
        let all_tables = stm_schema::all_tables();

        // ── Left: table selector panel ────────────────────────────────────────
        let mut table_checks: Vec<Element<Message>> = all_tables
            .iter()
            .map(|t| {
                let tname = t.name.to_string();
                let checked = self.scratch_tables.contains(&tname);
                let is_auto = self.scratch_auto_added.contains(&tname);
                let label = if is_auto {
                    format!("{} (auto)", t.name)
                } else {
                    format!("{}\n{}", t.name, t.label)
                };
                checkbox(label, checked)
                    .text_size(fs - 2.0)
                    .style(checkbox_style)
                    .on_toggle(move |b| Message::ScratchTableToggled(tname.clone(), b))
                    .into()
            })
            .collect();

        // Unknown tables from pasted SQL — shown with a red badge, no checkbox.
        if !self.scratch_unknown_tables.is_empty() {
            table_checks.push(
                text("── not in schema ──")
                    .size(fs - 3.0)
                    .color(Color { a: 0.4, r: 0.8, g: 0.3, b: 0.3 })
                    .into(),
            );
            for ut in &self.scratch_unknown_tables {
                table_checks.push(
                    container(
                        text(format!("✕  {ut}"))
                            .size(fs - 2.0)
                            .color(Color::from_rgb(0.85, 0.32, 0.32)),
                    )
                    .padding([2, 4])
                    .into(),
                );
            }
        }

        let left = container(
            column![
                text("TABLES").size(fs - 3.0).color(iced::Color { a: 0.5, r: 0.5, g: 0.5, b: 0.5 }),
                scrollable(column(table_checks).spacing(10).padding([4, 4]))
                    .height(Length::Fill),
            ]
            .spacing(8)
            .padding([10, 8])
            .height(Length::Fill),
        )
        .width(Length::Fixed(220.0))
        .height(Length::Fill)
        .style(|theme: &Theme| {
            let pal = theme.extended_palette();
            container::Style {
                background: Some(
                    mix(pal.background.base.color, pal.primary.base.color, 0.14).into(),
                ),
                ..Default::default()
            }
        });

        // ── Right: column selector + SQL output ───────────────────────────────
        let right: Element<Message> = if self.scratch_joins.is_empty() {
            container(
                text("Select one or more tables on the left to build a query.")
                    .size(fs - 1.0),
            )
            .padding([24, 18])
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        } else {
            // Auto-added notice
            let auto_notice: Option<Element<Message>> = if !self.scratch_auto_added.is_empty() {
                let names = self.scratch_auto_added.join(", ");
                Some(
                    container(
                        text(format!("Auto-added to complete joins: {names}"))
                            .size(fs - 2.0),
                    )
                    .padding([6, 10])
                    .width(Length::Fill)
                    .style(|theme: &Theme| {
                        let pal = theme.extended_palette();
                        container::Style {
                            background: Some(
                                mix(pal.background.base.color, pal.primary.base.color, 0.18).into(),
                            ),
                            border: Border { radius: 5.0.into(), ..Default::default() },
                            ..Default::default()
                        }
                    })
                    .into(),
                )
            } else {
                None
            };

            // Column checkboxes grouped by effective table (in join order)
            let effective_tables: Vec<&str> = self.scratch_joins.iter().map(|(t, _)| t.as_str()).collect();

            let mut col_groups: Vec<Element<Message>> = Vec::new();
            for tname in &effective_tables {
                let Some(table) = stm_schema::find_table(tname) else { continue };
                let is_auto = self.scratch_auto_added.iter().any(|a| a == tname);
                let header_label = if is_auto {
                    format!("{} — {} (auto-added)", table.name, table.label)
                } else {
                    format!("{} — {}", table.name, table.label)
                };
                col_groups.push(
                    text(header_label)
                        .size(fs - 2.0)
                        .color(iced::Color { a: if is_auto { 0.5 } else { 0.75 }, r: 0.4, g: 0.55, b: 0.75 })
                        .into(),
                );
                let checks: Vec<Element<Message>> = table
                    .columns
                    .iter()
                    .map(|col| {
                        let qualified = format!("{}.{}", table.name, col.name);
                        let checked = self.scratch_columns.contains(&qualified);
                        let q2 = qualified.clone();
                        checkbox(
                            format!("{:<14}  {}", col.name, col.label),
                            checked,
                        )
                        .text_size(fs - 2.0)
                        .style(checkbox_style)
                        .on_toggle(move |b| Message::ScratchColumnToggled(q2.clone(), b))
                        .into()
                    })
                    .collect();
                col_groups.push(
                    container(column(checks).spacing(3))
                        .padding([2, 12])
                        .into(),
                );
            }

            // ── Column picker (left half of right panel) ──────────────────
            let col_selector = container(
                column![
                    text("AVAILABLE COLUMNS").size(fs - 3.0)
                        .color(iced::Color { a: 0.5, r: 0.5, g: 0.5, b: 0.5 }),
                    scrollable(column(col_groups).spacing(8))
                        .height(Length::Fill),
                ]
                .spacing(6)
                .height(Length::Fill),
            )
            .padding([8, 10])
            .height(Length::Fixed(280.0))
            .width(Length::FillPortion(1))
            .style(|theme: &Theme| {
                let pal = theme.extended_palette();
                container::Style {
                    background: Some(
                        mix(pal.background.base.color, pal.primary.base.color, 0.04).into(),
                    ),
                    border: Border { radius: 6.0.into(), ..Default::default() },
                    ..Default::default()
                }
            });

            // ── Column order (right half of right panel) ──────────────────
            let drag_idx  = self.scratch_drag_idx;
            let hover_idx = self.scratch_hover_idx;
            let unknown_cols = &self.scratch_unknown_columns;
            let order_rows: Vec<Element<Message>> = self
                .scratch_columns
                .iter()
                .chain(unknown_cols.iter())
                .enumerate()
                .map(|(i, col)| {
                    let is_unknown = unknown_cols.contains(col);
                    let (tbl, col_name) = col.split_once('.').unwrap_or(("", col.as_str()));
                    let is_dragging = !is_unknown && drag_idx == Some(i);
                    let is_target   = !is_unknown && drag_idx.is_some()
                                      && hover_idx == Some(i) && !is_dragging;

                    let col_color = if is_unknown {
                        Color::from_rgb(0.85, 0.32, 0.32)
                    } else {
                        Color { a: 1.0, r: 0.8, g: 0.8, b: 0.8 }
                    };
                    let tbl_color = if is_unknown {
                        Color { a: 0.7, r: 0.85, g: 0.32, b: 0.32 }
                    } else {
                        Color { a: 0.5, r: 0.5, g: 0.5, b: 0.5 }
                    };

                    let inner = container(
                        row![
                            // Drag handle (hidden for unknowns)
                            text(if is_unknown { "✕" } else { "⠿" })
                                .size(fs)
                                .color(if is_unknown {
                                    Color { a: 0.7, r: 0.85, g: 0.32, b: 0.32 }
                                } else {
                                    Color { a: 0.35, r: 0.5, g: 0.5, b: 0.5 }
                                }),
                            // Ordinal
                            text(format!("{}", i + 1))
                                .size(fs - 3.0)
                                .width(Length::Fixed(16.0))
                                .color(Color { a: 0.4, r: 0.5, g: 0.5, b: 0.5 }),
                            // Column name + table
                            column![
                                text(col_name).size(fs - 1.0).color(col_color),
                                text(tbl).size(fs - 3.0).color(tbl_color),
                            ]
                            .spacing(1)
                            .width(Length::Fill),
                        ]
                        .spacing(6)
                        .align_y(Vertical::Center),
                    )
                    .padding([5, 8])
                    .width(Length::Fill)
                    .style(move |theme: &Theme| {
                        let pal = theme.extended_palette();
                        let (bg, border_color, border_w) = if is_unknown {
                            (
                                Color { a: 0.12, r: 0.85, g: 0.25, b: 0.25 }.into(),
                                Color::from_rgb(0.85, 0.32, 0.32),
                                1.5_f32,
                            )
                        } else if is_dragging {
                            (mix(pal.background.base.color, pal.primary.base.color, 0.30).into(),
                             Color::TRANSPARENT, 0.0)
                        } else if is_target {
                            (mix(pal.background.base.color, pal.primary.base.color, 0.22).into(),
                             pal.primary.base.color, 2.0)
                        } else {
                            (mix(pal.background.base.color, pal.primary.base.color, 0.06).into(),
                             Color::TRANSPARENT, 0.0)
                        };
                        container::Style {
                            background: Some(bg),
                            border: Border { radius: 5.0.into(), width: border_w, color: border_color },
                            ..Default::default()
                        }
                    });

                    if is_unknown {
                        inner.into()
                    } else {
                        mouse_area(inner)
                            .on_press(Message::ScratchDragStart(i))
                            .on_release(Message::ScratchDragDrop(i))
                            .on_enter(Message::ScratchDragHover(i))
                            .into()
                    }
                })
                .collect();

            let order_hint = if drag_idx.is_some() {
                "Release to drop"
            } else {
                "COLUMN ORDER  —  drag ⠿ to reorder"
            };

            let order_body: Element<Message> = if self.scratch_columns.is_empty()
                && self.scratch_unknown_columns.is_empty()
            {
                text("Check columns on the left to add them here.")
                    .size(fs - 2.0)
                    .color(iced::Color { a: 0.4, r: 0.5, g: 0.5, b: 0.5 })
                    .into()
            } else {
                scrollable(column(order_rows).spacing(3))
                    .height(Length::Fill)
                    .into()
            };

            let col_order = mouse_area(
                container(
                    column![
                        text(order_hint).size(fs - 3.0)
                            .color(iced::Color { a: 0.5, r: 0.5, g: 0.5, b: 0.5 }),
                        order_body,
                    ]
                    .spacing(6)
                    .height(Length::Fill),
                )
                .padding([8, 10])
                .height(Length::Fixed(280.0))
                .width(Length::FillPortion(1))
                .style(|theme: &Theme| {
                    let pal = theme.extended_palette();
                    container::Style {
                        background: Some(
                            mix(pal.background.base.color, pal.primary.base.color, 0.07).into(),
                        ),
                        border: Border { radius: 6.0.into(), ..Default::default() },
                        ..Default::default()
                    }
                }),
            )
            .on_release(Message::ScratchDragCancel);

            let pickers = row![col_selector, col_order].spacing(8).height(Length::Fixed(280.0));

            let scratch_sql_text = self.scratch_sql.text();
            let actions = row![
                ghost_btn("Reset", fs).on_press(Message::ResetScratch),
                horizontal_space(),
                ghost_btn("Copy SQL", fs).on_press(Message::ScratchCopyToClipboard),
                ghost_btn("Email SQL", fs).on_press(Message::EmailSql(scratch_sql_text)),
            ]
            .spacing(6)
            .align_y(Vertical::Center);

            let editor = text_editor(&self.scratch_sql)
                .on_action(Message::ScratchSqlEditorAction)
                .font(iced::Font::MONOSPACE)
                .size(fs - 1.0)
                .height(Length::Fill)
                .style(|theme: &Theme, status| {
                    let mut s = iced::widget::text_editor::default(theme, status);
                    s.border.width = 0.0;
                    s.background = theme.extended_palette().background.weak.color.into();
                    s
                });

            let mut content: Vec<Element<Message>> = Vec::new();
            if let Some(notice) = auto_notice { content.push(notice); }
            content.push(pickers.into());
            content.push(actions.into());
            if let Some(ref e) = self.email_error {
                content.push(
                    text(format!("⚠ {e}"))
                        .size(fs - 2.0)
                        .color(Color::from_rgb(0.85, 0.32, 0.32))
                        .into(),
                );
            }
            content.push(editor.into());

            container(
                column(content)
                    .spacing(10)
                    .padding([10, 14])
                    .height(Length::Fill),
            )
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        };

        row![left, right]
            .height(Length::Fill)
            .into()
    }

    // ── Template tree (left panel) ────────────────────────────────────────────

    fn view_template_tree(&self, fs: f32) -> Element<'_, Message> {
        let selected = &self.selected_template;
        let all = self.all_templates();
        let mut tree: Vec<Element<Message>> = Vec::new();

        // Collect unique groups preserving first-seen order.
        let mut seen_groups: Vec<String> = Vec::new();
        for t in &all {
            let g = t.group.clone().unwrap_or_else(|| "Other".to_string());
            if !seen_groups.contains(&g) {
                seen_groups.push(g);
            }
        }
        // Always show "My Templates" last.
        if let Some(pos) = seen_groups.iter().position(|g| g == "My Templates") {
            let g = seen_groups.remove(pos);
            seen_groups.push(g);
        }

        for group_name in &seen_groups {
            let expanded = *self.group_expanded.get(group_name.as_str()).unwrap_or(&true);
            let arrow = if expanded { "▾" } else { "▸" };
            tree.push(nav_group_header(arrow, group_name, fs,
                Message::ToggleGroup(group_name.clone())));

            if expanded {
                let members: Vec<&QueryTemplate> = all
                    .iter()
                    .filter(|t| t.group.as_deref().unwrap_or("Other") == group_name)
                    .collect();

                if members.is_empty() {
                    tree.push(
                        container(text("No templates yet.").size(fs - 2.0))
                            .padding([2, 16])
                            .into(),
                    );
                } else {
                    for t in members {
                        let active = selected.as_deref() == Some(&t.id);
                        tree.push(nav_item(t.name.clone(), t.id.clone(), active, fs));
                    }
                }
            }
        }

        let new_btn = solid_btn("+ New Template", fs)
            .on_press(Message::OpenBuilder)
            .width(Length::Fill);

        container(
            column![
                scrollable(
                    column(tree).spacing(1).padding([6, 4]).width(Length::Fill)
                )
                .height(Length::Fill),
                container(new_btn).padding([8, 8]).width(Length::Fill),
            ]
            .height(Length::Fill),
        )
        .width(Length::Fixed(210.0))
        .height(Length::Fill)
        .style(|theme: &Theme| {
            let pal = theme.extended_palette();
            container::Style {
                background: Some(mix(pal.background.base.color, pal.primary.base.color, 0.14).into()),
                ..Default::default()
            }
        })
        .into()
    }

    // ── Editor panel (right column) ───────────────────────────────────────────

    fn view_editor(&self, fs: f32) -> Element<'_, Message> {
        let all = self.all_templates();
        let selected = &self.selected_template;

        let param_area: Option<Element<Message>> = selected
            .as_ref()
            .and_then(|id| all.iter().find(|t| &t.id == id))
            .and_then(|tmpl| {
                let keys = tmpl.param_keys();
                if keys.is_empty() {
                    None
                } else {
                    let rows: Vec<Element<Message>> = keys
                        .into_iter()
                        .map(|key| {
                            let val = self.parameters.get(&key).map(|s| s.as_str()).unwrap_or("");
                            row![
                                text(key.clone()).size(fs - 1.0).width(Length::Fixed(130.0)),
                                text_input("", val)
                                    .size(fs - 1.0)
                                    .style(|theme: &Theme, status| {
                                        let mut s = iced::widget::text_input::default(theme, status);
                                        s.border.width = 0.0;
                                        s.background =
                                            theme.extended_palette().background.weak.color.into();
                                        s
                                    })
                                    .on_input(move |v| Message::ParameterChanged(key.clone(), v))
                                    .width(Length::Fill),
                            ]
                            .spacing(8)
                            .align_y(Vertical::Center)
                            .into()
                        })
                        .collect();
                    Some(column(rows).spacing(4).into())
                }
            });

        let error_row: Option<Element<Message>> = self.error.as_ref().map(|e| {
            text(format!("⚠ {e}"))
                .size(fs - 1.0)
                .color(Color::from_rgb(0.65, 0.25, 0.25))
                .into()
        });

        let sql_text = self.sql_editor.text();
        let email_err_row: Option<Element<Message>> = self.email_error.as_ref().map(|e| {
            text(format!("⚠ {e}"))
                .size(fs - 2.0)
                .color(Color::from_rgb(0.85, 0.32, 0.32))
                .into()
        });
        let actions = row![
            solid_btn("Generate", fs).on_press(Message::GenerateQuery),
            ghost_btn("Reset", fs).on_press(Message::ResetTemplates),
            horizontal_space(),
            ghost_btn("Copy", fs).on_press(Message::CopyToClipboard),
            ghost_btn("Email", fs).on_press(Message::EmailSql(sql_text)),
            ghost_btn("Save", fs).on_press(Message::SaveQuery),
        ]
        .spacing(6)
        .align_y(Vertical::Center);

        let editor = text_editor(&self.sql_editor)
            .on_action(Message::SqlEditorAction)
            .font(iced::Font::MONOSPACE)
            .size(fs - 1.0)
            .height(Length::Fill)
            .style(|theme: &Theme, status| {
                let mut s = iced::widget::text_editor::default(theme, status);
                s.border.width = 0.0;
                s.background = theme.extended_palette().background.weak.color.into();
                s
            });

        let mut content: Vec<Element<Message>> = Vec::new();
        if let Some(c) = self.customiser.as_ref() {
            content.push(view_customiser(c, fs));
        }
        if let Some(p) = param_area {
            content.push(p);
        }
        if let Some(e) = error_row {
            content.push(e);
        }
        if let Some(e) = email_err_row {
            content.push(e);
        }
        content.push(actions.into());
        content.push(editor.into());

        container(
            column(content)
                .spacing(8)
                .padding([10, 14])
                .height(Length::Fill),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|theme: &Theme| container::Style {
            background: Some(theme.extended_palette().background.base.color.into()),
            ..Default::default()
        })
        .into()
    }

    // ── Builder view ──────────────────────────────────────────────────────────

    fn view_builder(&self) -> Element<'_, Message> {
        let fs = self.font_size as f32;

        let bar = toolbar(
            row![
                ghost_btn("← Back", fs).on_press(Message::BuilderBack),
                text("New Template").size(fs + 2.0),
                horizontal_space(),
                solid_btn("Save Template", fs).on_press(Message::BuilderSave),
            ]
            .spacing(12)
            .align_y(Vertical::Center),
        );

        let name_section = section(
            "Template Name",
            fs,
            flat_input("e.g. Requests by Cost Center", &self.builder.template_name, fs)
                .on_input(Message::BuilderNameChanged)
                .width(Length::Fill)
                .into(),
        );

        let table_names: Vec<String> =
            stm_schema::all_tables().iter().map(|t| t.name.to_string()).collect();

        let table_desc: Element<Message> =
            if let Some(ref tname) = self.builder.selected_table {
                if let Some(t) = stm_schema::find_table(tname) {
                    text(t.label).size(fs - 2.0).into()
                } else {
                    text("").into()
                }
            } else {
                text("Choose a table to see its fields.").size(fs - 2.0).into()
            };

        let table_section = section(
            "Table",
            fs,
            column![
                pick_list(table_names, self.builder.selected_table.clone(), Message::BuilderTableSelected)
                    .style(pick_list_style)
                    .width(Length::Fill),
                table_desc,
            ]
            .spacing(3)
            .into(),
        );

        let fields_and_conditions: Element<Message> =
            if let Some(ref tname) = self.builder.selected_table {
                if let Some(table) = stm_schema::find_table(tname) {
                    let checkboxes: Vec<Element<Message>> = table
                        .columns
                        .iter()
                        .map(|col| {
                            let col_name = col.name.to_string();
                            let checked = self.builder.selected_columns.contains(&col_name);
                            checkbox(format!("{:<8}  {}", col.name, col.label), checked)
                                .text_size(fs - 1.0)
                                .style(checkbox_style)
                                .on_toggle(move |b| Message::BuilderColumnToggled(col_name.clone(), b))
                                .into()
                        })
                        .collect();

                    let fields_section = section(
                        "SELECT Fields",
                        fs,
                        scrollable(column(checkboxes).spacing(4)).height(180).into(),
                    );

                    let field_options: Vec<String> =
                        table.columns.iter().map(|c| c.name.to_string()).collect();

                    let condition_rows: Vec<Element<Message>> = self
                        .builder
                        .conditions
                        .iter()
                        .enumerate()
                        .map(|(i, cond)| {
                            let opts = field_options.clone();
                            row![
                                pick_list(opts, cond.field.clone(), move |f| {
                                    Message::BuilderConditionFieldChanged(i, f)
                                })
                                .placeholder("Field")
                                .style(pick_list_style)
                                .width(Length::FillPortion(3)),
                                pick_list(
                                    WhereOperator::all(),
                                    Some(cond.operator.clone()),
                                    move |op| Message::BuilderConditionOperatorChanged(i, op),
                                )
                                .style(pick_list_style)
                                .width(Length::FillPortion(1)),
                                flat_input("param name", &cond.param_name, fs)
                                    .on_input(move |v| Message::BuilderConditionParamChanged(i, v))
                                    .width(Length::FillPortion(2)),
                                ghost_btn("×", fs).on_press(Message::BuilderConditionRemoved(i)),
                            ]
                            .spacing(6)
                            .align_y(Vertical::Center)
                            .into()
                        })
                        .collect();

                    let hint = text("param_name → '{param_name}' placeholder in SQL")
                        .size(fs - 2.0);

                    let where_body: Element<Message> = column![
                        hint,
                        column(condition_rows).spacing(6),
                        ghost_btn("+ Add Condition", fs).on_press(Message::BuilderConditionAdded),
                    ]
                    .spacing(6)
                    .into();

                    let conditions_section = section("WHERE Conditions", fs, where_body);

                    let preview = self.builder.build_sql().unwrap_or_else(|| {
                        "— select fields to see preview —".to_string()
                    });
                    let preview_section = section(
                        "SQL Preview",
                        fs,
                        code_block(text(preview).size(fs - 1.0).font(iced::Font::MONOSPACE).into()),
                    );

                    column![fields_section, conditions_section, preview_section]
                        .spacing(14)
                        .into()
                } else {
                    text("").into()
                }
            } else {
                text("").into()
            };

        let error_view: Element<Message> = if let Some(ref e) = self.builder.save_error {
            text(format!("⚠ {e}"))
                .size(fs - 1.0)
                .color(Color::from_rgb(0.65, 0.25, 0.25))
                .into()
        } else {
            text("").into()
        };

        let body = scrollable(
            column![name_section, table_section, fields_and_conditions, error_view]
                .spacing(14)
                .padding([12, 18])
                .width(Length::Fill),
        );

        container(column![bar, body])
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|theme: &Theme| container::Style {
                background: Some(theme.extended_palette().background.base.color.into()),
                ..Default::default()
            })
            .into()
    }
}

// ── Theme builder ─────────────────────────────────────────────────────────────

fn build_theme(color: &ThemeColor, mode: &ThemeMode) -> Theme {
    Theme::custom(format!("{color} {mode}"), palette_for(color, mode))
}

fn palette_for(color: &ThemeColor, mode: &ThemeMode) -> Palette {
    match (color, mode) {
        // Blue — light sits ~0.07 above dark on every channel
        (ThemeColor::Blue, ThemeMode::Light) => Palette {
            background: Color::from_rgb(0.172, 0.208, 0.268),
            text:       Color::from_rgb(0.812, 0.838, 0.874),
            primary:    Color::from_rgb(0.422, 0.554, 0.674),
            success:    Color::from_rgb(0.308, 0.454, 0.392),
            danger:     Color::from_rgb(0.494, 0.330, 0.330),
        },
        (ThemeColor::Blue, ThemeMode::Dark) => Palette {
            background: Color::from_rgb(0.102, 0.137, 0.196),
            text:       Color::from_rgb(0.769, 0.800, 0.847),
            primary:    Color::from_rgb(0.400, 0.533, 0.659),
            success:    Color::from_rgb(0.290, 0.439, 0.376),
            danger:     Color::from_rgb(0.478, 0.314, 0.314),
        },
        // Gray — light sits ~0.07 above dark on every channel
        (ThemeColor::Gray, ThemeMode::Light) => Palette {
            background: Color::from_rgb(0.180, 0.198, 0.228),
            text:       Color::from_rgb(0.762, 0.798, 0.840),
            primary:    Color::from_rgb(0.490, 0.550, 0.626),
            success:    Color::from_rgb(0.308, 0.426, 0.352),
            danger:     Color::from_rgb(0.492, 0.356, 0.356),
        },
        (ThemeColor::Gray, ThemeMode::Dark) => Palette {
            background: Color::from_rgb(0.110, 0.129, 0.157),
            text:       Color::from_rgb(0.722, 0.761, 0.808),
            primary:    Color::from_rgb(0.467, 0.529, 0.608),
            success:    Color::from_rgb(0.290, 0.416, 0.345),
            danger:     Color::from_rgb(0.478, 0.345, 0.345),
        },
        // Green — light sits ~0.06 above dark on every channel
        (ThemeColor::Green, ThemeMode::Light) => Palette {
            background: Color::from_rgb(0.156, 0.208, 0.194),
            text:       Color::from_rgb(0.762, 0.836, 0.808),
            primary:    Color::from_rgb(0.356, 0.554, 0.448),
            success:    Color::from_rgb(0.244, 0.432, 0.336),
            danger:     Color::from_rgb(0.490, 0.382, 0.322),
        },
        (ThemeColor::Green, ThemeMode::Dark) => Palette {
            background: Color::from_rgb(0.094, 0.145, 0.133),
            text:       Color::from_rgb(0.722, 0.800, 0.769),
            primary:    Color::from_rgb(0.333, 0.533, 0.427),
            success:    Color::from_rgb(0.227, 0.416, 0.322),
            danger:     Color::from_rgb(0.478, 0.376, 0.314),
        },
    }
}

// ── Widget helpers ────────────────────────────────────────────────────────────

/// Solid primary button — used for the main call-to-action.
fn solid_btn(label: &str, fs: f32) -> button::Button<'_, Message> {
    button(text(label).size(fs))
        .padding([5, 12])
        .style(|theme: &Theme, status| {
            let pal = theme.extended_palette();
            let (bg, fg) = match status {
                button::Status::Hovered | button::Status::Pressed => {
                    (pal.primary.strong.color, pal.primary.strong.text)
                }
                _ => (pal.primary.base.color, pal.primary.base.text),
            };
            button::Style {
                background: Some(bg.into()),
                text_color: fg,
                border: Border { radius: 6.0.into(), width: 0.0, color: Color::TRANSPARENT },
                shadow: iced::Shadow::default(),
            }
        })
}

/// Ghost button — transparent background, subtle hover tint.
fn ghost_btn(label: &str, fs: f32) -> button::Button<'_, Message> {
    button(text(label).size(fs))
        .padding([5, 10])
        .style(|theme: &Theme, status| {
            let pal = theme.extended_palette();
            let bg = match status {
                button::Status::Hovered => Some(Color { a: 0.08, ..pal.background.base.text }.into()),
                button::Status::Pressed => Some(Color { a: 0.14, ..pal.background.base.text }.into()),
                _ => None,
            };
            button::Style {
                background: bg,
                text_color: pal.background.base.text,
                border: Border { radius: 6.0.into(), width: 0.0, color: Color::TRANSPARENT },
                shadow: iced::Shadow::default(),
            }
        })
}

/// Flat text input — no border, weak background tint.
fn flat_input<'a>(placeholder: &'a str, value: &'a str, fs: f32) -> text_input::TextInput<'a, Message> {
    text_input(placeholder, value)
        .size(fs - 1.0)
        .style(|theme, status| {
            let mut s = iced::widget::text_input::default(theme, status);
            s.border.width = 0.0;
            s.background = theme.extended_palette().background.weak.color.into();
            s
        })
}

/// Colored square swatch — no border when inactive, thin ring when active.
fn theme_swatch(color: ThemeColor, mode: ThemeMode, active: bool) -> Element<'static, Message> {
    let fill = swatch_color(&color, &mode);
    button(
        container(text(""))
            .width(Length::Fixed(16.0))
            .height(Length::Fixed(16.0)),
    )
    .padding(2)
    .on_press(Message::SetTheme(color, mode))
    .style(move |theme: &Theme, _status| button::Style {
        background: Some(fill.into()),
        border: Border {
            radius: 4.0.into(),
            width: if active { 2.0 } else { 0.0 },
            color: theme.extended_palette().background.base.text,
        },
        text_color: Color::TRANSPARENT,
        shadow: iced::Shadow::default(),
    })
    .into()
}

/// Linear interpolate between two colors by `t` (0 = base, 1 = onto).
fn mix(base: Color, onto: Color, t: f32) -> Color {
    Color {
        r: base.r + (onto.r - base.r) * t,
        g: base.g + (onto.g - base.g) * t,
        b: base.b + (onto.b - base.b) * t,
        a: 1.0,
    }
}

fn swatch_color(color: &ThemeColor, mode: &ThemeMode) -> Color {
    match (color, mode) {
        (ThemeColor::Blue,  ThemeMode::Light) => Color::from_rgb(0.27, 0.36, 0.49),
        (ThemeColor::Blue,  ThemeMode::Dark)  => Color::from_rgb(0.16, 0.24, 0.36),
        (ThemeColor::Gray,  ThemeMode::Light) => Color::from_rgb(0.29, 0.33, 0.40),
        (ThemeColor::Gray,  ThemeMode::Dark)  => Color::from_rgb(0.20, 0.24, 0.30),
        (ThemeColor::Green, ThemeMode::Light) => Color::from_rgb(0.24, 0.35, 0.31),
        (ThemeColor::Green, ThemeMode::Dark)  => Color::from_rgb(0.16, 0.26, 0.23),
    }
}

/// Font size − / [input] / + control.
fn font_size_control(app: &App, fs: f32) -> Element<'_, Message> {
    let dec = {
        let b = ghost_btn("−", fs);
        if app.font_size > FONT_MIN { b.on_press(Message::FontDecrease) } else { b }
    };
    let inc = {
        let b = ghost_btn("+", fs);
        if app.font_size < FONT_MAX { b.on_press(Message::FontIncrease) } else { b }
    };
    let size_input = flat_input("", &app.font_size_raw, fs)
        .on_input(Message::FontSizeInput)
        .width(Length::Fixed(36.0));

    row![dec, size_input, inc]
        .spacing(1)
        .align_y(Vertical::Center)
        .into()
}

/// Thin top bar — primary-tinted background.
fn toolbar<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    container(content)
        .padding([7, 12])
        .width(Length::Fill)
        .style(|theme: &Theme| {
            let pal = theme.extended_palette();
            container::Style {
                background: Some(
                    mix(pal.background.base.color, pal.primary.base.color, 0.10).into(),
                ),
                ..Default::default()
            }
        })
        .into()
}

/// Collapsible group label in the nav sidebar.
fn nav_group_header(arrow: &str, label: &str, fs: f32, msg: Message) -> Element<'static, Message> {
    let arrow = arrow.to_string();
    let label = label.to_uppercase();
    button(
        row![text(arrow).size(fs - 3.0), text(label).size(fs - 3.0)]
            .spacing(5)
            .align_y(Vertical::Center),
    )
    .on_press(msg)
    .padding([8, 10])
    .style(|theme: &Theme, _status| button::Style {
        background: None,
        text_color: theme.extended_palette().primary.base.color,
        border: Border { width: 0.0, ..Default::default() },
        shadow: iced::Shadow::default(),
    })
    .width(Length::Fill)
    .into()
}

/// Clickable template name in the nav sidebar.
fn nav_item(name: String, id: String, active: bool, fs: f32) -> Element<'static, Message> {
    button(text(name).size(fs - 1.0))
        .on_press(Message::TemplateSelected(id))
        .padding([5, 14])
        .style(move |theme: &Theme, status| {
            let pal = theme.extended_palette();
            let (bg, fg) = if active {
                (
                    Some(mix(pal.background.base.color, pal.primary.base.color, 0.22).into()),
                    pal.primary.strong.color,
                )
            } else if matches!(status, button::Status::Hovered | button::Status::Pressed) {
                (
                    Some(mix(pal.background.base.color, pal.primary.base.color, 0.10).into()),
                    pal.background.base.text,
                )
            } else {
                (None, pal.background.base.text)
            };
            button::Style {
                background: bg,
                text_color: fg,
                border: Border { radius: 5.0.into(), width: 0.0, color: Color::TRANSPARENT },
                shadow: iced::Shadow::default(),
            }
        })
        .width(Length::Fill)
        .into()
}

fn section<'a>(title: &'a str, fs: f32, content: Element<'a, Message>) -> Element<'a, Message> {
    column![
        text(title).size(fs - 1.0),
        content,
    ]
    .spacing(6)
    .into()
}

/// Flat pick_list — borderless, primary-tinted handle color.
fn pick_list_style(theme: &Theme, status: iced::widget::pick_list::Status) -> iced::widget::pick_list::Style {
    use iced::widget::pick_list;
    let pal = theme.extended_palette();
    let bg = match status {
        pick_list::Status::Opened | pick_list::Status::Hovered => {
            mix(pal.background.base.color, pal.primary.base.color, 0.14)
        }
        _ => mix(pal.background.base.color, pal.primary.base.color, 0.06),
    };
    pick_list::Style {
        text_color: pal.background.base.text,
        placeholder_color: Color { a: 0.45, ..pal.background.base.text },
        handle_color: pal.primary.base.color,
        background: bg.into(),
        border: Border { radius: 6.0.into(), width: 0.0, color: Color::TRANSPARENT },
    }
}

/// Modern checkbox — primary color when checked, flat border.
fn checkbox_style(theme: &Theme, status: iced::widget::checkbox::Status) -> iced::widget::checkbox::Style {
    use iced::widget::checkbox;
    let pal = theme.extended_palette();
    let (is_checked, is_hovered) = match status {
        checkbox::Status::Active   { is_checked } => (is_checked, false),
        checkbox::Status::Hovered  { is_checked } => (is_checked, true),
        checkbox::Status::Disabled { is_checked } => (is_checked, false),
    };
    let (bg, border_color, border_width) = if is_checked {
        (pal.primary.base.color.into(), pal.primary.base.color, 0.0_f32)
    } else if is_hovered {
        (
            mix(pal.background.base.color, pal.primary.base.color, 0.18).into(),
            pal.primary.base.color,
            1.5_f32,
        )
    } else {
        (
            mix(pal.background.base.color, pal.primary.base.color, 0.10).into(),
            Color { a: 0.55, ..pal.background.base.text },
            1.5_f32,
        )
    };
    checkbox::Style {
        background: bg,
        icon_color: pal.primary.base.text,
        border: Border { radius: 4.0.into(), width: border_width, color: border_color },
        text_color: None,
    }
}

fn view_customiser<'a>(c: &'a QueryCustomiser, fs: f32) -> Element<'a, Message> {
    // ── Fields ────────────────────────────────────────────────────────────────
    // Pack checkboxes into rows of 3 columns.
    let n = c.available_columns.len();
    let mut checkbox_rows: Vec<Element<'a, Message>> = Vec::new();
    let mut i = 0;
    while i < n {
        let end = (i + 3).min(n);
        let mut row_els: Vec<Element<'a, Message>> = Vec::new();
        for col in &c.available_columns[i..end] {
            let col_name = col.clone();
            let checked = c.selected_fields.contains(&col_name);
            row_els.push(
                checkbox(col_name.clone(), checked)
                    .text_size(fs - 1.0)
                    .style(checkbox_style)
                    .on_toggle(move |b| Message::FieldToggled(col_name.clone(), b))
                    .width(Length::FillPortion(1))
                    .into(),
            );
        }
        while row_els.len() < 3 {
            row_els.push(horizontal_space().width(Length::FillPortion(1)).into());
        }
        checkbox_rows.push(row(row_els).spacing(6).into());
        i += 3;
    }

    let add_field_row = row![
        flat_input("Add field…", &c.new_field_input, fs)
            .on_input(Message::CustomFieldInput)
            .width(Length::Fill),
        ghost_btn("+ Add", fs).on_press(Message::AddCustomField),
    ]
    .spacing(6)
    .align_y(Vertical::Center);

    let fields_panel = section(
        "SELECT Fields",
        fs,
        column![
            scrollable(column(checkbox_rows).spacing(3)).height(Length::Fixed(130.0)),
            add_field_row,
        ]
        .spacing(6)
        .into(),
    );

    // ── Joins ─────────────────────────────────────────────────────────────────
    let join_rows: Vec<Element<'a, Message>> = c
        .joins
        .iter()
        .enumerate()
        .map(|(i, j)| {
            row![
                pick_list(JoinType::all(), Some(j.join_type.clone()), move |jt| {
                    Message::JoinTypeChanged(i, jt)
                })
                .style(pick_list_style)
                .width(Length::Fixed(120.0)),
                flat_input("table", &j.table, fs)
                    .on_input(move |v| Message::JoinTableChanged(i, v))
                    .width(Length::FillPortion(2)),
                flat_input("ON …", &j.on_condition, fs)
                    .on_input(move |v| Message::JoinConditionChanged(i, v))
                    .width(Length::FillPortion(3)),
                ghost_btn("×", fs).on_press(Message::RemoveJoin(i)),
            ]
            .spacing(6)
            .align_y(Vertical::Center)
            .into()
        })
        .collect();

    let joins_panel = section(
        "JOIN Tables",
        fs,
        column![
            column(join_rows).spacing(4),
            ghost_btn("+ Add Join", fs).on_press(Message::AddJoin),
        ]
        .spacing(6)
        .into(),
    );

    container(
        column![fields_panel, joins_panel].spacing(12),
    )
    .padding([8u16, 0u16])
    .into()
}

/// Percent-encode a string for use in a `mailto:` URI body.
fn percent_encode(s: &str) -> String {
    s.bytes().fold(String::with_capacity(s.len() * 2), |mut out, b| {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9'
            | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b'\n' => out.push_str("%0A"),
            b'\r' => {}
            _ => { let _ = std::fmt::write(&mut out, format_args!("%{b:02X}")); }
        }
        out
    })
}

fn code_block(content: Element<'_, Message>) -> Element<'_, Message> {
    container(scrollable(content).height(140))
        .padding(10)
        .width(Length::Fill)
        .style(|theme: &Theme| container::Style {
            background: Some(theme.extended_palette().background.weak.color.into()),
            border: Border { radius: 6.0.into(), width: 0.0, color: Color::TRANSPARENT },
            ..Default::default()
        })
        .into()
}
