//! Cleanup in plan millimetres: axis snapping, collinear merging, opening detection in wall gaps,
//! junction snapping, node merging, thickness rounding, and building the plan-core floor.

use std::collections::HashMap;

use plan_core::commands::{add_wall_chain, ChainPoint};
use plan_core::geom::*;
use plan_core::rooms::derive_rooms;
use plan_core::{Floor, Hinge, Id, Opening, OpeningKind, Project, RoomMeta, Swing, WallKind};
use uuid::Uuid;

use super::fusion::WSeg;
use super::preprocess::Mask;

pub const THICKNESSES: [u32; 9] = [100, 120, 150, 175, 200, 250, 300, 365, 400];
const DOOR_H: u32 = 2050;
const WINDOW_SILL: u32 = 850;
const WINDOW_H: u32 = 1400;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OKind {
    Door,
    Window,
    Passage,
}

/// Opening candidate in mm.
#[derive(Clone, Debug, PartialEq)]
pub struct OCand {
    pub center: P,
    pub width: f64,
    pub kind: OKind,
    pub hinge: Option<P>,
    pub into: Option<P>,
    pub conf: f32,
    /// From the LLM (true) or a CV wall gap (false).
    pub llm: bool,
}

pub struct RoomCand {
    pub name: String,
    pub pos: P,
}

pub struct CleanInput<'a> {
    pub walls: Vec<WSeg>,
    pub openings: Vec<OCand>,
    pub rooms: Vec<RoomCand>,
    pub mask: &'a Mask,
    pub mm_per_px: f64,
    pub floor_id: Id,
    pub used_llm: bool,
}

pub struct CleanOutput {
    pub floor: Floor,
    pub confidence: HashMap<Id, f32>,
    pub notes: Vec<String>,
}

// ---- image sampling ----------------------------------------------------------------------------

/// Samples the ink mask with mm coordinates.
pub struct Sampler<'a> {
    pub mask: &'a Mask,
    pub px_per_mm: f64,
}

impl Sampler<'_> {
    fn ink(&self, p: P) -> bool {
        let x = (p[0] * self.px_per_mm).floor() as i64;
        let y = (p[1] * self.px_per_mm).floor() as i64;
        self.mask.get(x, y)
    }
    /// For the strip p0→p1 with half width `half` (mm): (coverage along, fill ratio).
    pub fn gap_stats(&self, p0: P, p1: P, half: f64) -> (f64, f64) {
        let l = dist(p0, p1);
        let step = 1.0 / self.px_per_mm;
        let d = norm(sub(p1, p0));
        let n = perp(d);
        let na = ((l / step).floor() as i64 - 2).max(1);
        let nc = ((2.0 * half / step).ceil() as i64).max(1);
        let (mut cov, mut fill, mut tot) = (0usize, 0usize, 0usize);
        for i in 1..=na {
            let base = add(p0, mul(d, i as f64 * step));
            let mut any = false;
            for j in 0..=nc {
                let q = add(base, mul(n, -half + j as f64 * step));
                tot += 1;
                if self.ink(q) {
                    any = true;
                    fill += 1;
                }
            }
            if any {
                cov += 1;
            }
        }
        (cov as f64 / na as f64, fill as f64 / tot.max(1) as f64)
    }
    /// Door leaf next to a gap: returns (hinge point, a point in the swing side) if a leaf line is found.
    pub fn door_leaf(&self, p0: P, p1: P, half: f64) -> Option<(P, P, f64)> {
        let g = dist(p0, p1);
        let d = norm(sub(p1, p0));
        let n = perp(d);
        let step = 1.0 / self.px_per_mm;
        let w = (3.0 * step).max(0.06 * g);
        let mut best: Option<(P, P, f64)> = None;
        for (e, inward) in [(p0, d), (p1, mul(d, -1.0))] {
            for s in [1.0, -1.0] {
                let nn = mul(n, s);
                let k = 24;
                let mut hits = 0;
                for i in 0..k {
                    let across = half + g * (0.15 + 0.7 * i as f64 / (k - 1) as f64);
                    let c = add(e, mul(nn, across));
                    let m = (w / step).ceil() as i64;
                    let found = (-m..=m).any(|j| self.ink(add(c, mul(inward, j as f64 * step))));
                    if found {
                        hits += 1;
                    }
                }
                let score = hits as f64 / k as f64;
                if best.as_ref().is_none_or(|b| score > b.2) {
                    let into = add(lerp(p0, p1, 0.5), mul(nn, half + g * 0.5));
                    best = Some((e, into, score));
                }
            }
        }
        best.filter(|b| b.2 >= 0.6)
    }
}

