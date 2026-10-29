//! Datadog dashboard widgets and ObserveCraft's, for the Datadog-compatible `/api/v1/dashboard`
//! ([`super::dashboards`], ADR-0003).
//!
//! - [`check_definition`]: a Datadog widget definition the web app can render, or the path of the part
//!   it can't (the whole request is then rejected: widgets are never dropped).
//! - [`to_observecraft`]: the UI's document (`layoutType`, `templateVariables`, `widgets`) derived from
//!   a Datadog dashboard, as `scripts/import-integration-dashboards.mjs` converts integration
//!   dashboards: notes, free text, images and groups become our own widgets, every other widget an
//!   `integration` widget carrying its Datadog definition (`original`), which the dashboard page
//!   renders (`web/src/pages/dashboards/widgets/IntegrationWidget.tsx`); `log_stream` and
//!   `manage_status` become our log stream and monitor summary widgets.
//! - [`to_datadog_widgets`]: the Datadog widgets of our document. A widget the UI left as it was
//!   derived is the stored Datadog widget, verbatim; a widget the UI changed or added is translated
//!   back from ours.

use std::collections::HashMap;

use serde_json::{json, Map, Value};

/// Widget types the dashboard page renders from a Datadog definition.
pub const SUPPORTED_WIDGETS: &[&str] = &[
    "timeseries",
    "query_value",
    "toplist",
    "query_table",
    "change",
    "heatmap",
    "sunburst",
    "treemap",
    "check_status",
    "list_stream",
    "note",
    "free_text",
    "image",
    "group",
    "log_stream",
    "manage_status",
];
/// Widgets drawn from `requests` (`q`, or `queries` + `formulas`).
const METRIC_WIDGETS: &[&str] = &["timeseries", "query_value", "toplist", "query_table", "change", "heatmap", "sunburst", "treemap"];
/// Data sources of `requests[].queries[]` the panels query.
const QUERY_SOURCES: &[&str] = &["metrics", "logs", "events"];
/// Data sources of a `list_stream` request the panels list.
const LIST_SOURCES: &[&str] = &["event_stream", "infrastructure"];
/// Rows a group's banner image takes above its widgets (as in the importer).
const BANNER_ROWS: f64 = 5.0;

/// What can't be rendered: a path inside the widget definition (`requests[0].queries[1].data_source`)
/// and why.
#[derive(Debug, PartialEq, Eq)]
pub struct Invalid {
    pub path: String,
    pub message: String,
}

impl Invalid {
    fn new(path: impl Into<String>, message: impl Into<String>) -> Self {
        Invalid { path: path.into(), message: message.into() }
    }

    /// The same problem seen from `prefix` (`widgets[2].definition`).
    pub fn under(self, prefix: &str) -> Self {
        let path = if self.path.is_empty() { prefix.to_string() } else { format!("{prefix}.{}", self.path) };
        Invalid { path, message: self.message }
    }
}

impl std::fmt::Display for Invalid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.path, self.message)
    }
}

/// Checks a Datadog widget definition can be rendered, group children included.
pub fn check_definition(def: &Value) -> Result<(), Invalid> {
    check(def, false)
}

fn check(def: &Value, nested: bool) -> Result<(), Invalid> {
    let Some(def) = def.as_object() else { return Err(Invalid::new("", "a widget needs a definition object")) };
    let Some(kind) = def.get("type").and_then(Value::as_str) else { return Err(Invalid::new("type", "the widget definition has no type")) };
    if !SUPPORTED_WIDGETS.contains(&kind) {
        return Err(Invalid::new("type", format!("widget type {kind:?} is not supported by ObserveCraft yet (supported: {})", SUPPORTED_WIDGETS.join(", "))));
    }
    let text = |field: &str| match def.get(field) {
        Some(Value::String(_)) => Ok(()),
        _ => Err(Invalid::new(field, format!("a {kind} widget needs {field} (a string)"))),
    };
    match kind {
        "group" => {
            if nested {
                return Err(Invalid::new("type", "a group can't contain another group"));
            }
            let Some(children) = def.get("widgets").and_then(Value::as_array) else { return Err(Invalid::new("widgets", "a group needs a list of widgets")) };
            for (i, child) in children.iter().enumerate() {
                check(&child["definition"], true).map_err(|e| e.under(&format!("widgets[{i}].definition")))?;
            }
            Ok(())
        }
        "note" => text("content"),
        "free_text" => text("text"),
        "image" => text("url"),
        "check_status" => text("check"),
        "manage_status" => text("query"),
        "log_stream" => match def.get("query") {
            None | Some(Value::String(_)) => Ok(()),
            _ => Err(Invalid::new("query", "a log_stream widget's query must be a string")),
        },
        "list_stream" => {
            for (i, request) in requests(def, kind)?.iter().enumerate() {
                let source = request["query"]["data_source"].as_str().unwrap_or_default();
                if !LIST_SOURCES.contains(&source) {
                    return Err(Invalid::new(
                        format!("requests[{i}].query.data_source"),
                        format!("list_stream data source {source:?} is not supported by ObserveCraft yet (supported: {})", LIST_SOURCES.join(", ")),
                    ));
                }
            }
            Ok(())
        }
        _ => {
            debug_assert!(METRIC_WIDGETS.contains(&kind));
            for (i, request) in requests(def, kind)?.iter().enumerate() {
                let Some(request) = request.as_object() else { return Err(Invalid::new(format!("requests[{i}]"), "a request must be an object")) };
                for key in request.keys() {
                    if key == "query" || key.ends_with("_query") {
                        return Err(Invalid::new(
                            format!("requests[{i}].{key}"),
                            format!("{key} requests are not supported by ObserveCraft yet; use q, or queries with data_source {}", QUERY_SOURCES.join(", ")),
                        ));
                    }
                }
                if let Some(queries) = request.get("queries") {
                    let Some(queries) = queries.as_array() else { return Err(Invalid::new(format!("requests[{i}].queries"), "queries must be a list")) };
                    for (j, query) in queries.iter().enumerate() {
                        let source = query["data_source"].as_str().unwrap_or_default();
                        if !QUERY_SOURCES.contains(&source) {
                            return Err(Invalid::new(
                                format!("requests[{i}].queries[{j}].data_source"),
                                format!("data source {source:?} is not supported by ObserveCraft yet (supported: {})", QUERY_SOURCES.join(", ")),
                            ));
                        }
                    }
                }
            }
            Ok(())
        }
    }
}

