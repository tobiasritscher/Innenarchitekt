//! Non-blocking validation: produces a list of hints.

use serde::{Deserialize, Serialize};

use crate::geom::*;
use crate::model::*;
use crate::rooms::derive_rooms;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum Severity {
    Warning,
    Info,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Issue {
    pub severity: Severity,
    pub code: String,
    pub message: String,
    pub ids: Vec<Id>,
}

fn issue(severity: Severity, code: &str, message: String, ids: Vec<Id>) -> Issue {
    Issue { severity, code: code.into(), message, ids }
}

pub fn validate(f: &Floor) -> Vec<Issue> {
    let mut out = Vec::new();
    for w in &f.walls {
        let l = f.wall_length(w);
        if l < 10.0 {
            out.push(issue(Severity::Warning, "wall_too_short", "Wand ist kürzer als 1 cm".into(), vec![w.id]));
            continue;
        }
        let mut ops: Vec<&Opening> = f.openings_on(w.id).collect();
        ops.sort_by_key(|o| o.offset_mm);
        for o in &ops {
            if (o.offset_mm + o.width_mm) as f64 > l + 0.5 {
                out.push(issue(
                    Severity::Warning,
                    "opening_too_wide",
                    format!("Öffnung ({} mm ab Wandanfang, {} mm breit) ragt über die Wand ({:.0} mm) hinaus", o.offset_mm, o.width_mm, l),
                    vec![o.id, w.id],
                ));
            }
            if o.height_mm + o.kind.sill_mm() > f.ceiling_height_mm {
                out.push(issue(Severity::Warning, "opening_too_high", "Öffnung ist höher als die Raumhöhe".into(), vec![o.id]));
            }
        }
        for pair in ops.windows(2) {
            if pair[0].offset_mm + pair[0].width_mm > pair[1].offset_mm {
                out.push(issue(Severity::Warning, "openings_overlap", "Öffnungen überlappen sich".into(), vec![pair[0].id, pair[1].id]));
            }
        }
        let deg = |n: Id| f.walls_at(n).count();
        if deg(w.a) == 1 || deg(w.b) == 1 {
            out.push(issue(Severity::Info, "wall_unconnected", "Wand ohne Anschluss an einem Ende".into(), vec![w.id]));
        }
    }
    // crossing walls without a shared node
    for (i, w1) in f.walls.iter().enumerate() {
        let Some((a, b)) = f.wall_points(w1) else { continue };
        for w2 in &f.walls[i + 1..] {
            if w1.a == w2.a || w1.a == w2.b || w1.b == w2.a || w1.b == w2.b {
                continue;
            }
            let Some((c, d)) = f.wall_points(w2) else { continue };
            if let Some((t, u, _)) = segment_intersect(a, b, c, d) {
                if t > 1e-3 && t < 1.0 - 1e-3 && u > 1e-3 && u < 1.0 - 1e-3 {
                    out.push(issue(Severity::Warning, "walls_cross", "Wände kreuzen sich ohne gemeinsamen Knoten".into(), vec![w1.id, w2.id]));
                }
            }
        }
    }
    let rooms = derive_rooms(f);
    for r in &f.rooms {
        if !rooms.iter().any(|d| d.meta == Some(r.id)) {
            out.push(issue(Severity::Warning, "room_open", format!("Raum «{}» ist nicht geschlossen", r.name), vec![r.id]));
        }
    }
    for n in &f.nodes {
        if f.walls_at(n.id).next().is_none() {
            out.push(issue(Severity::Info, "node_orphan", "Einzelner Knoten ohne Wand".into(), vec![n.id]));
        }
    }
    out
}