// ---- segment helpers -----------------------------------------------------------------------------

fn angle_deg(d: P) -> f64 {
    d[1].atan2(d[0]).to_degrees().rem_euclid(180.0)
}

/// Snap near-axis (±3°) and near-45° (±6°) directions exactly, rotating about the midpoint.
fn axis_snap(s: &mut WSeg) {
    let ang = angle_deg(sub(s.b, s.a));
    for target in [0.0, 45.0, 90.0, 135.0, 180.0] {
        let tol = if target == 45.0 || target == 135.0 { 6.0 } else { 3.0 };
        if (ang - target).abs() <= tol {
            let m = lerp(s.a, s.b, 0.5);
            let h = s.len() / 2.0;
            let mut d = [target.to_radians().cos(), target.to_radians().sin()];
            if dot(d, sub(s.b, s.a)) < 0.0 {
                d = mul(d, -1.0);
            }
            if target == 0.0 || target == 180.0 {
                d = [d[0].signum(), 0.0];
            } else if target == 90.0 {
                d = [0.0, d[1].signum()];
            }
            s.a = sub(m, mul(d, h));
            s.b = add(m, mul(d, h));
            return;
        }
    }
}

fn parallel(a: &WSeg, b: &WSeg, max_deg: f64) -> bool {
    cross(a.dir(), b.dir()).abs() <= max_deg.to_radians().sin()
}

/// Position of b's endpoints along a (from a.a) and perpendicular offset of b's midpoint.
fn relate(a: &WSeg, b: &WSeg) -> (f64, f64, f64) {
    let d = a.dir();
    let ta = dot(sub(b.a, a.a), d);
    let tb = dot(sub(b.b, a.a), d);
    let off = cross(d, sub(lerp(b.a, b.b, 0.5), a.a));
    (ta.min(tb), ta.max(tb), off)
}

fn merge_two(a: &WSeg, b: &WSeg) -> WSeg {
    let d = a.dir();
    let (b0, b1, off) = relate(a, b);
    let (la, lb) = (a.len(), b.len());
    let wsum = (la + lb).max(1e-9);
    let off_m = off * lb / wsum;
    let n = perp(d);
    let base = add(a.a, mul(n, off_m));
    let t0 = 0f64.min(b0);
    let t1 = la.max(b1);
    WSeg {
        a: add(base, mul(d, t0)),
        b: add(base, mul(d, t1)),
        t: (a.t * la + b.t * lb) / wsum,
        conf: ((a.conf as f64 * la + b.conf as f64 * lb) / wsum) as f32,
        exterior: a.exterior.or(b.exterior),
    }
}

/// Merge overlapping / touching collinear segments.
fn merge_collinear(segs: &mut Vec<WSeg>) {
    loop {
        let mut did = false;
        'outer: for i in 0..segs.len() {
            for j in (i + 1)..segs.len() {
                let (a, b) = (&segs[i], &segs[j]);
                if !parallel(a, b, 2.5) {
                    continue;
                }
                let (b0, b1, off) = relate(a, b);
                if off.abs() > 0.5 * a.t.max(b.t) {
                    continue;
                }
                let la = a.len();
                let ov = b1.min(la) - b0.max(0.0);
                let gap_tol = 60f64.max(0.5 * a.t.min(b.t));
                let ratio = a.t.max(b.t) / a.t.min(b.t).max(1.0);
                if ov > 0.0 || (ov >= -gap_tol && ratio <= 1.5) {
                    let m = if la >= b.len() { merge_two(a, b) } else { merge_two(b, a) };
                    segs[i] = m;
                    segs.remove(j);
                    did = true;
                    break 'outer;
                }
            }
        }
        if !did {
            break;
        }
    }
}