fn requests<'a>(def: &'a Map<String, Value>, kind: &str) -> Result<&'a Vec<Value>, Invalid> {
    def.get("requests").and_then(Value::as_array).ok_or_else(|| Invalid::new("requests", format!("a {kind} widget needs a list of requests")))
}

// ================================================================ Datadog → ObserveCraft

/// Our colors for Datadog's note and group colors (`vivid_*` are the saturated variants).
fn note_color(name: &str) -> &'static str {
    match name.trim_start_matches("vivid_") {
        "gray" => "gray",
        "blue" => "blue",
        "purple" | "pink" => "purple",
        "yellow" | "orange" => "yellow",
        "green" => "green",
        "red" => "red",
        _ => "white",
    }
}

fn group_color(name: &str) -> &'static str {
    match name.trim_start_matches("vivid_") {
        "purple" | "pink" => "purple",
        "green" => "green",
        "gray" | "white" => "gray",
        "orange" | "yellow" | "red" => "orange",
        _ => "blue",
    }
}

/// A font size in px (a string or a number, `auto`) as our note sizes.
fn note_size(font_size: &Value) -> &'static str {
    let px = match font_size {
        Value::String(s) => s.parse::<f64>().unwrap_or(0.0),
        v => v.as_f64().unwrap_or(0.0),
    };
    if px == 0.0 || px.is_nan() {
        "md"
    } else if px <= 14.0 {
        "sm"
    } else if px <= 16.0 {
        "md"
    } else if px <= 24.0 {
        "lg"
    } else {
        "xl"
    }
}

fn num(v: &Value) -> f64 {
    v.as_f64().unwrap_or(0.0)
}

fn items(v: &Value) -> &[Value] {
    v.as_array().map(Vec::as_slice).unwrap_or_default()
}

/// How one level of Datadog widgets maps onto our grid: our columns, the Datadog width they span,
/// and our rows per Datadog row.
#[derive(Clone, Copy, Debug)]
struct Scale {
    cols: f64,
    extent: f64,
    row_scale: f64,
}

impl Scale {
    /// The importer's: free dashboards use our 48-column canvas at the top level (Datadog's free
    /// coordinates are grid units, one row each); ordered dashboards and groups use 12 columns with
    /// two of our rows per Datadog row.
    fn of(items: &[Value], source_width: Option<f64>, nested: bool, free: bool) -> Scale {
        let top_free = free && !nested;
        let cols = if top_free { 48.0 } else { 12.0 };
        let floor = if free { 1.0 } else { 12.0 };
        let extent = source_width.unwrap_or_else(|| items.iter().map(|w| num(&w["layout"]["x"]) + num(&w["layout"]["width"])).fold(floor, f64::max));
        Scale { cols, extent: if extent > 0.0 { extent } else { cols }, row_scale: if top_free { 1.0 } else { 2.0 } }
    }

    fn default_for(nested: bool, free: bool) -> Scale {
        let top_free = free && !nested;
        let cols = if top_free { 48.0 } else { 12.0 };
        Scale { cols, extent: cols, row_scale: if top_free { 1.0 } else { 2.0 } }
    }
}

/// Our id of a Datadog widget.
fn our_id(dd: &Value) -> String {
    match &dd["id"] {
        Value::Number(n) => format!("dd-{n}"),
        Value::String(s) => format!("dd-{s}"),
        _ => String::new(),
    }
}

/// The UI's document of a Datadog dashboard (`layoutType`, `templateVariables`, `widgets`).
pub fn to_observecraft(doc: &Value) -> Value {
    let free = doc["layout_type"] == "free";
    json!({
        "layoutType": if free { "free" } else { "grid" },
        "templateVariables": template_variables(doc),
        "widgets": convert_level(items(&doc["widgets"]), None, false, free),
    })
}

/// `{name, prefix, default}` per Datadog template variable (`defaults[0]` when several are set).
pub fn template_variables(doc: &Value) -> Vec<Value> {
    items(&doc["template_variables"]).iter().map(template_variable).collect()
}

fn template_variable(v: &Value) -> Value {
    let default = v["default"].as_str().or_else(|| v["defaults"][0].as_str()).unwrap_or("*");
    json!({ "name": v["name"].as_str().unwrap_or_default(), "prefix": v["prefix"].as_str().unwrap_or_default(), "default": default })
}

