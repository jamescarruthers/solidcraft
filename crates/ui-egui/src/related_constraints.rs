//! The Related Constraints window: a tree of what constraints tie the selected sketch curves or
//! points to. The selection is the root; under it is each curve or point a constraint ties it to,
//! with those constraints beside it, each with a button that removes it. A row folds open to the
//! curves and points constraints tie that one to in turn (rows start folded; an entity already
//! higher on the branch is left out, so a loop of constraints ends). Hovering a row lights its
//! entity in the sketch, and hovering a constraint lights its glyph; the entity or glyph under the
//! pointer in the sketch lights its row and constraint here. The window stays open and follows
//! the selection. `ui.start` of `sketch.related_constraints` (the SKETCH tab's Constrain menu, or
//! the context menu of a sketch curve or point) opens it; `engine.execute` only returns the
//! JSON. It closes when the sketch is finished.

use std::collections::{BTreeSet, HashMap};

use serde_json::{Value, json};
use solidcraft_engine::Sel;
use solidcraft_engine::sketch::{CurveKind, Sketch};

use crate::SolveApp;
use crate::ui_kit;

/// Deepest a branch unfolds, and most rows the tree lists, so a large sketch stays responsive.
const MAX_DEPTH: usize = 16;
const MAX_ROWS: usize = 400;

/// A constraint as a row shows it: its id and its text ("Coincident", "Dimension 10.00 mm").
#[derive(Clone, Debug, PartialEq)]
pub struct Shown {
    pub id: String,
    pub name: String,
    pub text: String,
}

/// Another curve or point, and the constraints between it and the entities asked about.
#[derive(Clone, Debug, PartialEq)]
pub struct Link {
    pub id: String,
    pub kind: String,
    pub construction: bool,
    pub constraints: Vec<Shown>,
}

/// What `sketch.related_constraints` said about some entities.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Related {
    pub links: Vec<Link>,
    pub own: Vec<Shown>,
}

/// One row of the tree as the window draws it.
#[derive(Clone, Debug, PartialEq)]
pub struct TreeRow {
    pub depth: usize,
    /// The entity ids from the root's child down to this row; empty for the root.
    pub path: Vec<String>,
    /// The entities the row stands for (the root: the selection).
    pub entities: Vec<String>,
    /// The entity's kind (`line`, `point`, …) for its icon; the root's first entity's.
    pub kind: String,
    pub label: String,
    /// The root: the selection's own constraints; any other row: those between it and its parent.
    pub constraints: Vec<Shown>,
    pub foldable: bool,
    pub open: bool,
}

/// What the pointer is over in the window.
#[derive(Clone, Debug, PartialEq)]
pub enum Hover {
    Entity(String),
    /// A constraint, on the row of this entity.
    Constraint {
        id: String,
        entity: String,
    },
}

/// The open window.
pub struct RelatedConstraints {
    /// The sketch the window follows: the one being edited when it read last.
    pub sketch: Option<u64>,
    /// The sketch curves and points it shows: the last selection that held any.
    pub entities: Vec<String>,
    /// Whether the root row is folded open (it starts open).
    pub root_open: bool,
    /// The paths of the other rows folded open (they start folded).
    pub open: BTreeSet<Vec<String>>,
    /// The engine's replies for this revision and sketch, by the entities asked about.
    cache: HashMap<Vec<String>, Result<Related, String>>,
    cache_key: Option<(u64, Option<u64>)>,
    /// The tree as drawn last; `None` while nothing is selected.
    pub rows: Option<Result<Vec<TreeRow>, String>>,
    /// What the pointer was over in the window last frame.
    pub hovered: Option<Hover>,
}

/// The selected curves and points of the sketch being edited, in selection order.
pub fn selected_entities(app: &SolveApp) -> Vec<String> {
    let active = app.session.active_sketch;
    let mut out: Vec<String> = Vec::new();
    for sel in &app.session.selection {
        let id = match sel {
            Sel::SketchCurve { id, sketch } if sketch.is_none() || *sketch == active => id,
            Sel::SketchPoint { id } => id,
            _ => continue,
        };
        if !out.contains(id) {
            out.push(id.clone());
        }
    }
    out
}