fn classify_gap(s: &Sampler, p0: P, p1: P, half: f64) -> Option<(OKind, Option<(P, P)>)> {
    let g = dist(p0, p1);
    let (cov, fill) = s.gap_stats(p0, p1, half);
    if cov >= 0.75 && fill <= 0.65 && (300.0..=6000.0).contains(&g) {
        return Some((OKind::Window, None));
    }
    if cov <= 0.35 {
        let leaf = s.door_leaf(p0, p1, half);
        if let Some((h, into, _)) = leaf {
            if (500.0..=1400.0).contains(&g) {
                return Some((OKind::Door, Some((h, into))));
            }
        }
        if (500.0..=2600.0).contains(&g) {
            return Some((OKind::Passage, None));
        }
    }
    None
}

fn gap_blocked(segs: &[WSeg], skip: &[usize], p0: P, p1: P) -> bool {
    segs.iter().enumerate().any(|(k, s)| !skip.contains(&k) && segment_intersect(p0, p1, s.a, s.b).is_some())
}

/// Bridge gaps between collinear segments (doors, windows, passages).
fn bridge_collinear(segs: &mut Vec<WSeg>, opens: &mut Vec<OCand>, s: &Sampler) {
    loop {
        #[allow(clippy::type_complexity)]
        let mut best: Option<(usize, usize, f64, P, P, OKind, Option<(P, P)>)> = None;
        for i in 0..segs.len() {
            for j in 0..segs.len() {
                if i == j {
                    continue;
                }
                let (a, b) = (&segs[i], &segs[j]);
                if !parallel(a, b, 2.5) {
                    continue;
                }
                let (b0, _b1, off) = relate(a, b);
                if off.abs() > 0.5 * a.t.max(b.t) || a.t.max(b.t) / a.t.min(b.t).max(1.0) > 1.6 {
                    continue;
                }
                let la = a.len();
                let gap = b0 - la; // b starts after a ends
                if gap <= 0.0 || gap > 6000.0 || best.as_ref().is_some_and(|x| x.2 <= gap) {
                    continue;
                }
                let d = a.dir();
                let p0 = add(a.a, mul(d, la));
                let p1 = add(a.a, mul(d, b0));
                let p1 = add(p1, mul(perp(d), -cross(d, sub(p1, a.a)))); // on a's line
                if gap_blocked(segs, &[i, j], lerp(p0, p1, 0.02), lerp(p0, p1, 0.98)) {
                    continue;
                }
                let half = a.t.min(b.t) / 2.0;
                if let Some((k, leaf)) = classify_gap(s, p0, p1, half) {
                    best = Some((i, j, gap, p0, p1, k, leaf));
                }
            }
        }
        let Some((i, j, gap, p0, p1, kind, leaf)) = best else { break };
        let m = merge_two(&segs[i], &segs[j]);
        let conf = match kind {
            OKind::Door => 0.6,
            OKind::Window => 0.55,
            OKind::Passage => 0.4,
        };
        opens.push(OCand { center: lerp(p0, p1, 0.5), width: gap, kind, hinge: leaf.map(|l| l.0), into: leaf.map(|l| l.1), conf, llm: false });
        let (hi, lo) = (i.max(j), i.min(j));
        segs.remove(hi);
        segs[lo] = m;
    }
}

/// Endpoint index helpers: (segment, end 0=a/1=b).
fn end(s: &WSeg, e: usize) -> P {
    if e == 0 {
        s.a
    } else {
        s.b
    }
}
fn set_end(s: &mut WSeg, e: usize, p: P) {
    if e == 0 {
        s.a = p
    } else {
        s.b = p
    }
}
fn out_dir(s: &WSeg, e: usize) -> P {
    if e == 0 {
        mul(s.dir(), -1.0)
    } else {
        s.dir()
    }
}

/// Extend/trim endpoints to the centre lines of nearby crossing walls (L/T junctions).
#[allow(clippy::needless_range_loop)]
fn snap_junctions(segs: &mut [WSeg]) -> Vec<Vec<bool>> {
    let n = segs.len();
    let mut snapped = vec![vec![false, false]; n];
    for i in 0..n {
        for e in 0..2 {
            let p = end(&segs[i], e);
            let d = out_dir(&segs[i], e);
            let ti = segs[i].t;
            let mut best: Option<(f64, P)> = None;
            for j in 0..n {
                if i == j {
                    continue;
                }
                let o = &segs[j];
                let dj = o.dir();
                if cross(d, dj).abs() < (30f64).to_radians().sin() {
                    continue;
                }
                let Some(x) = line_intersect(p, d, o.a, dj) else { continue };
                let s = dot(sub(x, p), d);
                let lim = o.t / 2.0 + ti.max(100.0) + 50.0;
                if s.abs() > lim {
                    continue;
                }
                let tj = dot(sub(x, o.a), dj);
                let tol = ti / 2.0 + 60.0;
                if tj < -tol || tj > o.len() + tol {
                    continue;
                }
                // the remaining wall must keep a sensible length
                let other = end(&segs[i], 1 - e);
                if dist(other, x) < 0.5 * segs[i].len().min(200.0) {
                    continue;
                }
                if best.is_none_or(|b| s.abs() < b.0) {
                    best = Some((s.abs(), x));
                }
            }
            if let Some((_, x)) = best {
                set_end(&mut segs[i], e, x);
                snapped[i][e] = true;
            }
        }
    }
    snapped
}