fn convert_level(items: &[Value], source_width: Option<f64>, nested: bool, free: bool) -> Vec<Value> {
    let scale = Scale::of(items, source_width, nested, free);
    let mut cursor = 0.0_f64;
    let mut converted = Vec::with_capacity(items.len());
    for item in items {
        let id = our_id(item);
        let d = &item["definition"];
        let kind = d["type"].as_str().unwrap_or_default();
        let str_of = |field: &str| d[field].as_str().unwrap_or_default().to_string();
        let definition = match kind {
            "group" => {
                let mut children = convert_level(items_of(d), item["layout"]["width"].as_f64(), true, free);
                if let Some(banner) = d["banner_img"].as_str().filter(|b| !b.is_empty()) {
                    for child in &mut children {
                        child["layout"]["y"] = json!(child["layout"]["y"].as_i64().unwrap_or(0) + BANNER_ROWS as i64);
                    }
                    children.insert(
                        0,
                        json!({ "id": format!("{id}-banner"), "definition": { "type": "image", "title": "", "src": banner }, "layout": { "x": 0, "y": 0, "w": 12, "h": BANNER_ROWS as i64 } }),
                    );
                }
                let mut group = json!({ "type": "group", "title": str_of("title"), "color": group_color(d["background_color"].as_str().unwrap_or_default()), "compact": false, "widgets": children });
                if d["show_title"] == false {
                    group["showTitle"] = json!(false);
                }
                group
            }
            "note" => {
                let mut note = json!({
                    "type": "note", "content": str_of("content"), "background": note_color(d["background_color"].as_str().unwrap_or_default()),
                    "align": if d["text_align"] == "center" { "center" } else { "left" }, "fontSize": note_size(&d["font_size"]),
                });
                if d["show_tick"] == true {
                    note["pointer"] = json!(true);
                }
                note
            }
            "free_text" => json!({
                "type": "note", "content": str_of("text"), "background": "white",
                "align": if d["text_align"] == "center" { "center" } else { "left" }, "fontSize": note_size(&d["font_size"]),
            }),
            "image" => json!({ "type": "image", "title": "", "src": str_of("url") }),
            "log_stream" => json!({ "type": "log_stream", "title": str_of("title"), "query": str_of("query"), "limit": 50 }),
            "manage_status" => json!({ "type": "monitor_summary", "title": str_of("title"), "query": str_of("query") }),
            // Requests, formulas, units, thresholds and display options as they are.
            _ => json!({ "type": "integration", "title": str_of("title"), "original": d }),
        };
        let layout = &item["layout"];
        let mut geometry = if layout.is_object() {
            let s = scale.cols / scale.extent;
            json!({
                "x": (num(&layout["x"]) * s).round() as i64,
                "y": (num(&layout["y"]) * scale.row_scale).round() as i64,
                "w": ((num(&layout["width"]) * s).round() as i64).max(1),
                "h": ((num(&layout["height"]) * scale.row_scale).round() as i64).max(4),
            })
        } else {
            let h = match kind {
                "group" => 8.max(children_bottom(&definition) + 2),
                "note" => 3,
                _ => 6,
            };
            json!({ "x": 0, "y": cursor as i64, "w": scale.cols as i64, "h": h })
        };
        if definition["type"] == "group" {
            // Two rows for the group's header, unless it has none.
            let header = if definition["showTitle"] == false { 0 } else { 2 };
            let h = geometry["h"].as_i64().unwrap_or(0).max(header + children_bottom(&definition));
            geometry["h"] = json!(h);
        }
        cursor = cursor.max(num(&geometry["y"]) + num(&geometry["h"]));
        converted.push(json!({ "id": id, "definition": definition, "layout": geometry }));
    }
    // Group headers add space that was implicit in Datadog's layout: move later rows below them.
    if !free || nested {
        push_down_overlaps(&mut converted);
    }
    converted
}

fn items_of(d: &Value) -> &[Value] {
    items(&d["widgets"])
}

fn children_bottom(group: &Value) -> i64 {
    items(&group["widgets"]).iter().map(|w| w["layout"]["y"].as_i64().unwrap_or(0) + w["layout"]["h"].as_i64().unwrap_or(0)).max().unwrap_or(0).max(0)
}

fn push_down_overlaps(widgets: &mut [Value]) {
    let rect = |w: &Value| ["x", "y", "w", "h"].map(|k| w["layout"][k].as_i64().unwrap_or(0));
    let mut order: Vec<usize> = (0..widgets.len()).collect();
    order.sort_by_key(|&i| {
        let [x, y, _, _] = rect(&widgets[i]);
        (y, x)
    });
    let mut placed: Vec<[i64; 4]> = Vec::new();
    for i in order {
        let mut a = rect(&widgets[i]);
        loop {
            let bottom = placed.iter().filter(|b| a[0] < b[0] + b[2] && a[0] + a[2] > b[0] && a[1] < b[1] + b[3] && a[1] + a[3] > b[1]).map(|b| b[1] + b[3]).max();
            match bottom {
                Some(y) => a[1] = y,
                None => break,
            }
        }
        widgets[i]["layout"]["y"] = json!(a[1]);
        placed.push(a);
    }
}

// ================================================================ ObserveCraft → Datadog