/// Whether a constraint of `sketch` names one of `entities`, or a point of one of its curves:
/// whether the window would show anything for them.
pub fn has_constraints(sketch: &Sketch, entities: &[String]) -> bool {
    let curves: Vec<usize> = entities.iter().filter_map(|e| sketch.curve_index(e)).collect();
    let mut points: Vec<usize> = entities.iter().filter(|e| sketch.curve_index(e).is_none()).filter_map(|e| sketch.resolve_point(e)).collect();
    points.extend(curves.iter().flat_map(|&c| sketch.curves.get(c).map(|x| x.kind.point_ids()).unwrap_or_default()));
    sketch.constraints.iter().any(|c| {
        let (pts, crvs) = c.kind.indices();
        crvs.iter().any(|i| curves.contains(i)) || pts.iter().any(|p| points.contains(p))
    })
}

/// Open the window on the selection.
pub fn open(app: &mut SolveApp) {
    let entities = selected_entities(app);
    app.related_constraints = Some(RelatedConstraints {
        sketch: app.session.active_sketch,
        entities,
        root_open: true,
        open: BTreeSet::new(),
        cache: HashMap::new(),
        cache_key: None,
        rows: None,
        hovered: None,
    });
}

/// The constraint under the pointer in the window, for the sketch to light its glyph.
pub fn hovered_constraint(app: &SolveApp) -> Option<String> {
    match app.related_constraints.as_ref()?.hovered.as_ref()? {
        Hover::Constraint { id, .. } => Some(id.clone()),
        Hover::Entity(_) => None,
    }
}

/// The entity of the row under the pointer in the window is the sketch's hover, as if the
/// pointer were over it there. Called by the viewport after it works out its own hover.
pub(crate) fn hover_override(app: &mut SolveApp) {
    let id = match app.related_constraints.as_ref().and_then(|w| w.hovered.as_ref()) {
        Some(Hover::Entity(id) | Hover::Constraint { entity: id, .. }) => id.clone(),
        None => return,
    };
    let st = crate::open_meshing::session_world(&app.session);
    let Some(ss) = app.session.active_sketch.and_then(|s| st.sketch(s)) else { return };
    let sel = if ss.sketch.curve_index(&id).is_some() {
        Sel::SketchCurve { id, sketch: None }
    } else if ss.sketch.resolve_point(&id).is_some() {
        Sel::SketchPoint { id }
    } else {
        return;
    };
    app.viewport.hover = crate::viewport::sel_hit(app, &sel);
}

/// The entity under the pointer in the sketch, if it is a curve or point of the sketch.
fn sketch_hover(app: &SolveApp) -> Option<String> {
    match app.viewport.hover.as_ref()? {
        crate::viewport::Hit::SketchCurve { id, .. } | crate::viewport::Hit::SketchPoint { id, .. } => Some(id.clone()),
        _ => None,
    }
}

/// The message id of a curve kind as `sketch.related_constraints` names it.
fn type_text(kind: &str) -> String {
    let id = match kind {
        "line" | "circle" | "arc" | "ellipse" | "spline" | "conic" | "point" => format!("related_constraints.type.{kind}"),
        _ => return kind.to_string(),
    };
    solidcraft_ui_text::tr(&id).to_string()
}

/// "Line l1": an entity as the window names it.
fn entity_text(kind: &str, id: &str, construction: bool) -> String {
    let id_key = if construction { "related_constraints.entity_construction" } else { "related_constraints.entity" };
    solidcraft_ui_text::format(id_key, &[("type", &type_text(kind)), ("id", id)])
}

/// The icon of an entity kind.
fn kind_icon(kind: &str) -> &'static str {
    match kind {
        "line" => "line",
        "circle" => "circle",
        "arc" => "arc3",
        "ellipse" => "ellipse",
        "spline" => "spline",
        "conic" => "conic",
        _ => "point",
    }
}

/// The icon of a constraint, by its kind's name.
fn constraint_icon(name: &str, dimension: bool) -> Option<&'static str> {
    if dimension {
        return Some("dimension");
    }
    match name {
        "Horizontal" | "Vertical" => Some("c_hv"),
        _ => crate::icons::markers::glyph_icon(name),
    }
}

/// The kind of a sketch entity of `sketch`, as `sketch.related_constraints` names it.
fn entity_kind(sketch: &Sketch, id: &str) -> (&'static str, bool) {
    match sketch.curve_index(id).and_then(|i| sketch.curves.get(i)) {
        Some(c) => (
            match c.kind {
                CurveKind::Line { .. } => "line",
                CurveKind::Circle { .. } => "circle",
                CurveKind::Arc { .. } => "arc",
                CurveKind::Ellipse { .. } => "ellipse",
                CurveKind::Spline { .. } => "spline",
                CurveKind::Conic { .. } => "conic",
            },
            c.construction,
        ),
        None => ("point", false),
    }
}

