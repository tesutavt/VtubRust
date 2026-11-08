//! Periodic plugin API events computed from the model geometry:
//! ModelOutlineEvent (15 FPS convex hull), ArtMeshTrackingEvent (tracked points on
//! ArtMeshes) and ArtMeshOutlineEvent (20-point boundary rings), plus the
//! ArtMeshSelectionRequest window state.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::app::App;
use crate::scene::live2d::Live2D;

/// Per-session state of the periodic events.
#[derive(Default)]
pub struct EvState {
    pub last: HashMap<&'static str, Instant>,
    pub counter: HashMap<&'static str, u64>,
    /// Sent the "no model loaded" event already.
    pub sent_unloaded: HashSet<&'static str>,
    /// ArtMesh boundary rings (vertex index loops), per (model id, ArtMesh id).
    pub rings: HashMap<(String, String), Vec<Vec<usize>>>,
}

/// Pending ArtMeshSelectionRequest shown to the user.
pub struct MeshSelect {
    pub session: u64,
    pub request_id: String,
    pub text: String,
    pub help: String,
    pub count: i64,
    pub active: HashSet<String>,
    pub filter: String,
    /// ArtMeshes under the last click on the model (list filter), if any.
    pub at_click: Option<Vec<String>>,
    pub show_help: bool,
}

fn vts(app: &App, w: [f32; 2]) -> [f32; 2] {
    let (vw, vh) = app.viewport();
    [w[0] / (vw / 2.0), w[1] / (vh / 2.0)]
}

fn world_vertices(a: &Live2D, d: usize) -> Vec<[f32; 2]> {
    let xf = a.eff_xf();
    a.model.positions(d).iter().map(|v| xf.to_world([v.x, v.y])).collect()
}

fn visible(a: &Live2D, d: usize) -> bool {
    a.model.dynamic_flags()[d] & 1 != 0 && a.model.opacities()[d] > 0.001
}

/// Monotone chain convex hull.
fn convex_hull(mut p: Vec<[f32; 2]>) -> Vec<[f32; 2]> {
    p.sort_by(|a, b| a[0].partial_cmp(&b[0]).unwrap().then(a[1].partial_cmp(&b[1]).unwrap()));
    p.dedup();
    if p.len() < 3 {
        return p;
    }
    let cross = |o: [f32; 2], a: [f32; 2], b: [f32; 2]| (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0]);
    let mut lower: Vec<[f32; 2]> = Vec::new();
    for &q in &p {
        while lower.len() >= 2 && cross(lower[lower.len() - 2], lower[lower.len() - 1], q) <= 0.0 {
            lower.pop();
        }
        lower.push(q);
    }
    let mut upper: Vec<[f32; 2]> = Vec::new();
    for &q in p.iter().rev() {
        while upper.len() >= 2 && cross(upper[upper.len() - 2], upper[upper.len() - 1], q) <= 0.0 {
            upper.pop();
        }
        upper.push(q);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

pub fn model_outline(app: &App) -> Option<Value> {
    let a = app.avatar.as_ref()?;
    let mut centers = Vec::new();
    for d in 0..a.model.drawables.len() {
        if !visible(a, d) || a.model.drawables[d].index_count == 0 {
            continue;
        }
        let v = world_vertices(a, d);
        let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
        for p in &v {
            lo = [lo[0].min(p[0]), lo[1].min(p[1])];
            hi = [hi[0].max(p[0]), hi[1].max(p[1])];
        }
        centers.push(vts(app, [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0]));
    }
    let hull = convex_hull(centers);
    if hull.len() < 3 {
        return None;
    }
    let n = hull.len() as f32;
    let c = [hull.iter().map(|p| p[0]).sum::<f32>() / n, hull.iter().map(|p| p[1]).sum::<f32>() / n];
    let (vw, vh) = app.viewport();
    Some(json!({
        "modelName": a.name, "modelID": a.model_id,
        "convexHull": hull.iter().map(|p| json!({ "x": p[0], "y": p[1] })).collect::<Vec<_>>(),
        "convexHullCenter": { "x": c[0], "y": c[1] },
        "windowSize": { "x": vw, "y": vh },
    }))
}

/// Boundary rings of a mesh: edges used by exactly one triangle, chained into loops.
pub fn boundary_rings(idx: &[u16]) -> Vec<Vec<usize>> {
    let mut count: HashMap<(usize, usize), u32> = HashMap::new();
    for t in idx.chunks_exact(3) {
        for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            let k = (a.min(b) as usize, a.max(b) as usize);
            *count.entry(k).or_default() += 1;
        }
    }
    let mut adj: HashMap<usize, Vec<usize>> = HashMap::new();
    for ((a, b), n) in &count {
        if *n == 1 {
            adj.entry(*a).or_default().push(*b);
            adj.entry(*b).or_default().push(*a);
        }
    }
    let mut seen = HashSet::new();
    let mut rings = Vec::new();
    let mut starts: Vec<usize> = adj.keys().copied().collect();
    starts.sort_unstable();
    for s in starts {
        if seen.contains(&s) {
            continue;
        }
        let mut ring = vec![s];
        seen.insert(s);
        let mut cur = s;
        loop {
            let next = adj[&cur].iter().copied().find(|v| !seen.contains(v));
            match next {
                Some(n) => {
                    seen.insert(n);
                    ring.push(n);
                    cur = n;
                }
                None => break,
            }
        }
        if ring.len() >= 3 {
            rings.push(ring);
        }
    }
    rings
}