/// Integer widget ids for widgets that have none (Datadog assigns them on write).
pub fn assign_widget_ids(widgets: &mut [Value]) {
    for w in widgets.iter_mut() {
        if !w["id"].is_i64() && !w["id"].is_u64() {
            w["id"] = json!(random_widget_id());
        }
        if w["definition"]["type"] == "group" {
            if let Some(children) = w["definition"]["widgets"].as_array_mut() {
                assign_widget_ids(children);
            }
        }
    }
}

/// Below 2^53, so JavaScript clients read it exactly.
fn random_widget_id() -> i64 {
    use rand::Rng;
    rand::thread_rng().gen_range(1..(1_i64 << 52))
}

/// A stable Datadog id for a widget made in the UI (FNV-1a of its id, below 2^52).
fn hashed_widget_id(id: &str) -> i64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in id.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    (h & ((1 << 52) - 1)) as i64 + 1
}

fn datadog_id(our: &str) -> i64 {
    our.strip_prefix("dd-").and_then(|n| n.parse().ok()).unwrap_or_else(|| hashed_widget_id(our))
}

/// What the stored Datadog document says about each of our widget ids.
#[derive(Default)]
struct Stored {
    /// Our id → the stored Datadog widget.
    datadog: HashMap<String, Value>,
    /// Our id → our widget as derived from it.
    derived: HashMap<String, Value>,
    /// Group id ("" for the dashboard) → the scale of its level.
    scales: HashMap<String, Scale>,
    /// Groups with a banner image (their children start below it).
    banners: HashMap<String, f64>,
}

impl Stored {
    fn index(doc: &Value) -> Stored {
        let mut s = Stored::default();
        let free = doc["layout_type"] == "free";
        let top = items(&doc["widgets"]);
        s.scales.insert(String::new(), Scale::of(top, None, false, free));
        for w in top {
            s.datadog.insert(our_id(w), w.clone());
            if w["definition"]["type"] == "group" {
                let children = items_of(&w["definition"]);
                s.scales.insert(our_id(w), Scale::of(children, w["layout"]["width"].as_f64(), true, free));
                if w["definition"]["banner_img"].as_str().is_some_and(|b| !b.is_empty()) {
                    s.banners.insert(our_id(w), BANNER_ROWS);
                }
                for c in children {
                    s.datadog.insert(our_id(c), c.clone());
                }
            }
        }
        for w in convert_level(top, None, false, free) {
            for c in items(&w["definition"]["widgets"]) {
                s.derived.insert(c["id"].as_str().unwrap_or_default().to_string(), c.clone());
            }
            s.derived.insert(w["id"].as_str().unwrap_or_default().to_string(), w);
        }
        s
    }
}

/// The Datadog widgets of our widgets, with `stored` the Datadog document last written through the
/// API (if any). Returns the widgets and whether layouts had to be added to widgets that had none
/// (an ordered dashboard with `reflow_type: auto` whose widgets were moved in the UI), which makes
/// the dashboard's reflow type `fixed`.
pub fn to_datadog_widgets(ours: &[Value], stored: Option<&Value>, free: bool) -> (Vec<Value>, bool) {
    let stored = stored.map(Stored::index).unwrap_or_default();
    level_to_datadog(ours, "", &stored, false, free)
}

fn level_to_datadog(ours: &[Value], parent: &str, stored: &Stored, nested: bool, free: bool) -> (Vec<Value>, bool) {
    let scale = stored.scales.get(parent).copied().unwrap_or_else(|| Scale::default_for(nested, free));
    let offset = stored.banners.get(parent).copied().unwrap_or(0.0);
    let banner_id = format!("{parent}-banner");
    let mut fixed = false;
    let mut out: Vec<(Value, Option<Value>, &Value)> = Vec::new();
    for w in ours {
        let id = w["id"].as_str().unwrap_or_default();
        if nested && id == banner_id && stored.banners.contains_key(parent) {
            // The group's banner_img, kept on the group's definition.
            continue;
        }
        let def = &w["definition"];
        let (dd, derived) = (stored.datadog.get(id), stored.derived.get(id));
        let definition = if def["type"] == "group" {
            let (children, children_fixed) = level_to_datadog(items(&def["widgets"]), id, stored, true, free);
            fixed |= children_fixed;
            let mut group = match (dd, derived) {
                (Some(dd), Some(derived)) if dd["definition"]["type"] == "group" => {
                    let mut g = dd["definition"].clone();
                    if !same_group_header(&derived["definition"], def) {
                        apply_group_header(&mut g, def);
                    }
                    g
                }
                _ => reverse_definition(def),
            };
            group["widgets"] = Value::Array(children);
            group
        } else {
            match (dd, derived) {
                (Some(dd), Some(derived)) if derived["definition"] == *def => dd["definition"].clone(),
                _ => reverse_definition(def),
            }
        };
        let layout = match (dd, derived) {
            (Some(dd), Some(derived)) if derived["layout"] == w["layout"] => dd.get("layout").cloned(),
            _ => Some(reverse_layout(&w["layout"], scale, offset)),
        };
        out.push((json!({ "id": datadog_id(id), "definition": definition }), layout, w));
    }
    // Datadog wants every widget of an ordered level to have a layout, or none.
    let some = out.iter().any(|(_, l, _)| l.is_some());
    let all = out.iter().all(|(_, l, _)| l.is_some());
    if some && !all {
        fixed = true;
        for (_, layout, w) in out.iter_mut() {
            if layout.is_none() {
                *layout = Some(reverse_layout(&w["layout"], scale, offset));
            }
        }
    }
    let widgets = out
        .into_iter()
        .map(|(mut widget, layout, _)| {
            if let Some(l) = layout {
                widget["layout"] = l;
            }
            widget
        })
        .collect();
    (widgets, fixed)
}