/// A constraint's text: its kind, and a dimension's value as the view draws it.
fn constraint_text(app: &SolveApp, sketch: &Sketch, id: &str, name: &str) -> String {
    if let Some(c) = sketch.constraints.iter().find(|c| c.id == id)
        && c.kind.is_dimension()
    {
        let expr = c.param.as_ref().and_then(|p| app.session.doc.param(p)).map(|p| p.expr.as_str());
        let value = crate::dim_view::label(&c.kind, expr, c.driven, &app.session.doc.units, app.preferences.length_decimals);
        return solidcraft_ui_text::format("related_constraints.kind.dimension", &[("value", &value)]);
    }
    let key = match name {
        "Coincident" => "coincident",
        "PointAlong" => "point_along",
        "Horizontal" => "horizontal",
        "Vertical" => "vertical",
        "Parallel" => "parallel",
        "Perpendicular" => "perpendicular",
        "Collinear" => "collinear",
        "Tangent" => "tangent",
        "Smooth" => "curvature",
        "Equal" => "equal",
        "Concentric" => "concentric",
        "MidPoint" => "midpoint",
        "Symmetry" => "symmetry",
        "Fix" => "fix",
        "Pierce" => "pierce",
        "Pattern" => "pattern",
        _ => return name.to_string(),
    };
    solidcraft_ui_text::tr(&format!("related_constraints.kind.{key}")).to_string()
}

/// The constraints of one list of the reply, as rows show them.
fn shown(app: &SolveApp, sketch: &Sketch, list: &Value) -> Vec<Shown> {
    list.as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| {
            let id = c["id"].as_str()?;
            let name = c["name"].as_str().unwrap_or_default();
            let dimension = c["dimension"].as_bool().unwrap_or(false);
            let text = constraint_text(app, sketch, id, name);
            Some(Shown { id: id.to_string(), name: if dimension { "Dimension".to_string() } else { name.to_string() }, text })
        })
        .collect()
}

/// Ask the engine about `entities` of `sketch`.
fn query(app: &mut SolveApp, sketch: u64, entities: &[String]) -> Result<Related, String> {
    let reply = app.session.execute("sketch.related_constraints", &json!({ "sketch": sketch, "entities": entities })).map_err(|e| e.to_string())?;
    let st = crate::open_meshing::session_world(&app.session);
    let ss = st.sketch(sketch).ok_or_else(|| solidcraft_ui_text::tr("related_constraints.pick").to_string())?;
    let links = reply["related"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|e| Link {
            id: e["id"].as_str().unwrap_or_default().to_string(),
            kind: e["type"].as_str().unwrap_or_default().to_string(),
            construction: e["construction"].as_bool().unwrap_or(false),
            constraints: shown(app, &ss.sketch, &e["constraints"]),
        })
        .collect();
    Ok(Related { links, own: shown(app, &ss.sketch, &reply["own"]) })
}

/// The reply about `entities`, asked once per revision.
fn related(app: &mut SolveApp, w: &mut RelatedConstraints, sketch: u64, entities: &[String]) -> Result<Related, String> {
    if let Some(r) = w.cache.get(entities) {
        return r.clone();
    }
    let r = query(app, sketch, entities);
    w.cache.insert(entities.to_vec(), r.clone());
    r
}