fn is_free(segs: &[WSeg], i: usize, e: usize, r: f64) -> bool {
    let p = end(&segs[i], e);
    !segs.iter().enumerate().any(|(j, o)| j != i && closest_on_segment(p, o.a, o.b).2 <= r.max(o.t / 2.0 + 20.0))
}

/// Free wall ends pointing at another wall across a door/window gap: extend and add the opening.
#[allow(clippy::needless_range_loop)]
fn extend_free_ends(segs: &mut [WSeg], opens: &mut Vec<OCand>, s: &Sampler) {
    let n = segs.len();
    for i in 0..n {
        for e in 0..2 {
            if !is_free(segs, i, e, 60.0) {
                continue;
            }
            let p = end(&segs[i], e);
            let d = out_dir(&segs[i], e);
            let mut best: Option<(f64, P, f64)> = None;
            for j in 0..n {
                if i == j {
                    continue;
                }
                let o = &segs[j];
                let dj = o.dir();
                if cross(d, dj).abs() < (30f64).to_radians().sin() {
                    continue;
                }
                let Some(x) = line_intersect(p, d, o.a, dj) else { continue };
                let sd = dot(sub(x, p), d);
                let tj = dot(sub(x, o.a), dj);
                if sd <= 0.0 || sd > 6000.0 + o.t || tj < -o.t / 2.0 || tj > o.len() + o.t / 2.0 {
                    continue;
                }
                if best.is_none_or(|b| sd < b.0) {
                    best = Some((sd, x, o.t));
                }
            }
            let Some((sd, x, tj)) = best else { continue };
            let face = sd - tj / 2.0;
            if face < 400.0 {
                continue;
            }
            let p1 = add(p, mul(d, face));
            if gap_blocked(segs, &[i], lerp(p, p1, 0.02), lerp(p, p1, 0.98)) {
                continue;
            }
            let half = segs[i].t / 2.0;
            let Some((kind, leaf)) = classify_gap(s, p, p1, half) else { continue };
            // without a collinear partner only accept clear evidence: a drawn door leaf or a window
            if kind == OKind::Passage {
                continue;
            }
            set_end(&mut segs[i], e, x);
            let conf = if kind == OKind::Door { 0.55 } else { 0.5 };
            opens.push(OCand { center: lerp(p, p1, 0.5), width: face, kind, hinge: leaf.map(|l| l.0), into: leaf.map(|l| l.1), conf, llm: false });
        }
    }
}