fn same_group_header(derived: &Value, ours: &Value) -> bool {
    ["title", "color", "showTitle"].iter().all(|k| derived[*k] == ours[*k])
}

fn apply_group_header(group: &mut Value, ours: &Value) {
    group["title"] = json!(ours["title"].as_str().unwrap_or_default());
    group["background_color"] = json!(ours["color"].as_str().unwrap_or("blue"));
    match ours["showTitle"].as_bool() {
        Some(false) => group["show_title"] = json!(false),
        _ => {
            if let Some(g) = group.as_object_mut() {
                g.remove("show_title");
            }
        }
    }
}

fn reverse_layout(layout: &Value, scale: Scale, offset: f64) -> Value {
    let s = scale.extent / scale.cols;
    json!({
        "x": (num(&layout["x"]) * s).round() as i64,
        "y": (((num(&layout["y"]) - offset) / scale.row_scale).round() as i64).max(0),
        "width": ((num(&layout["w"]) * s).round() as i64).max(1),
        "height": ((num(&layout["h"]) / scale.row_scale).round() as i64).max(1),
    })
}

/// A request of one metric query (`queries` + `formulas`, as the provider writes them).
fn metric_request(query: &Value, aggregator: Option<&Value>, response_format: &str) -> Value {
    let mut q = json!({ "data_source": "metrics", "name": "query1", "query": query["query"].as_str().unwrap_or_default() });
    if let Some(agg) = aggregator.and_then(Value::as_str) {
        q["aggregator"] = json!(agg);
    }
    let mut formula = json!({ "formula": "query1" });
    if let Some(alias) = query["alias"].as_str().filter(|a| !a.is_empty()) {
        formula["alias"] = json!(alias);
    }
    json!({ "response_format": response_format, "queries": [q], "formulas": [formula] })
}

fn datadog_palette(ours: &str) -> &'static str {
    match ours {
        "green" => "white_on_green",
        "red" => "white_on_red",
        "yellow" => "white_on_yellow",
        "blue" => "custom_bg",
        _ => "white_on_gray",
    }
}

fn font_px(size: &str) -> &'static str {
    match size {
        "sm" => "14",
        "lg" => "24",
        "xl" => "36",
        _ => "16",
    }
}