/// The rows under the row at `path` (whose own entities and ancestors are `above`), into `out`.
fn branch(app: &mut SolveApp, w: &mut RelatedConstraints, sketch: u64, links: Vec<Link>, path: &[String], above: &[String], out: &mut Vec<TreeRow>) {
    for link in links.into_iter().filter(|l| !above.contains(&l.id)) {
        if out.len() >= MAX_ROWS {
            return;
        }
        let mut child_path = path.to_vec();
        child_path.push(link.id.clone());
        let mut child_above = above.to_vec();
        child_above.push(link.id.clone());
        let children: Vec<Link> = if child_path.len() < MAX_DEPTH {
            related(app, w, sketch, std::slice::from_ref(&link.id))
                .map(|r| r.links.into_iter().filter(|l| !child_above.contains(&l.id)).collect())
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let open = !children.is_empty() && w.open.contains(&child_path);
        out.push(TreeRow {
            depth: child_path.len(),
            path: child_path.clone(),
            entities: vec![link.id.clone()],
            kind: link.kind.clone(),
            label: entity_text(&link.kind, &link.id, link.construction),
            constraints: link.constraints,
            foldable: !children.is_empty(),
            open,
        });
        if open {
            branch(app, w, sketch, children, &child_path, &child_above, out);
        }
    }
}

/// The tree for the entities the window follows, rows in drawing order.
fn tree(app: &mut SolveApp, w: &mut RelatedConstraints, sketch: u64) -> Result<Vec<TreeRow>, String> {
    let entities = w.entities.clone();
    let root = related(app, w, sketch, &entities)?;
    let st = crate::open_meshing::session_world(&app.session);
    let ss = st.sketch(sketch).ok_or_else(|| solidcraft_ui_text::tr("related_constraints.pick").to_string())?;
    let label = entities
        .iter()
        .map(|id| {
            let (kind, construction) = entity_kind(&ss.sketch, id);
            entity_text(kind, id, construction)
        })
        .collect::<Vec<_>>()
        .join(", ");
    let kind = entities.first().map_or("point", |id| entity_kind(&ss.sketch, id).0).to_string();
    let mut out = vec![TreeRow {
        depth: 0,
        path: Vec::new(),
        entities: entities.clone(),
        kind,
        label,
        constraints: root.own,
        foldable: !root.links.is_empty(),
        open: w.root_open,
    }];
    if w.root_open {
        branch(app, w, sketch, root.links, &[], &entities, &mut out);
    }
    Ok(out)
}

/// Keep the window on the selection and the design: a new selection of sketch curves or points
/// becomes the root, and an edit reads the sketch again. A selection of nothing else keeps it.
pub fn refresh(app: &mut SolveApp, w: &mut RelatedConstraints) {
    let active = app.session.active_sketch;
    if w.sketch != active {
        w.sketch = active;
        w.entities.clear();
    }
    let selected = selected_entities(app);
    if !selected.is_empty() && selected != w.entities {
        w.entities = selected;
        w.root_open = true;
        w.open.clear();
    }
    // Forget entities the sketch no longer has (deleted, or undone).
    let st = crate::open_meshing::session_world(&app.session);
    if let Some(ss) = active.and_then(|s| st.sketch(s)) {
        w.entities.retain(|e| ss.sketch.curve_index(e).is_some() || ss.sketch.resolve_point(e).is_some());
    }
    let key = (app.session.revision, w.sketch);
    if w.cache_key != Some(key) {
        w.cache.clear();
        w.cache_key = Some(key);
    }
    w.rows = match w.sketch {
        Some(sketch) if !w.entities.is_empty() => Some(tree(app, w, sketch)),
        _ => None,
    };
}

/// What a click in one row asked for.
#[derive(Default)]
struct RowOut {
    fold: bool,
    remove: Option<String>,
    hovered: Option<Hover>,
}

/// A stable key for a row's handles: its path, or `root`.
fn row_key(row: &TreeRow) -> String {
    if row.path.is_empty() { "root".to_string() } else { row.path.join("/") }
}

/// Draw one row: fold arrow, icon, the entity's name, then each constraint with its remove
/// button, wrapping onto more lines when they do not fit. `lit` marks the row of the entity
/// under the pointer in the sketch, `lit_constraint` the constraint whose glyph is.
fn draw_row(ui: &mut egui::Ui, row: &TreeRow, lit: bool, lit_constraint: Option<&str>) -> RowOut {
    let t = solidcraft_theme::Tokens::get();
    let h = ui_kit::row_height();
    let key = row_key(row);
    let font = if row.depth == 0 { ui_kit::type_face::semibold(t.font_label) } else { egui::FontId::proportional(t.font_label) };
    let small = egui::FontId::proportional(t.font_label);
    let width = ui.available_width();
    let left = ui.cursor().min.x;
    let fold_x = left + t.space_4 + row.depth as f32 * t.space_16;
    let icon_x = fold_x + t.space_16;
    let name_x = icon_x + t.space_16 + t.space_6;
    let name = ui.painter().layout_no_wrap(row.label.clone(), font, t.text);
    let remove_w = h * 0.8;
    // Each constraint: (icon, text galley, width).
    let chips: Vec<_> = row
        .constraints
        .iter()
        .map(|c| {
            let galley = ui.painter().layout_no_wrap(c.text.clone(), small.clone(), t.text_dim);
            let icon = constraint_icon(&c.name, c.name == "Dimension");
            let w = icon.map_or(0.0, |_| t.space_16 + t.space_4) + t.space_6 + galley.size().x + t.space_4 + remove_w;
            (icon, galley, w)
        })
        .collect();
    let right = left + width - t.space_4;
    let mut x = name_x + name.size().x + t.space_16;
    let mut line = 0usize;
    let mut places = Vec::with_capacity(chips.len());
    for (_, _, w) in &chips {
        if x + w > right && x > name_x {
            line += 1;
            x = name_x;
        }
        places.push((line, x));
        x += w + t.space_8;
    }
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, h * (line + 1) as f32), egui::Sense::hover());
    solidcraft_ui_registry::publish_rect(&format!("related:row:{key}"), rect);
    let mut out = RowOut::default();
    let over_row = ui.rect_contains_pointer(rect);
    if over_row || lit {
        ui.painter().rect_filled(rect, t.radius_sm.saturating_sub(1), t.hover);
    }
    if lit {
        let bar = egui::Rect::from_min_size(rect.left_top() + egui::vec2(0.0, t.space_2), egui::vec2(3.0, rect.height() - 4.0));
        ui.painter().rect_filled(bar, t.radius_sm / 4, t.accent);
    }
    let cy = rect.top() + h / 2.0;
    if row.foldable {
        let fold = egui::Rect::from_center_size(egui::pos2(fold_x + t.space_6, cy), egui::vec2(t.space_16, h));
        let resp = ui.interact(fold, ui.scope_id().with(("related_fold", &key)), egui::Sense::click());
        ui_kit::disclosure(ui.painter(), fold.center(), row.open, if resp.hovered() { t.text } else { t.text_dim });
        out.fold = resp.clicked();
        solidcraft_ui_registry::publish_handle(&format!("related:fold:{key}"), fold.center());
    }
    let icon = egui::Rect::from_min_size(egui::pos2(icon_x, cy - t.space_8), egui::vec2(t.space_16, t.space_16));
    crate::icons::paint_icon(ui.painter(), icon, kind_icon(&row.kind), 1.0);
    ui.painter().galley(egui::pos2(name_x, cy - name.size().y / 2.0), name, t.text);
    let mut over_chip = None;
    for (c, ((icon, galley, w), (line, x))) in row.constraints.iter().zip(chips.into_iter().zip(places)) {
        let top = rect.top() + line as f32 * h;
        let chip = egui::Rect::from_min_size(egui::pos2(x, top + t.space_3), egui::vec2(w, h - t.space_6));
        let hot = ui.rect_contains_pointer(chip) || lit_constraint == Some(c.id.as_str());
        if hot {
            ui.painter().rect_filled(chip, t.radius_sm, t.accent_soft);
        }
        if ui.rect_contains_pointer(chip) {
            over_chip = Some(c.id.clone());
        }
        let cy = chip.center().y;
        let mut cx = chip.left() + t.space_6;
        if let Some(icon) = icon {
            crate::icons::paint_icon(
                ui.painter(),
                egui::Rect::from_min_size(egui::pos2(cx, cy - t.space_8), egui::vec2(t.space_16, t.space_16)),
                icon,
                1.0,
            );
            cx += t.space_16 + t.space_4;
        }
        let ink = if hot { t.text } else { t.text_dim };
        ui.painter().galley(egui::pos2(cx, cy - galley.size().y / 2.0), galley, ink);
        let remove = egui::Rect::from_center_size(egui::pos2(chip.right() - remove_w / 2.0, cy), egui::vec2(remove_w, remove_w));
        let label = solidcraft_ui_text::format("related_constraints.remove", &[("constraint", &c.text)]);
        let clicked = ui.push_id(("related_remove", &key, &c.id), |ui| ui_kit::quiet_icon_button_at(ui, remove, "close", &label)).inner.clicked();
        if clicked {
            out.remove = Some(c.id.clone());
        }
        solidcraft_ui_registry::publish_handle(&format!("related:remove:{key}:{}", c.id), remove.center());
        solidcraft_ui_registry::publish_rect(&format!("related:constraint:{key}:{}", c.id), chip);
    }
    // The root is the selection, lit already: hovering it lights nothing more.
    let entity = row.entities.first().filter(|_| row.depth > 0);
    out.hovered = match (over_chip, entity) {
        (Some(id), Some(entity)) => Some(Hover::Constraint { id, entity: entity.clone() }),
        (Some(id), None) => row.entities.first().map(|entity| Hover::Constraint { id, entity: entity.clone() }),
        (None, Some(entity)) if over_row => Some(Hover::Entity(entity.clone())),
        _ => None,
    };
    out
}