/// Resamples a closed polyline to `n` points evenly spaced by arc length.
fn resample(pts: &[[f32; 2]], n: usize) -> Vec<[f32; 2]> {
    let m = pts.len();
    let seg: Vec<f32> = (0..m).map(|i| {
        let (a, b) = (pts[i], pts[(i + 1) % m]);
        ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt()
    }).collect();
    let total: f32 = seg.iter().sum();
    if total <= 0.0 {
        return vec![pts[0]; n];
    }
    let mut out = Vec::with_capacity(n);
    let (mut i, mut acc) = (0usize, 0.0f32);
    for k in 0..n {
        let target = total * k as f32 / n as f32;
        while i < m - 1 && acc + seg[i] < target {
            acc += seg[i];
            i += 1;
        }
        let t = if seg[i] > 0.0 { ((target - acc) / seg[i]).clamp(0.0, 1.0) } else { 0.0 };
        let (a, b) = (pts[i], pts[(i + 1) % m]);
        out.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]);
    }
    out
}

fn area(p: &[[f32; 2]]) -> f32 {
    let n = p.len();
    (0..n).map(|i| p[i][0] * p[(i + 1) % n][1] - p[(i + 1) % n][0] * p[i][1]).sum::<f32>().abs() / 2.0
}

fn r4(v: f32) -> f64 {
    ((v as f64) * 10000.0).round() / 10000.0
}

fn due(st: &mut EvState, name: &'static str, freq: f64) -> bool {
    let period = Duration::from_secs_f64(1.0 / freq.clamp(1.0, 60.0));
    let now = Instant::now();
    match st.last.get(name) {
        Some(t) if now.duration_since(*t) < period => false,
        _ => {
            st.last.insert(name, now);
            true
        }
    }
}

