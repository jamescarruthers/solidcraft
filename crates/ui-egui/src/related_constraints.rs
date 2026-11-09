//! The Related Constraints window: for the selected sketch curves or points, the other curves and
//! points of the sketch that constraints tie them to, each with those constraints and a button
//! that removes one. The window stays open and follows the selection, so a click on another
//! curve shows that curve's constraints. `ui.start` of `sketch.related_constraints` (the SKETCH
//! tab's Constrain menu, or the context menu of a sketch curve or point) opens it;
//! `engine.execute` only returns the JSON. It closes when the sketch is finished.

use serde_json::{Value, json};
use solidcraft_engine::Sel;
use solidcraft_engine::sketch::{CurveKind, Sketch};

use crate::SolveApp;
use crate::ui_kit;

/// One block of the window: a related curve or point (or the selection itself), and the
/// constraints between it and the selection as (constraint id, text shown).
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub title: String,
    pub rows: Vec<(String, String)>,
}

/// What the window shows for the entities it follows.
#[derive(Clone, Debug, PartialEq)]
pub struct View {
    /// The selected entities, as people read them ("Line l1").
    pub subject: String,
    /// The related curves and points in the sketch's order, then the constraints on the
    /// selection alone.
    pub groups: Vec<Group>,
}

/// The open window.
pub struct RelatedConstraints {
    /// The sketch the window follows: the one being edited when it read last.
    pub sketch: Option<u64>,
    /// The sketch curves and points it shows: the last selection that held any.
    pub entities: Vec<String>,
    /// The session revision, sketch and entities `view` was read for.
    read: Option<(u64, Option<u64>, Vec<String>)>,
    /// `None` while nothing is selected.
    pub view: Option<Result<View, String>>,
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
    app.related_constraints = Some(RelatedConstraints { sketch: app.session.active_sketch, entities, read: None, view: None });
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

/// The rows of one constraint list of the reply.
fn rows(app: &SolveApp, sketch: &Sketch, list: &Value) -> Vec<(String, String)> {
    list.as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| {
            let id = c["id"].as_str()?;
            Some((id.to_string(), constraint_text(app, sketch, id, c["name"].as_str().unwrap_or_default())))
        })
        .collect()
}

/// Ask the engine about `entities` of `sketch`, and turn the reply into what the window shows.
fn read(app: &mut SolveApp, sketch: u64, entities: &[String]) -> Result<View, String> {
    let reply = app.session.execute("sketch.related_constraints", &json!({ "sketch": sketch, "entities": entities })).map_err(|e| e.to_string())?;
    let st = crate::open_meshing::session_world(&app.session);
    let ss = st.sketch(sketch).ok_or_else(|| solidcraft_ui_text::tr("related_constraints.pick").to_string())?;
    let sk = &ss.sketch;
    let subject = entities
        .iter()
        .map(|id| {
            let (kind, construction) = entity_kind(sk, id);
            entity_text(kind, id, construction)
        })
        .collect::<Vec<_>>()
        .join(", ");
    let mut groups: Vec<Group> = reply["related"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|e| Group {
            title: entity_text(
                e["type"].as_str().unwrap_or_default(),
                e["id"].as_str().unwrap_or_default(),
                e["construction"].as_bool().unwrap_or(false),
            ),
            rows: rows(app, sk, &e["constraints"]),
        })
        .collect();
    let own = rows(app, sk, &reply["own"]);
    if !own.is_empty() {
        groups.push(Group { title: solidcraft_ui_text::tr("related_constraints.own").to_string(), rows: own });
    }
    Ok(View { subject, groups })
}