/// Draw the window when it is open. Called once per frame.
pub fn show(app: &mut SolveApp, ctx: &egui::Context) {
    let Some(mut w) = app.related_constraints.take() else { return };
    // Finishing or cancelling the sketch closes it.
    if app.session.active_sketch.is_none() {
        return;
    }
    refresh(app, &mut w);
    let t = solidcraft_theme::Tokens::get();
    // The bottom right of the view, left of the sketch palette so the palette's Finish Sketch
    // stays in reach. It opens there; it can then be moved.
    let view = app.viewport.rect.unwrap_or_else(|| ctx.content_rect());
    let right = app.viewport.palette_rect.map_or(view.right() - t.space_16, |p| p.left().min(view.right()) - t.space_8);
    let corner = egui::pos2(right, view.bottom() - t.space_16);
    let title = solidcraft_ui_text::tr("related_constraints.title");
    let lit = sketch_hover(app);
    let lit_constraint = crate::sketch_tools::hovered_glyph();
    let mut remove: Option<String> = None;
    let mut fold: Option<Vec<String>> = None;
    let mut hovered: Option<Hover> = None;
    let (closed, action) = ui_kit::Window::new(egui::Id::unique("sc_related_constraints"), "related_constraints", title)
        .placed(egui::Align2::RIGHT_BOTTOM, corner)
        // Wide enough for an entity and two or three constraints on one line.
        .cards(2.0)
        .fields()
        .actions("", solidcraft_ui_text::tr("ui.close"), None)
        .show(ctx, true, |ui| match &w.rows {
            None => {
                ui_kit::status_line(ui, solidcraft_ui_text::tr("related_constraints.pick"), ui_kit::StatusTone::Dim);
            }
            Some(Err(e)) => {
                ui_kit::text(ui, e, ui_kit::Text::Error("error"));
            }
            Some(Ok(rows)) => {
                ui.spacing_mut().item_spacing.y = 0.0;
                for row in rows {
                    let lit = row.depth > 0 && lit.as_ref().is_some_and(|id| row.entities.contains(id));
                    let out = draw_row(ui, row, lit, lit_constraint.as_deref());
                    if out.fold {
                        fold = Some(row.path.clone());
                    }
                    if out.remove.is_some() {
                        remove = out.remove;
                    }
                    if out.hovered.is_some() {
                        hovered = out.hovered;
                    }
                }
                if rows.len() == 1 && rows.first().is_some_and(|r| r.constraints.is_empty() && !r.foldable) {
                    ui_kit::status_line(ui, solidcraft_ui_text::tr("related_constraints.none"), ui_kit::StatusTone::Dim);
                }
            }
        });
    w.hovered = hovered;
    if let Some(path) = fold {
        if path.is_empty() {
            w.root_open = !w.root_open;
        } else if !w.open.remove(&path) {
            w.open.insert(path);
        }
    }
    if let Some(id) = remove
        && let Err(e) = app.run("sketch.delete", json!({ "entities": [id] }))
    {
        app.set_status(crate::friendly_error("sketch.delete", &e), true);
    }
    if closed || action != ui_kit::Action::None {
        return;
    }
    app.related_constraints = Some(w);
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{Hover, TreeRow};
    use crate::scenario::Harness;
    use solidcraft_engine::Sel;

    /// Three lines in a chain: the first horizontal and perpendicular to the second, the second
    /// parallel to the third, and a free fourth line: (first, second, third, free).
    fn chain(h: &mut Harness) -> (String, String, String, String) {
        h.app.home.open = false;
        h.app.run("sketch.create", json!({"plane": "XY"})).unwrap();
        let id = |v: serde_json::Value| v["curves"][0].as_str().map(str::to_string).unwrap();
        let a = id(h.app.run("sketch.line", json!({"points": [[0, 0], [10, 0]]})).unwrap());
        let b = id(h.app.run("sketch.line", json!({"points": [[12, 2], [12, 10]]})).unwrap());
        let c = id(h.app.run("sketch.line", json!({"points": [[20, 2], [20, 10]]})).unwrap());
        let free = id(h.app.run("sketch.line", json!({"points": [[40, 0], [50, 5]]})).unwrap());
        h.app.run("sketch.constraint.perpendicular", json!({"a": a, "b": b})).unwrap();
        h.app.run("sketch.constraint.parallel", json!({"a": b, "b": c})).unwrap();
        h.app.run("sketch.constraint.horizontal_vertical", json!({"line": a})).unwrap();
        (a, b, c, free)
    }

    fn select(h: &mut Harness, id: &str) {
        h.app.session.selection = vec![Sel::SketchCurve { id: id.to_string(), sketch: None }];
    }

    fn rows(h: &Harness) -> Vec<TreeRow> {
        h.app.related_constraints.as_ref().unwrap().rows.clone().unwrap().unwrap()
    }

    fn click(h: &mut Harness, handle: &str) {
        let at = crate::scenario::handle_at(handle).unwrap_or_else(|| panic!("no handle {handle}"));
        h.step(&json!({"click": [at.x, at.y]})).unwrap();
        h.frames(2);
    }

    #[test]
    fn the_tree_has_the_selection_at_its_root_and_folds_open_one_level_at_a_time() {
        let mut h = Harness::new();
        let (a, b, c, free) = chain(&mut h);
        select(&mut h, &a);
        h.step(&json!({"start": "sketch.related_constraints"})).unwrap();
        h.frames(3);
        let tree = rows(&h);
        assert_eq!(tree[0].depth, 0);
        assert!(tree[0].label.contains(&a), "{tree:?}");
        assert!(tree[0].open, "the root starts open");
        assert_eq!(tree[0].constraints.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["Horizontal"], "the line's own constraint");
        let second = tree.iter().find(|r| r.path == [b.clone()]).expect("the second line is under the root");
        assert!(second.constraints.iter().any(|k| k.name == "Perpendicular"), "{second:?}");
        assert!(second.foldable && !second.open, "the second line leads on to the third, folded");
        assert!(!tree.iter().any(|r| r.entities.contains(&c)), "the third line is one level further down");
        // Unfold the second line: the third line appears under it, the first is not repeated.
        click(&mut h, &format!("related:fold:{b}"));
        let tree = rows(&h);
        let third = tree.iter().find(|r| r.path == [b.clone(), c.clone()]).expect("the third line under the second");
        assert_eq!(third.depth, 2);
        assert!(third.constraints.iter().any(|k| k.name == "Parallel"), "{third:?}");
        assert!(!tree.iter().skip(1).any(|r| r.entities.contains(&a)), "an entity higher on the branch is left out: {tree:?}");
        // Fold the root: only the root is left.
        click(&mut h, "related:fold:root");
        assert_eq!(rows(&h).len(), 1);
        // A click on the free line makes it the root; nothing ties it, so the tree says so.
        select(&mut h, &free);
        h.frames(2);
        let tree = rows(&h);
        assert_eq!(tree.len(), 1);
        assert!(tree[0].label.contains(&free) && tree[0].constraints.is_empty() && !tree[0].foldable);
        // A click on empty space keeps the root.
        h.app.session.selection.clear();
        h.frames(2);
        assert_eq!(h.app.related_constraints.as_ref().unwrap().entities, vec![free.clone()]);
        // Finishing the sketch closes the window.
        h.app.run("sketch.finish", json!({})).unwrap();
        h.frames(2);
        assert!(h.app.related_constraints.is_none());
    }

    #[test]
    fn a_constraint_is_removed_from_its_row() {
        let mut h = Harness::new();
        let (a, b, _, _) = chain(&mut h);
        select(&mut h, &a);
        h.step(&json!({"start": "sketch.related_constraints"})).unwrap();
        h.frames(3);
        let second = rows(&h).into_iter().find(|r| r.path == [b.clone()]).unwrap();
        let perpendicular = second.constraints.iter().find(|k| k.name == "Perpendicular").unwrap().id.clone();
        let undo = h.app.session.undo.len();
        click(&mut h, &format!("related:remove:{b}:{perpendicular}"));
        h.frames(1);
        let st = crate::open_meshing::session_world(&h.app.session);
        let sk = &st.sketch(h.app.session.active_sketch.unwrap()).unwrap().sketch;
        assert!(!sk.constraints.iter().any(|k| k.id == perpendicular), "the constraint is gone");
        assert_eq!(h.app.session.undo.len(), undo + 1, "one undo step");
        let tree = rows(&h);
        assert!(!tree.iter().flat_map(|r| &r.constraints).any(|k| k.id == perpendicular), "the tree reads the sketch again: {tree:?}");
    }

    #[test]
    fn hovering_lights_the_other_side() {
        let mut h = Harness::new();
        let (a, b, _, _) = chain(&mut h);
        select(&mut h, &a);
        h.step(&json!({"start": "sketch.related_constraints"})).unwrap();
        h.frames(3);
        // The pointer on the second line's row lights the second line in the sketch.
        let row = crate::scenario::rect_of(&format!("related:row:{b}")).expect("the row is drawn");
        h.step(&json!({"move": [row.left() + 40.0, row.center().y]})).unwrap();
        h.frames(2);
        assert_eq!(h.app.related_constraints.as_ref().unwrap().hovered, Some(Hover::Entity(b.clone())));
        assert!(
            matches!(h.app.viewport.hover.as_ref(), Some(crate::viewport::Hit::SketchCurve { id, .. }) if *id == b),
            "the sketch lights the row's line: {:?}",
            h.app.viewport.hover
        );
        // The pointer on its constraint lights that constraint's glyph too.
        let second = rows(&h).into_iter().find(|r| r.path == [b.clone()]).unwrap();
        let perpendicular = second.constraints.iter().find(|k| k.name == "Perpendicular").unwrap().id.clone();
        let chip = crate::scenario::rect_of(&format!("related:constraint:{b}:{perpendicular}")).unwrap();
        h.step(&json!({"move": [chip.left() + 4.0, chip.center().y]})).unwrap();
        h.frames(2);
        assert_eq!(super::hovered_constraint(&h.app), Some(perpendicular));
        // Off the window, nothing is lit from it.
        h.step(&json!({"move": [5.0, 500.0]})).unwrap();
        h.frames(2);
        assert_eq!(h.app.related_constraints.as_ref().unwrap().hovered, None);
    }

    #[test]
    fn the_context_menu_offers_it_only_for_constrained_entities() {
        let mut h = Harness::new();
        let (a, _, _, free) = chain(&mut h);
        let offered = |h: &Harness| {
            crate::context_menu::items(&h.app, &crate::context_menu::Target::Viewport).iter().any(|i| i.id == "sketch.related_constraints")
        };
        select(&mut h, &a);
        assert!(offered(&h), "a constrained line offers the item");
        select(&mut h, &free);
        assert!(!offered(&h), "a free line does not");
        // The item opens the window, as the ribbon's does.
        select(&mut h, &a);
        let item = crate::context_menu::items(&h.app, &crate::context_menu::Target::Viewport)
            .into_iter()
            .find(|i| i.id == "sketch.related_constraints")
            .unwrap();
        crate::context_menu::run_item(&mut h.app, &item, egui::pos2(100.0, 100.0));
        h.frames(2);
        assert!(h.app.related_constraints.is_some());
    }

    /// With `SOLIDCRAFT_RELATED_CONSTRAINTS_EVIDENCE` set to a folder, draw the window and the
    /// context menu as the app paints them (CPU raster, no GPU viewport) into PNGs there.
    #[test]
    fn evidence_pictures() {
        let Ok(dir) = std::env::var("SOLIDCRAFT_RELATED_CONSTRAINTS_EVIDENCE") else { return };
        let mut h = Harness::new();
        let (a, b, _, _) = chain(&mut h);
        h.app.run("sketch.dimension", json!({"entities": [a], "value": 10})).unwrap();
        select(&mut h, &a);
        let size = egui::vec2(1600.0, 1000.0);
        let shoot = |h: &mut Harness, name: &str, before: &dyn Fn(&mut Harness)| {
            h.ctx = egui::Context::default();
            crate::fonts::setup_bundled_fonts(&h.ctx);
            let mut textures = std::collections::HashMap::new();
            for frame in 0..8 {
                if frame == 3 {
                    before(h);
                }
                let mut output = h.ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                        time: Some(f64::from(frame) / 60.0),
                        ..Default::default()
                    },
                    |ui| {
                        h.app.logic(ui.ctx());
                        h.app.ui(ui);
                    },
                );
                crate::ui_kit::test_textures_update(&mut textures, &output.textures_delta);
                output.textures_delta.clear();
                if frame == 7 {
                    let bounds = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
                    let image = crate::ui_kit::test_raster(&h.ctx, output.shapes, output.pixels_per_point, bounds, &textures);
                    std::fs::create_dir_all(&dir).unwrap();
                    image.save(std::path::Path::new(&dir).join(name)).unwrap();
                }
            }
        };
        shoot(&mut h, "context-menu.png", &|h| {
            crate::context_menu::open_for(&mut h.app, egui::pos2(700.0, 450.0), crate::context_menu::Target::Viewport);
        });
        crate::context_menu::close(&mut h.app);
        h.app.start("sketch.related_constraints");
        let open = vec![b.clone()];
        shoot(&mut h, "window.png", &move |h| {
            if let Some(w) = h.app.related_constraints.as_mut() {
                w.open.insert(open.clone());
            }
        });
    }

    #[test]
    fn the_command_is_in_the_constrain_menu() {
        let spec = solidcraft_engine::find_command("sketch.related_constraints").unwrap();
        assert_eq!((spec.tab, spec.panel), ("SKETCH", "CONSTRAINTS"));
        let specs = solidcraft_engine::command_specs();
        let (commands, promoted) = crate::workspace::layout("SKETCH", "CONSTRAIN", &specs).unwrap();
        let at = commands.iter().position(|c| c.id == "sketch.related_constraints").expect("listed in the Constrain menu");
        assert!(at >= promoted, "in the drop-down, not a ribbon button");
    }
}