/// Merge endpoints closer than `r` (junction-snapped points win).
fn merge_nodes(segs: &mut [WSeg], snapped: &[Vec<bool>]) {
    let mut pts: Vec<(usize, usize, P, bool, f64)> = Vec::new();
    for (i, s) in segs.iter().enumerate() {
        for e in 0..2 {
            pts.push((i, e, end(s, e), snapped.get(i).is_some_and(|v| v[e]), s.t));
        }
    }
    let n = pts.len();
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(p: &mut [usize], i: usize) -> usize {
        let mut r = i;
        while p[r] != r {
            r = p[r];
        }
        p[i] = r;
        r
    }
    for a in 0..n {
        for b in (a + 1)..n {
            if pts[a].0 == pts[b].0 {
                continue;
            }
            let r = 80f64.max(0.5 * pts[a].4.max(pts[b].4));
            if dist(pts[a].2, pts[b].2) < r {
                let (ra, rb) = (find(&mut parent, a), find(&mut parent, b));
                parent[ra] = rb;
            }
        }
    }
    let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
    for k in 0..n {
        let r = find(&mut parent, k);
        groups.entry(r).or_default().push(k);
    }
    for (_, g) in groups {
        if g.len() < 2 {
            continue;
        }
        let snapped_pts: Vec<P> = g.iter().filter(|&&k| pts[k].3).map(|&k| pts[k].2).collect();
        let src: Vec<P> = if snapped_pts.is_empty() { g.iter().map(|&k| pts[k].2).collect() } else { snapped_pts };
        let c = mul(src.iter().fold([0.0, 0.0], |acc, p| add(acc, *p)), 1.0 / src.len() as f64);
        for &k in &g {
            set_end(&mut segs[pts[k].0], pts[k].1, c);
        }
    }
    // keep axis walls exactly axis-aligned after averaging
    for s in segs.iter_mut() {
        let d = sub(s.b, s.a);
        if d[1].abs() < 1.0 && d[0].abs() > 1.0 {
            let y = (s.a[1] + s.b[1]) / 2.0;
            s.a[1] = y;
            s.b[1] = y;
        } else if d[0].abs() < 1.0 && d[1].abs() > 1.0 {
            let x = (s.a[0] + s.b[0]) / 2.0;
            s.a[0] = x;
            s.b[0] = x;
        }
    }
}

pub fn round_thickness(t: f64) -> u32 {
    if t > 425.0 {
        return ((t / 25.0).round() * 25.0) as u32;
    }
    *THICKNESSES.iter().min_by(|a, b| ((**a as f64) - t).abs().partial_cmp(&((**b as f64) - t).abs()).unwrap()).unwrap()
}

fn cp(p: P) -> ChainPoint {
    ChainPoint { node: None, wall: None, x: p[0].round() as i32, y: p[1].round() as i32 }
}

pub fn is_wet_room(name: &str) -> bool {
    let n = name.to_lowercase();
    ["bad", "dusche", "wc", "toilette", "nasszelle", "du/", "/du", "sanitär"].iter().any(|k| n.contains(k)) || n.trim() == "du"
}

fn degree(f: &Floor, node: Id) -> usize {
    f.walls_at(node).count()
}

// ---- main ----------------------------------------------------------------------------------------