/// Keep the window on the selection and the design: a new selection of sketch curves or points
/// replaces what it shows, and an edit reads it again. A selection of nothing else keeps it.
pub fn refresh(app: &mut SolveApp, w: &mut RelatedConstraints) {
    let active = app.session.active_sketch;
    if w.sketch != active {
        w.sketch = active;
        w.entities.clear();
    }
    let selected = selected_entities(app);
    if !selected.is_empty() {
        w.entities = selected;
    }
    // Forget entities the sketch no longer has (deleted, or undone).
    let st = crate::open_meshing::session_world(&app.session);
    if let Some(ss) = active.and_then(|s| st.sketch(s)) {
        w.entities.retain(|e| ss.sketch.curve_index(e).is_some() || ss.sketch.resolve_point(e).is_some());
    }
    let key = (app.session.revision, w.sketch, w.entities.clone());
    if w.read.as_ref() == Some(&key) {
        return;
    }
    w.view = match w.sketch {
        Some(sketch) if !w.entities.is_empty() => Some(read(app, sketch, &w.entities.clone())),
        _ => None,
    };
    w.read = Some(key);
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
    let corner = app.viewport.rect.unwrap_or_else(|| ctx.content_rect()).right_bottom() - egui::vec2(t.space_16, t.space_16);
    let title = solidcraft_ui_text::tr("related_constraints.title");
    let mut remove: Option<String> = None;
    let (closed, action) = ui_kit::Window::new(egui::Id::unique("sc_related_constraints"), "related_constraints", title)
        .placed(egui::Align2::RIGHT_BOTTOM, corner)
        .fields()
        .actions("", solidcraft_ui_text::tr("ui.close"), None)
        .show(ctx, true, |ui| match &w.view {
            None => {
                ui_kit::status_line(ui, solidcraft_ui_text::tr("related_constraints.pick"), ui_kit::StatusTone::Dim);
            }
            Some(Err(e)) => {
                ui_kit::text(ui, e, ui_kit::Text::Error("error"));
            }
            Some(Ok(view)) => {
                ui_kit::row(ui, solidcraft_ui_text::tr("related_constraints.selected"), |ui| ui_kit::text(ui, &view.subject, ui_kit::Text::Cell));
                if view.groups.is_empty() {
                    ui_kit::status_line(ui, solidcraft_ui_text::tr("related_constraints.none"), ui_kit::StatusTone::Dim);
                }
                for (i, group) in view.groups.iter().enumerate() {
                    ui_kit::divider(ui, &format!("related_constraints:{i}"));
                    ui_kit::text(ui, &group.title, ui_kit::Text::Heading("related_constraints"));
                    for (id, text) in &group.rows {
                        // One constraint can tie the selection to several entities: each row has
                        // its own key.
                        if ui_kit::list_row(ui, &format!("{id}:{i}"), text).clicked() {
                            remove = Some(id.clone());
                        }
                    }
                }
            }
        });
    if let Some(id) = remove {
        if let Err(e) = app.run("sketch.delete", json!({ "entities": [id] })) {
            app.set_status(crate::friendly_error("sketch.delete", &e), true);
        }
        // The selection stays the window's subject while the next frame reads it again.
        w.read = None;
    }
    if closed || action != ui_kit::Action::None {
        return;
    }
    app.related_constraints = Some(w);
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::scenario::Harness;
    use solidcraft_engine::Sel;

    /// Two lines meeting at a corner, the first horizontal and perpendicular to the second, and
    /// a free third line: (first, second, third).
    fn corner(h: &mut Harness) -> (String, String, String) {
        h.app.home.open = false;
        h.app.run("sketch.create", json!({"plane": "XY"})).unwrap();
        let id = |v: serde_json::Value| v["curves"][0].as_str().map(str::to_string).unwrap();
        let a = id(h.app.run("sketch.line", json!({"points": [[0, 0], [10, 0]]})).unwrap());
        let b = id(h.app.run("sketch.line", json!({"points": [[10, 0], [10, 10]]})).unwrap());
        let c = id(h.app.run("sketch.line", json!({"points": [[20, 0], [30, 5]]})).unwrap());
        h.app.run("sketch.constraint.perpendicular", json!({"a": a, "b": b})).unwrap();
        h.app.run("sketch.constraint.horizontal_vertical", json!({"line": a})).unwrap();
        (a, b, c)
    }

    fn select(h: &mut Harness, id: &str) {
        h.app.session.selection = vec![Sel::SketchCurve { id: id.to_string(), sketch: None }];
    }

    #[test]
    fn the_window_follows_the_selection_and_removes_a_constraint() {
        let mut h = Harness::new();
        let (a, b, c) = corner(&mut h);
        select(&mut h, &a);
        h.step(&json!({"start": "sketch.related_constraints"})).unwrap();
        h.frames(3);
        let w = h.app.related_constraints.as_ref().expect("the menu item opens the window");
        let view = w.view.as_ref().expect("a line is selected").as_ref().expect("the sketch is read");
        assert!(view.groups.iter().any(|g| g.title.contains(&b)), "the second line is related: {view:?}");
        assert!(view.groups.iter().any(|g| g.title.contains("selection")), "the horizontal constraint is the line's own: {view:?}");
        // A click on the free line shows it; the window stays open.
        select(&mut h, &c);
        h.frames(2);
        let view = h.app.related_constraints.as_ref().unwrap().view.as_ref().unwrap().as_ref().unwrap();
        assert!(view.subject.contains(&c));
        assert!(view.groups.is_empty(), "nothing ties the free line: {view:?}");
        // A click on empty space keeps what the window shows.
        h.app.session.selection.clear();
        h.frames(2);
        assert!(h.app.related_constraints.as_ref().unwrap().entities == vec![c.clone()]);
        // Back to the first line, and remove a constraint to the second line from the window.
        select(&mut h, &a);
        h.frames(3);
        let view = h.app.related_constraints.as_ref().unwrap().view.clone().unwrap().unwrap();
        let group = view.groups.iter().position(|g| g.title.contains(&b)).unwrap();
        let (removed, _) = view.groups[group].rows[0].clone();
        let handle = format!("dialog:list:{removed}:{group}:remove");
        let at = crate::scenario::handle_at(&handle).expect("each constraint has a remove button");
        let undo = h.app.session.undo.len();
        h.step(&json!({"click": [at.x, at.y]})).unwrap();
        h.frames(3);
        let st = crate::open_meshing::session_world(&h.app.session);
        let sk = &st.sketch(h.app.session.active_sketch.unwrap()).unwrap().sketch;
        assert!(!sk.constraints.iter().any(|k| k.id == removed), "the constraint is gone");
        assert_eq!(h.app.session.undo.len(), undo + 1, "one undo step");
        let after = h.app.related_constraints.as_ref().expect("the window stays open").view.clone().unwrap().unwrap();
        let rows_of = |v: &super::View| v.groups.iter().find(|g| g.title.contains(&b)).map_or(0, |g| g.rows.len());
        assert_eq!(rows_of(&after), rows_of(&view) - 1, "the window reads the sketch again: {after:?}");
        assert!(!after.groups.iter().flat_map(|g| &g.rows).any(|(id, _)| *id == removed), "{after:?}");
        // Finishing the sketch closes the window.
        h.app.run("sketch.finish", json!({})).unwrap();
        h.frames(2);
        assert!(h.app.related_constraints.is_none());
    }

    #[test]
    fn the_context_menu_offers_it_only_for_constrained_entities() {
        let mut h = Harness::new();
        let (a, _, c) = corner(&mut h);
        let offered = |h: &Harness| {
            crate::context_menu::items(&h.app, &crate::context_menu::Target::Viewport).iter().any(|i| i.id == "sketch.related_constraints")
        };
        select(&mut h, &a);
        assert!(offered(&h), "a constrained line offers the item");
        select(&mut h, &c);
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