/// Validates the config of ArtMeshTracking / ArtMeshOutline subscriptions (error IDs as in VTS).
pub fn validate(name: &str, cfg: &Value) -> Result<(), (i32, String)> {
    match name {
        "ArtMeshTrackingEvent" => {
            let f = cfg["frequency"].as_f64().unwrap_or(30.0);
            if !(1.0..=60.0).contains(&f) {
                return Err((100151, "Frequency must be 1-60.".into()));
            }
            let pts = cfg["trackingPoints"].as_array().cloned().unwrap_or_default();
            let mut ids = HashSet::new();
            if pts.is_empty() || pts.len() > 500 {
                return Err((100150, "1-500 tracking points required.".into()));
            }
            for p in &pts {
                let id = p["trackingPointID"].as_str().unwrap_or("");
                let c = &p["artMeshCoords"];
                let w: f64 = ["vertexWeight1", "vertexWeight2", "vertexWeight3"].iter().map(|k| c[*k].as_f64().unwrap_or(-1.0)).sum();
                let size = c["size"].as_f64().unwrap_or(1.0);
                let bad = id.is_empty()
                    || id.len() > 128
                    || !ids.insert(id.to_string())
                    || c["artMeshID"].as_str().map(|s| s.is_empty() || s.len() > 128).unwrap_or(true)
                    || ["vertexID1", "vertexID2", "vertexID3"].iter().any(|k| c[*k].as_i64().map(|v| v < 0).unwrap_or(true))
                    || (w - 1.0).abs() > 0.01
                    || !(size > 0.0 && size <= 10000.0);
                if bad {
                    return Err((100150, format!("Tracking point \"{id}\" is invalid.")));
                }
            }
            Ok(())
        }
        "ArtMeshOutlineEvent" => {
            let f = cfg["frequency"].as_f64().unwrap_or(15.0);
            if !(1.0..=30.0).contains(&f) {
                return Err((100201, "Frequency must be 1-30.".into()));
            }
            let list = cfg["artMeshes"].as_array().cloned().unwrap_or_default();
            let mut seen = HashSet::new();
            if list.is_empty() || list.len() > 100 {
                return Err((100200, "1-100 ArtMeshes required.".into()));
            }
            for m in &list {
                let k = (m["modelID"].as_str().unwrap_or("").to_string(), m["artMeshID"].as_str().unwrap_or("").to_string());
                if k.1.is_empty() || !seen.insert(k) {
                    return Err((100200, "ArtMesh list invalid or contains duplicates.".into()));
                }
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn tracking_payload(app: &App, cfg: &Value, counter: u64) -> Value {
    let (vw, vh) = app.viewport();
    let pts = cfg["trackingPoints"].as_array().cloned().unwrap_or_default();
    let Some(a) = app.avatar.as_ref() else {
        return json!({ "modelLoaded": false, "modelID": "", "windowSize": { "x": vw, "y": vh }, "subscribedPointsCount": pts.len(), "foundPointsCount": 0, "eventCounter": counter, "trackingPoints": [] });
    };
    let mut found = Vec::new();
    for p in &pts {
        let c = &p["artMeshCoords"];
        if !c["modelID"].as_str().unwrap_or("").eq_ignore_ascii_case(&a.model_id) {
            continue;
        }
        let Some(d) = c["artMeshID"].as_str().and_then(|m| a.drawable_index(m)) else { continue };
        let tri = [c["vertexID1"].as_u64().unwrap_or(0) as u16, c["vertexID2"].as_u64().unwrap_or(0) as u16, c["vertexID3"].as_u64().unwrap_or(0) as u16];
        let Some(t) = a.tri_world(d, tri) else { continue };
        let w = [c["vertexWeight1"].as_f64().unwrap_or(0.0) as f32, c["vertexWeight2"].as_f64().unwrap_or(0.0) as f32, c["vertexWeight3"].as_f64().unwrap_or(0.0) as f32];
        let pos = [t[0][0] * w[0] + t[1][0] * w[1] + t[2][0] * w[2], t[0][1] * w[0] + t[1][1] * w[1] + t[2][1] * w[2]];
        // Rotation: the v1→v2 edge (plus the base angle); size scales with the edge length.
        let e = [t[1][0] - t[0][0], t[1][1] - t[0][1]];
        let len = (e[0] * e[0] + e[1] * e[1]).sqrt();
        let base_angle = c["angle"].as_f64().unwrap_or(0.0) as f32;
        let rot = (e[1].atan2(e[0]).to_degrees() + base_angle).rem_euclid(360.0);
        let size = c["size"].as_f64().unwrap_or(1.0) as f32 * len / (vh / 2.0);
        let v = vts(app, pos);
        found.push(json!({ "trackingPointID": p["trackingPointID"], "artMeshVisible": visible(a, d), "position": { "x": v[0], "y": v[1] }, "rotation": rot, "size": size }));
    }
    json!({ "modelLoaded": true, "modelID": a.model_id, "windowSize": { "x": vw, "y": vh }, "subscribedPointsCount": pts.len(), "foundPointsCount": found.len(), "eventCounter": counter, "trackingPoints": found })
}

fn outline_payload(app: &App, cfg: &Value, counter: u64, st: &mut EvState) -> Value {
    let (vw, vh) = app.viewport();
    let list = cfg["artMeshes"].as_array().cloned().unwrap_or_default();
    let Some(a) = app.avatar.as_ref() else {
        return json!({ "modelLoaded": false, "modelID": "", "windowSize": { "x": vw, "y": vh }, "subscribedArtMeshCount": list.len(), "foundArtMeshCount": 0, "eventCounter": counter, "artMeshOutlines": [] });
    };
    let mut out = Vec::new();
    for m in &list {
        if !m["modelID"].as_str().unwrap_or("").eq_ignore_ascii_case(&a.model_id) {
            continue;
        }
        let id = m["artMeshID"].as_str().unwrap_or("").to_string();
        let Some(d) = a.drawable_index(&id) else { continue };
        let rings = st.rings.entry((a.model_id.clone(), id.clone())).or_insert_with(|| boundary_rings(a.model.indices(d))).clone();
        let verts = world_vertices(a, d);
        let mut total = 0.0;
        let mut rings_out = Vec::new();
        for r in &rings {
            let pts: Vec<[f32; 2]> = r.iter().filter_map(|&i| verts.get(i).map(|p| vts(app, *p))).collect();
            if pts.len() < 3 {
                continue;
            }
            let rs = resample(&pts, 20);
            total += area(&rs);
            rings_out.push(json!({ "points": rs.iter().flat_map(|p| [r4(p[0]), r4(p[1])]).collect::<Vec<_>>() }));
        }
        out.push(json!({ "artMeshID": id, "artMeshVisible": visible(a, d), "outlineCount": rings_out.len(), "outlineArea": r4(total), "outlinePoints": rings_out }));
    }
    json!({ "modelLoaded": true, "modelID": a.model_id, "windowSize": { "x": vw, "y": vh }, "subscribedArtMeshCount": list.len(), "foundArtMeshCount": out.len(), "eventCounter": counter, "artMeshOutlines": out })
}

/// Builds the periodic events for one session. Returns (event name, payload) pairs.
pub fn tick(app: &App, events: &HashMap<String, Value>, st: &mut EvState) -> Vec<(&'static str, Value)> {
    let mut out = Vec::new();
    let loaded = app.avatar.is_some();
    for (name, freq) in [("ModelOutlineEvent", 15.0), ("ArtMeshTrackingEvent", 0.0), ("ArtMeshOutlineEvent", 0.0)] {
        let Some(cfg) = events.get(name) else { continue };
        let f = if freq > 0.0 { freq } else { cfg["frequency"].as_f64().unwrap_or(if name == "ArtMeshTrackingEvent" { 30.0 } else { 15.0 }) };
        if !loaded {
            // One "no model" event, then silence until a model is loaded (VTS).
            if name != "ModelOutlineEvent" && st.sent_unloaded.insert(name) {
                let c = *st.counter.get(name).unwrap_or(&0);
                let p = if name == "ArtMeshTrackingEvent" { tracking_payload(app, cfg, c) } else { outline_payload(app, cfg, c, st) };
                out.push((name, p));
                st.counter.insert(name, c + 1);
            }
            continue;
        }
        st.sent_unloaded.remove(name);
        if !due(st, name, f) {
            continue;
        }
        let c = *st.counter.get(name).unwrap_or(&0);
        let p = match name {
            "ModelOutlineEvent" => match model_outline(app) {
                Some(p) => p,
                None => continue,
            },
            "ArtMeshTrackingEvent" => tracking_payload(app, cfg, c),
            _ => outline_payload(app, cfg, c, st),
        };
        st.counter.insert(name, c + 1);
        out.push((name, p));
    }
    out
}

/// Debug drawing requested by plugins: ModelOutlineEvent "draw" and tracking-point visualizers.
pub fn draw_debug(app: &App, ctx: &egui::Context) {
    let Some(srv) = app.api.as_ref() else { return };
    let ppp = ctx.pixels_per_point();
    let (vw, vh) = app.viewport();
    let to_screen = |v: [f32; 2]| egui::pos2((v[0] * vw / 2.0 + vw / 2.0) / ppp, (vh / 2.0 - v[1] * vh / 2.0) / ppp);
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::new("api-debug")));
    let mut draw_outline = false;
    let mut points = Vec::new();
    for s in srv.sessions.values() {
        if s.events.get("ModelOutlineEvent").map(|c| c["draw"].as_bool() == Some(true)).unwrap_or(false) {
            draw_outline = true;
        }
        if let Some(cfg) = s.events.get("ArtMeshTrackingEvent") {
            let p = tracking_payload(app, cfg, 0);
            let vis: HashSet<String> = cfg["trackingPoints"].as_array().into_iter().flatten().filter(|p| p["visualize"].as_bool() == Some(true)).filter_map(|p| p["trackingPointID"].as_str().map(str::to_string)).collect();
            for tp in p["trackingPoints"].as_array().cloned().unwrap_or_default() {
                if vis.contains(tp["trackingPointID"].as_str().unwrap_or("")) {
                    points.push(tp);
                }
            }
        }
    }
    let red = egui::Color32::from_rgb(255, 60, 120);
    if draw_outline {
        if let Some(o) = model_outline(app) {
            let hull: Vec<egui::Pos2> = o["convexHull"].as_array().unwrap().iter().map(|p| to_screen([p["x"].as_f64().unwrap() as f32, p["y"].as_f64().unwrap() as f32])).collect();
            painter.add(egui::Shape::closed_line(hull, egui::Stroke::new(2.0f32, red)));
        }
    }
    for tp in points {
        let c = to_screen([tp["position"]["x"].as_f64().unwrap_or(0.0) as f32, tp["position"]["y"].as_f64().unwrap_or(0.0) as f32]);
        let r = tp["size"].as_f64().unwrap_or(0.05) as f32 * vh / 4.0 / ppp;
        let a = (tp["rotation"].as_f64().unwrap_or(0.0) as f32).to_radians();
        painter.circle_stroke(c, r.max(3.0), egui::Stroke::new(2.0f32, red));
        painter.line_segment([c, c + egui::vec2(a.cos(), -a.sin()) * r.max(3.0)], egui::Stroke::new(2.0f32, red));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn square_boundary_is_one_ring() {
        // Two triangles forming a quad: the boundary is the 4 outer vertices.
        let r = boundary_rings(&[0, 1, 2, 0, 2, 3]);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].len(), 4);
    }

    #[test]
    fn hull_and_resample() {
        let h = convex_hull(vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0], [0.5, 0.5]]);
        assert_eq!(h.len(), 4);
        let rs = resample(&h, 20);
        assert_eq!(rs.len(), 20);
        assert!((area(&h) - 1.0).abs() < 1e-5);
    }
}