pub fn cleanup(input: CleanInput) -> CleanOutput {
    let mut notes = Vec::new();
    let sampler = Sampler { mask: input.mask, px_per_mm: 1.0 / input.mm_per_px };
    let mut segs: Vec<WSeg> = input.walls.into_iter().filter(|s| s.len() > 1.0 && s.t > 0.0).collect();
    for s in segs.iter_mut() {
        axis_snap(s);
    }
    merge_collinear(&mut segs);
    let mut opens: Vec<OCand> = Vec::new();
    bridge_collinear(&mut segs, &mut opens, &sampler);
    let snapped = snap_junctions(&mut segs);
    extend_free_ends(&mut segs, &mut opens, &sampler);
    merge_nodes(&mut segs, &snapped);
    // drop degenerate / tiny isolated pieces
    let all = segs.clone();
    segs = all
        .iter()
        .enumerate()
        .filter(|(i, s)| {
            let l = s.len();
            if l < 100f64.max(s.t) {
                return false;
            }
            let free_a = is_free(&all, *i, 0, 60.0);
            let free_b = is_free(&all, *i, 1, 60.0);
            !(free_a && free_b && l < 1000.0 && s.conf < 0.6)
        })
        .map(|(_, s)| s.clone())
        .collect();

    // ---- build the floor with plan-core commands (splits at T-junctions / crossings)
    let floor_id = input.floor_id;
    let mut p = Project::new(Uuid::new_v4(), "scratch");
    p.floors = vec![Floor::new(floor_id, "EG")];
    let mut order: Vec<usize> = (0..segs.len()).collect();
    order.sort_by(|&a, &b| segs[b].len().partial_cmp(&segs[a].len()).unwrap());
    for &i in &order {
        let s = &segs[i];
        let t = round_thickness(s.t);
        let kind = if t >= 250 { WallKind::Exterior } else { WallKind::Interior };
        if let Ok(ops) = add_wall_chain(&p, floor_id, &[cp(s.a), cp(s.b)], false, t, kind) {
            let _ = plan_core::apply_batch(&mut p, &ops);
        }
    }
    let mut floor = p.floors[0].clone();
    // remove short dangling stubs (overshoots)
    loop {
        let stub = floor.walls.iter().find(|w| {
            let l = floor.wall_length(w);
            let da = degree(&floor, w.a);
            let db = degree(&floor, w.b);
            (da == 1 || db == 1) && l < 150f64.max(w.thickness_mm as f64) && !(da == 1 && db == 1 && l > 100.0)
        });
        let Some(w) = stub.cloned() else { break };
        floor.walls.retain(|x| x.id != w.id);
    }
    // drop small isolated wall clusters (symbols, text blobs, margin marks)
    {
        let mut comp: HashMap<Id, usize> = HashMap::new();
        let mut next = 0;
        for n in &floor.nodes {
            if comp.contains_key(&n.id) {
                continue;
            }
            let mut stack = vec![n.id];
            comp.insert(n.id, next);
            while let Some(v) = stack.pop() {
                for w in floor.walls_at(v) {
                    let o = w.other(v);
                    if let std::collections::hash_map::Entry::Vacant(e) = comp.entry(o) {
                        e.insert(next);
                        stack.push(o);
                    }
                }
            }
            next += 1;
        }
        let mut total = vec![0f64; next];
        for w in &floor.walls {
            total[comp[&w.a]] += floor.wall_length(w);
        }
        let max_total = total.iter().cloned().fold(0.0, f64::max);
        floor.walls.retain(|w| {
            let t = total[comp[&w.a]];
            t >= 2500.0 || t >= 0.5 * max_total
        });
    }
    // drop orphan nodes
    let used: std::collections::HashSet<Id> = floor.walls.iter().flat_map(|w| [w.a, w.b]).collect();
    floor.nodes.retain(|n| used.contains(&n.id));

    // ---- wall confidence by best-overlapping source segment
    let mut confidence: HashMap<Id, f32> = HashMap::new();
    for w in &floor.walls {
        let Some((a, b)) = floor.wall_points(w) else { continue };
        let ws = WSeg { a, b, t: w.thickness_mm as f64, conf: 0.0, exterior: None };
        let c = segs
            .iter()
            .filter(|s| parallel(&ws, s, 5.0))
            .filter_map(|s| {
                let (s0, s1, off) = relate(&ws, s);
                let ov = s1.min(ws.len()) - s0.max(0.0);
                (off.abs() <= s.t.max(ws.t) && ov > 0.0).then_some((ov, s.conf))
            })
            .max_by(|x, y| x.0.partial_cmp(&y.0).unwrap())
            .map(|x| x.1)
            .unwrap_or(0.3);
        confidence.insert(w.id, c);
    }

    // ---- openings
    let mut cands = opens;
    for o in input.openings {
        cands.push(o);
    }
    // CV first so LLM candidates can upgrade them
    cands.sort_by_key(|o| o.llm);
    let mut placed: Vec<(Opening, bool)> = Vec::new(); // (opening, from_llm)
    for o in cands {
        let max_d = if o.llm { 450.0 } else { 150.0 };
        let best = floor
            .walls
            .iter()
            .filter_map(|w| {
                let (a, b) = floor.wall_points(w)?;
                let (t, _, d) = closest_on_segment(o.center, a, b);
                let l = dist(a, b);
                let lim = w.thickness_mm as f64 / 2.0 + max_d;
                (d <= lim && t > 0.0 && t < 1.0 && l > 300.0).then_some((d, w.clone(), a, b))
            })
            .min_by(|x, y| x.0.partial_cmp(&y.0).unwrap());
        let Some((_, w, a, b)) = best else {
            if o.llm {
                notes.push(format!("{} der KI konnte keiner Wand zugeordnet werden.", match o.kind {
                    OKind::Window => "Ein Fenster",
                    _ => "Eine Tür",
                }));
            }
            continue;
        };
        let l = dist(a, b);
        let d = norm(sub(b, a));
        let along = dot(sub(o.center, a), d);
        let width = o.width.clamp(300.0, (l - 20.0).max(300.0)).min(l);
        if width > l {
            continue;
        }
        let offset = (along - width / 2.0).clamp(0.0, l - width);
        // existing opening on the same wall overlapping?
        if let Some((ex, from_llm)) = placed.iter_mut().find(|(x, _)| {
            x.wall == w.id && (x.offset_mm as f64) < offset + width && offset < (x.offset_mm + x.width_mm) as f64
        }) {
            if o.llm && !*from_llm {
                // LLM confirms a CV gap: keep CV geometry, take LLM semantics
                let kind = match o.kind {
                    OKind::Window => OpeningKind::Window { sill_mm: WINDOW_SILL },
                    OKind::Door => door_kind(o.hinge.or(hinge_point(ex, a, d)), o.into, a, d, ex.offset_mm as f64 + ex.width_mm as f64 / 2.0),
                    OKind::Passage => OpeningKind::Passage,
                };
                if !matches!(kind, OpeningKind::Door { .. }) || o.hinge.is_some() || !matches!(ex.kind, OpeningKind::Door { .. }) {
                    ex.kind = kind;
                    ex.height_mm = if matches!(kind, OpeningKind::Window { .. }) { WINDOW_H } else { DOOR_H };
                }
                *from_llm = true;
                confidence.insert(ex.id, 0.85);
            }
            continue;
        }
        let center_t = offset + width / 2.0;
        let kind = match o.kind {
            OKind::Window => OpeningKind::Window { sill_mm: WINDOW_SILL },
            OKind::Passage => OpeningKind::Passage,
            OKind::Door => door_kind(o.hinge, o.into, a, d, center_t),
        };
        let op = Opening {
            id: Uuid::new_v4(),
            wall: w.id,
            kind,
            offset_mm: offset.round() as u32,
            width_mm: width.round().max(1.0) as u32,
            height_mm: if o.kind == OKind::Window { WINDOW_H } else { DOOR_H },
        };
        let conf = if o.llm { 0.5 } else if input.used_llm { (o.conf - 0.1).max(0.3) } else { o.conf };
        confidence.insert(op.id, conf);
        placed.push((op, o.llm));
    }
    floor.openings = placed.into_iter().map(|(o, _)| o).collect();

    // ---- wall kind from room adjacency
    let derived = derive_rooms(&floor);
    if !derived.is_empty() {
        for w in floor.walls.iter_mut() {
            let (Some(a), Some(b)) = (floor.nodes.iter().find(|n| n.id == w.a), floor.nodes.iter().find(|n| n.id == w.b)) else { continue };
            let (pa, pb) = ([a.x_mm as f64, a.y_mm as f64], [b.x_mm as f64, b.y_mm as f64]);
            let m = lerp(pa, pb, 0.5);
            let nl = left_normal(norm(sub(pb, pa)));
            let off = w.thickness_mm as f64 / 2.0 + 250.0;
            let inside = |q: P| derived.iter().any(|r| point_in_polygon(q, &r.polygon));
            let outside_side = !inside(add(m, mul(nl, off))) || !inside(add(m, mul(nl, -off)));
            w.kind = if outside_side { WallKind::Exterior } else { WallKind::Interior };
        }
    }

    // ---- rooms
    let mut metas: Vec<(RoomMeta, f32, usize)> = Vec::new(); // (meta, conf, derived index)
    for rc in input.rooms {
        let hit = derived.iter().position(|r| point_in_polygon(rc.pos, &r.polygon));
        let (idx, anchor, conf) = match hit {
            Some(i) => {
                let r = &derived[i];
                let ok = point_in_polygon(rc.pos, &r.inner) && dist_to_polygon(rc.pos, &r.inner) > 100.0;
                (i, if ok { rc.pos } else { r.label }, if ok { 0.8 } else { 0.65 })
            }
            None => {
                let near = derived
                    .iter()
                    .enumerate()
                    .map(|(i, r)| (i, dist_to_polygon(rc.pos, &r.polygon)))
                    .filter(|(_, d)| *d <= 1500.0)
                    .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
                match near {
                    Some((i, _)) => (i, derived[i].label, 0.5),
                    None => {
                        notes.push(format!("Raum «{}» liegt in keinem geschlossenen Raum und wurde weggelassen.", rc.name));
                        continue;
                    }
                }
            }
        };
        if let Some(m) = metas.iter_mut().find(|m| m.2 == idx) {
            if !m.0.name.split(" / ").any(|n| n == rc.name) {
                m.0.name = format!("{} / {}", m.0.name, rc.name);
                if is_wet_room(&m.0.name) {
                    m.0.floor_material = "tile:white".into();
                }
            }
            continue;
        }
        let anchor = [anchor[0].round() as i32, anchor[1].round() as i32];
        metas.push((room_meta(&rc.name, anchor), conf, idx));
    }
    let mut k = 0;
    for (i, r) in derived.iter().enumerate() {
        if metas.iter().any(|m| m.2 == i) || r.area_m2 < 0.8 {
            continue;
        }
        k += 1;
        let anchor = [r.label[0].round() as i32, r.label[1].round() as i32];
        metas.push((room_meta(&format!("Raum {k}"), anchor), 0.3, i));
    }
    // verify anchors resolve to the intended derived room
    floor.rooms = metas.iter().map(|m| m.0.clone()).collect();
    for (m, c, _) in &metas {
        confidence.insert(m.id, *c);
    }
    CleanOutput { floor, confidence, notes }
}