/// Our widget definition as a Datadog one (a widget made or changed in the UI).
fn reverse_definition(def: &Value) -> Value {
    let title = def["title"].as_str().unwrap_or_default();
    let first = &def["request"];
    let scalar = |kind: &str| json!({ "type": kind, "title": title, "requests": [metric_request(first, Some(&def["aggregator"]), "scalar")] });
    match def["type"].as_str().unwrap_or_default() {
        "integration" => {
            let mut original = def["original"].clone();
            if original["title"].as_str().unwrap_or_default() != title {
                original["title"] = json!(title);
            }
            original
        }
        "timeseries" => {
            let display = match def["display"].as_str() {
                Some("area") => "area",
                Some("bars") => "bars",
                _ => "line",
            };
            let requests: Vec<Value> = items(&def["requests"])
                .iter()
                .map(|r| {
                    let mut req = metric_request(r, None, "timeseries");
                    req["display_type"] = json!(display);
                    req
                })
                .collect();
            let mut out = json!({ "type": "timeseries", "title": title, "requests": requests });
            if let Some(max) = def["yMax"].as_f64() {
                out["yaxis"] = json!({ "max": max.to_string() });
            }
            out
        }
        "query_value" => {
            let mut out = scalar("query_value");
            let formats: Vec<Value> =
                items(&def["formats"]).iter().map(|f| json!({ "comparator": f["comparator"], "value": f["value"], "palette": datadog_palette(f["palette"].as_str().unwrap_or_default()) })).collect();
            if !formats.is_empty() {
                out["requests"][0]["conditional_formats"] = json!(formats);
            }
            out["precision"] = def["precision"].clone();
            if let Some(unit) = def["unit"].as_str() {
                out["custom_unit"] = json!(unit);
            }
            out
        }
        "toplist" | "bar_chart" | "change" | "table" | "pie" | "treemap" => {
            let kind = match def["type"].as_str().unwrap_or_default() {
                "table" => "query_table",
                "pie" => "sunburst",
                other => other,
            };
            let mut out = scalar(kind);
            if let Some(limit) = def["limit"].as_i64() {
                out["requests"][0]["formulas"][0]["limit"] = json!({ "count": limit, "order": "desc" });
            }
            out
        }
        kind @ ("heatmap" | "distribution") => json!({ "type": kind, "title": title, "requests": [{ "q": first["query"].as_str().unwrap_or_default() }] }),
        "note" => {
            let mut out = json!({
                "type": "note", "content": def["content"].as_str().unwrap_or_default(), "background_color": def["background"].as_str().unwrap_or("white"),
                "font_size": font_px(def["fontSize"].as_str().unwrap_or("md")), "text_align": def["align"].as_str().unwrap_or("left"),
            });
            if def["pointer"] == true {
                out["show_tick"] = json!(true);
            }
            out
        }
        "image" => json!({ "type": "image", "url": def["src"].as_str().unwrap_or_default() }),
        "group" => {
            let mut out = json!({ "type": "group", "layout_type": "ordered", "widgets": [] });
            apply_group_header(&mut out, def);
            out
        }
        "monitor_summary" => json!({ "type": "manage_status", "title": title, "query": def["query"].as_str().unwrap_or_default(), "summary_type": "monitors", "display_format": "countsAndList" }),
        "log_stream" => json!({ "type": "log_stream", "title": title, "query": def["query"].as_str().unwrap_or_default(), "indexes": [], "columns": [] }),
        other => json!({ "type": other, "title": title }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timeseries(q: &str) -> Value {
        json!({ "type": "timeseries", "title": "CPU", "requests": [{ "q": q, "display_type": "line" }] })
    }

    #[test]
    fn renderable_widgets_pass() {
        for def in [
            timeseries("avg:system.cpu.user{*}"),
            json!({ "type": "query_value", "requests": [{ "response_format": "scalar", "queries": [{ "data_source": "metrics", "name": "query1", "query": "avg:system.load.1{*}", "aggregator": "avg" }], "formulas": [{ "formula": "query1" }] }] }),
            json!({ "type": "toplist", "requests": [{ "queries": [{ "data_source": "logs", "name": "query1", "compute": { "aggregation": "count" }, "search": { "query": "service:checkout-api" } }] }] }),
            json!({ "type": "timeseries", "requests": [{ "queries": [{ "data_source": "events", "name": "q", "search": { "query": "" }, "compute": { "aggregation": "count" } }] }] }),
            json!({ "type": "note", "content": "Read me" }),
            json!({ "type": "free_text", "text": "Checkout" }),
            json!({ "type": "image", "url": "https://example.com/logo.png" }),
            json!({ "type": "check_status", "check": "datadog.agent.up", "grouping": "cluster" }),
            json!({ "type": "manage_status", "query": "tag:team:checkout", "summary_type": "monitors" }),
            json!({ "type": "log_stream", "query": "service:checkout-api status:error", "indexes": ["main"] }),
            json!({ "type": "list_stream", "requests": [{ "query": { "data_source": "event_stream", "query_string": "" }, "columns": [] }] }),
            json!({ "type": "group", "layout_type": "ordered", "widgets": [{ "definition": timeseries("avg:system.mem.used{*}") }, { "definition": { "type": "note", "content": "x" } }] }),
        ] {
            assert_eq!(check_definition(&def), Ok(()), "{def}");
        }
    }

    #[test]
    fn widgets_we_cant_render_are_named_by_path() {
        let err = |def: Value| check_definition(&def).unwrap_err();
        let e = err(json!({ "type": "hostmap", "requests": { "fill": { "q": "avg:system.cpu.user{*}" } } }));
        assert_eq!(e.path, "type");
        assert!(e.message.contains("\"hostmap\" is not supported"), "{}", e.message);
        assert_eq!(err(json!({ "type": "timeseries", "requests": [{ "profile_metrics_query": { "compute": {} } }] })).path, "requests[0].profile_metrics_query");
        assert_eq!(err(json!({ "type": "heatmap", "requests": [{ "query": { "data_source": "metrics" } }] })).path, "requests[0].query");
        let e = err(json!({ "type": "timeseries", "requests": [{ "q": "avg:a{*}" }, { "queries": [{ "data_source": "metrics" }, { "data_source": "cloud_cost" }] }] }));
        assert_eq!(e.path, "requests[1].queries[1].data_source");
        assert!(e.message.contains("cloud_cost"));
        assert_eq!(err(json!({ "type": "list_stream", "requests": [{ "query": { "data_source": "logs_stream" } }] })).path, "requests[0].query.data_source");
        assert_eq!(err(json!({ "type": "group", "widgets": [{ "definition": { "type": "note", "content": "" } }, { "definition": { "type": "slo" } }] })).path, "widgets[1].definition.type");
        assert_eq!(err(json!({ "type": "group", "widgets": [{ "definition": { "type": "group", "widgets": [] } }] })).path, "widgets[0].definition.type");
        assert_eq!(err(json!({ "type": "query_value" })).path, "requests");
        assert_eq!(err(json!({ "type": "note" })).path, "content");
        assert_eq!(err(json!({ "requests": [] })).path, "type");
        let e = err(json!({ "type": "timeseries", "requests": [{ "apm_query": {} }] })).under("widgets[3].definition");
        assert!(e.to_string().starts_with("widgets[3].definition.requests[0].apm_query: apm_query requests are not supported"), "{e}");
    }

    fn ordered() -> Value {
        json!({
            "title": "Checkout", "layout_type": "ordered", "reflow_type": "fixed",
            "template_variables": [{ "name": "env", "prefix": "env", "defaults": ["prod"] }, { "name": "service", "prefix": "service" }, { "name": "host", "default": "i-0a1b2c3d4e5f60001" }],
            "widgets": [
                { "id": 11, "definition": timeseries("avg:system.cpu.user{$env}"), "layout": { "x": 0, "y": 0, "width": 6, "height": 3 } },
                { "id": 12, "definition": { "type": "note", "content": "On call: #checkout", "background_color": "vivid_yellow", "font_size": "24", "text_align": "center", "show_tick": true }, "layout": { "x": 6, "y": 0, "width": 6, "height": 2 } },
                { "id": 13, "definition": { "type": "group", "title": "Latency", "background_color": "vivid_purple", "layout_type": "ordered", "widgets": [
                    { "id": 14, "definition": { "type": "query_value", "requests": [{ "q": "avg:trace.http.request.duration{*}", "aggregator": "avg" }], "precision": 2 }, "layout": { "x": 0, "y": 0, "width": 4, "height": 2 } },
                    { "id": 15, "definition": { "type": "free_text", "text": "p95", "font_size": "14" }, "layout": { "x": 4, "y": 0, "width": 8, "height": 2 } },
                ] }, "layout": { "x": 0, "y": 3, "width": 12, "height": 3 } },
            ],
        })
    }

    #[test]
    fn datadog_widgets_become_ours() {
        let ours = to_observecraft(&ordered());
        assert_eq!(ours["layoutType"], "grid");
        assert_eq!(
            ours["templateVariables"],
            json!([{ "name": "env", "prefix": "env", "default": "prod" }, { "name": "service", "prefix": "service", "default": "*" }, { "name": "host", "prefix": "", "default": "i-0a1b2c3d4e5f60001" }])
        );
        let w = &ours["widgets"];
        assert_eq!(
            w[0],
            json!({ "id": "dd-11", "definition": { "type": "integration", "title": "CPU", "original": timeseries("avg:system.cpu.user{$env}") }, "layout": { "x": 0, "y": 0, "w": 6, "h": 6 } })
        );
        assert_eq!(
            w[1],
            json!({ "id": "dd-12", "definition": { "type": "note", "content": "On call: #checkout", "background": "yellow", "align": "center", "fontSize": "lg", "pointer": true }, "layout": { "x": 6, "y": 0, "w": 6, "h": 4 } })
        );
        let group = &w[2];
        assert_eq!((group["id"].as_str(), group["definition"]["title"].as_str(), group["definition"]["color"].as_str()), (Some("dd-13"), Some("Latency"), Some("purple")));
        assert_eq!(group["layout"], json!({ "x": 0, "y": 6, "w": 12, "h": 6 }));
        let children = &group["definition"]["widgets"];
        assert_eq!(children[0]["definition"]["type"], "integration");
        assert_eq!(children[0]["definition"]["original"]["requests"][0]["aggregator"], "avg");
        assert_eq!(children[1]["definition"], json!({ "type": "note", "content": "p95", "background": "white", "align": "left", "fontSize": "sm" }));
        assert_eq!(children[1]["layout"], json!({ "x": 4, "y": 0, "w": 8, "h": 4 }));
    }

    #[test]
    fn widgets_without_layout_stack_and_free_layouts_scale() {
        let auto = json!({ "layout_type": "ordered", "widgets": [{ "id": 1, "definition": timeseries("avg:a{*}") }, { "id": 2, "definition": { "type": "note", "content": "n" } }] });
        let w = to_observecraft(&auto)["widgets"].clone();
        assert_eq!((w[0]["layout"].clone(), w[1]["layout"].clone()), (json!({ "x": 0, "y": 0, "w": 12, "h": 6 }), json!({ "x": 0, "y": 6, "w": 12, "h": 3 })));

        let free = json!({ "layout_type": "free", "widgets": [
            { "id": 1, "definition": timeseries("avg:a{*}"), "layout": { "x": 0, "y": 0, "width": 48, "height": 15 } },
            { "id": 2, "definition": { "type": "image", "url": "https://example.com/a.png" }, "layout": { "x": 48, "y": 0, "width": 48, "height": 15 } },
        ] });
        let ours = to_observecraft(&free);
        assert_eq!(ours["layoutType"], "free");
        assert_eq!(ours["widgets"][1]["layout"], json!({ "x": 24, "y": 0, "w": 24, "h": 15 }));
        assert_eq!(ours["widgets"][1]["definition"], json!({ "type": "image", "title": "", "src": "https://example.com/a.png" }));
    }

    #[test]
    fn unchanged_widgets_read_back_verbatim() {
        for doc in [
            ordered(),
            json!({ "layout_type": "ordered", "widgets": [{ "id": 1, "definition": timeseries("avg:a{*}") }, { "id": 2, "definition": { "type": "note", "content": "n" } }] }),
            json!({ "layout_type": "free", "widgets": [{ "id": 7, "definition": timeseries("avg:a{*}"), "layout": { "x": 3, "y": 1, "width": 47, "height": 15 } }] }),
            json!({ "layout_type": "ordered", "widgets": [{ "id": 3, "definition": { "type": "group", "layout_type": "ordered", "banner_img": "https://example.com/b.png", "widgets": [{ "id": 4, "definition": timeseries("avg:b{*}") }] } }] }),
        ] {
            let ours = to_observecraft(&doc);
            let (widgets, fixed) = to_datadog_widgets(items(&ours["widgets"]), Some(&doc), doc["layout_type"] == "free");
            assert_eq!(Value::Array(widgets), doc["widgets"], "{doc}");
            assert!(!fixed);
        }
    }

    #[test]
    fn ui_edits_show_in_the_datadog_widgets() {
        let doc = ordered();
        let mut ours = to_observecraft(&doc)["widgets"].clone();
        // Moved and renamed in the UI.
        ours[0]["layout"] = json!({ "x": 0, "y": 0, "w": 12, "h": 8 });
        ours[0]["definition"]["title"] = json!("CPU by host");
        // A note removed from the group; a widget added in the UI.
        ours[2]["definition"]["widgets"].as_array_mut().unwrap().pop();
        ours.as_array_mut().unwrap().push(json!({ "id": "k3x-9vq", "definition": { "type": "timeseries", "title": "Memory", "requests": [{ "query": "avg:system.mem.used{*}" }], "display": "area" }, "layout": { "x": 0, "y": 20, "w": 6, "h": 4 } }));
        let (widgets, fixed) = to_datadog_widgets(items(&ours), Some(&doc), false);
        assert!(!fixed);
        assert_eq!(widgets.len(), 4);
        assert_eq!(widgets[0]["id"], 11);
        assert_eq!(widgets[0]["layout"], json!({ "x": 0, "y": 0, "width": 12, "height": 4 }));
        assert_eq!(widgets[0]["definition"]["title"], "CPU by host");
        assert_eq!(widgets[0]["definition"]["requests"], doc["widgets"][0]["definition"]["requests"]);
        assert_eq!(widgets[1], doc["widgets"][1]);
        assert_eq!(widgets[2]["definition"]["widgets"], json!([doc["widgets"][2]["definition"]["widgets"][0]]));
        assert_eq!(widgets[2]["definition"]["title"], "Latency");
        let added = &widgets[3];
        assert_eq!(added["id"], hashed_widget_id("k3x-9vq"));
        assert_eq!(added["layout"], json!({ "x": 0, "y": 10, "width": 6, "height": 2 }));
        assert_eq!(
            added["definition"],
            json!({ "type": "timeseries", "title": "Memory", "requests": [{ "response_format": "timeseries", "display_type": "area", "queries": [{ "data_source": "metrics", "name": "query1", "query": "avg:system.mem.used{*}" }], "formulas": [{ "formula": "query1" }] }] })
        );
        assert_eq!(check_definition(&added["definition"]), Ok(()));
        // Stable across reads.
        assert_eq!(to_datadog_widgets(items(&ours), Some(&doc), false).0[3]["id"], added["id"]);
    }

    #[test]
    fn moving_a_widget_of_an_auto_layout_gives_every_widget_a_layout() {
        let doc = json!({ "layout_type": "ordered", "reflow_type": "auto", "widgets": [{ "id": 1, "definition": timeseries("avg:a{*}") }, { "id": 2, "definition": timeseries("avg:b{*}") }] });
        let mut ours = to_observecraft(&doc)["widgets"].clone();
        ours[1]["layout"] = json!({ "x": 6, "y": 0, "w": 6, "h": 6 });
        let (widgets, fixed) = to_datadog_widgets(items(&ours), Some(&doc), false);
        assert!(fixed);
        assert_eq!(widgets[0]["layout"], json!({ "x": 0, "y": 0, "width": 12, "height": 3 }));
        assert_eq!(widgets[1]["layout"], json!({ "x": 6, "y": 0, "width": 6, "height": 3 }));
    }

    #[test]
    fn dashboards_made_in_the_ui_read_as_datadog_widgets() {
        let ours = json!([
            { "id": "a", "definition": { "type": "query_value", "title": "Errors", "request": { "query": "sum:trace.http.request.errors{service:checkout-api}" }, "aggregator": "sum", "precision": 0, "formats": [{ "comparator": ">", "value": 10, "palette": "red" }] }, "layout": { "x": 0, "y": 0, "w": 3, "h": 4 } },
            { "id": "b", "definition": { "type": "note", "content": "hi", "background": "blue", "align": "left", "fontSize": "sm" }, "layout": { "x": 3, "y": 0, "w": 3, "h": 4 } },
            { "id": "c", "definition": { "type": "group", "title": "G", "color": "green", "widgets": [{ "id": "d", "definition": { "type": "toplist", "request": { "query": "avg:system.load.1{*} by {host}" }, "aggregator": "avg", "limit": 5 }, "layout": { "x": 0, "y": 0, "w": 12, "h": 4 } }] }, "layout": { "x": 0, "y": 4, "w": 12, "h": 8 } },
        ]);
        let (widgets, fixed) = to_datadog_widgets(items(&ours), None, false);
        assert!(!fixed);
        for w in &widgets {
            assert_eq!(check_definition(&w["definition"]), Ok(()), "{w}");
        }
        assert_eq!(widgets[0]["definition"]["requests"][0]["conditional_formats"], json!([{ "comparator": ">", "value": 10, "palette": "white_on_red" }]));
        assert_eq!(widgets[1]["definition"], json!({ "type": "note", "content": "hi", "background_color": "blue", "font_size": "14", "text_align": "left" }));
        assert_eq!(widgets[2]["definition"]["background_color"], "green");
        assert_eq!(widgets[2]["definition"]["widgets"][0]["definition"]["requests"][0]["formulas"][0]["limit"], json!({ "count": 5, "order": "desc" }));
        assert_eq!(widgets[2]["layout"], json!({ "x": 0, "y": 2, "width": 12, "height": 4 }));
    }

    #[test]
    fn widget_ids_are_assigned_once() {
        let mut widgets = vec![json!({ "definition": { "type": "group", "widgets": [{ "definition": timeseries("avg:a{*}") }] } }), json!({ "id": 42, "definition": timeseries("avg:b{*}") })];
        assign_widget_ids(&mut widgets);
        let first = widgets[0]["id"].as_i64().unwrap();
        assert!(first > 0 && first < 1 << 52);
        assert!(widgets[0]["definition"]["widgets"][0]["id"].is_i64());
        assert_eq!(widgets[1]["id"], 42);
        assign_widget_ids(&mut widgets);
        assert_eq!(widgets[0]["id"], first);
    }
}