fn room_meta(name: &str, anchor: [i32; 2]) -> RoomMeta {
    RoomMeta {
        id: Uuid::new_v4(),
        anchor,
        name: name.to_string(),
        floor_material: if is_wet_room(name) { "tile:white".into() } else { "wood:oak".into() },
        ceiling_material: "paint:white".into(),
        open_ceiling: false,
    }
}

fn hinge_point(o: &Opening, a: P, d: P) -> Option<P> {
    match o.kind {
        OpeningKind::Door { hinge, .. } => {
            let t = if hinge == Hinge::Start { o.offset_mm as f64 } else { (o.offset_mm + o.width_mm) as f64 };
            Some(add(a, mul(d, t)))
        }
        _ => None,
    }
}

fn door_kind(hinge: Option<P>, into: Option<P>, a: P, d: P, center_t: f64) -> OpeningKind {
    let h = hinge.map(|h| if dot(sub(h, a), d) <= center_t { Hinge::Start } else { Hinge::End }).unwrap_or(Hinge::Start);
    let s = into.map(|q| if dot(sub(q, a), left_normal(d)) > 0.0 { Swing::Left } else { Swing::Right }).unwrap_or(Swing::Left);
    OpeningKind::Door { hinge: h, swing: s, glass: false }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(a: P, b: P, t: f64) -> WSeg {
        WSeg { a, b, t, conf: 0.7, exterior: None }
    }

    #[test]
    fn thickness_rounding() {
        assert_eq!(round_thickness(118.0), 120);
        assert_eq!(round_thickness(330.0), 300);
        assert_eq!(round_thickness(350.0), 365);
        assert_eq!(round_thickness(480.0), 475);
    }

    #[test]
    fn junctions_and_rooms() {
        // CV-like input: bands overshoot to outer faces; slight angle; split collinear piece.
        let segs = vec![
            w([-150.0, 0.0], [6150.0, 3.0], 300.0),
            w([-150.0, 4000.0], [3000.0, 4000.0], 300.0),
            w([3040.0, 4002.0], [6150.0, 4000.0], 300.0),
            w([0.0, -150.0], [0.0, 4150.0], 300.0),
            w([6000.0, -150.0], [6000.0, 4150.0], 300.0),
            w([3000.0, 150.0], [3000.0, 4150.0], 120.0), // T at top? ends near bottom wall
        ];
        let mask = Mask::new(10, 10);
        let out = cleanup(CleanInput {
            walls: segs,
            openings: vec![],
            rooms: vec![RoomCand { name: "Bad".into(), pos: [1500.0, 2000.0] }],
            mask: &mask,
            mm_per_px: 10.0,
            floor_id: Uuid::new_v4(),
            used_llm: true,
        });
        let f = &out.floor;
        let rooms = derive_rooms(f);
        assert_eq!(rooms.len(), 2, "{:#?}", f.walls.len());
        let bad = f.rooms.iter().find(|r| r.name == "Bad").unwrap();
        assert_eq!(bad.floor_material, "tile:white");
        for n in &f.nodes {
            let near = |v: i32, set: &[i32]| set.iter().any(|s| (v - s).abs() <= 3);
            assert!(near(n.x_mm, &[0, 3000, 6000]) && near(n.y_mm, &[0, 4000]), "{n:?}");
        }
        assert!(f.walls.iter().all(|w| out.confidence.contains_key(&w.id)));
        assert!(f.walls.iter().filter(|w| w.thickness_mm == 300).all(|w| w.kind == WallKind::Exterior));
        assert!(f.walls.iter().filter(|w| w.thickness_mm == 120).all(|w| w.kind == WallKind::Interior));
    }
}
