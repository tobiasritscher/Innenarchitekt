//! Fusion of CV wall candidates with the LLM's coarse walls (pixel space) and scale estimation.

use plan_core::geom::*;

use super::cv::Segment;
use super::semantics::{SemDim, SemWall, Semantics};

/// A wall candidate (pixel or mm units, depending on stage).
#[derive(Clone, Debug, PartialEq)]
pub struct WSeg {
    pub a: P,
    pub b: P,
    pub t: f64,
    pub conf: f32,
    pub exterior: Option<bool>,
}

impl WSeg {
    pub fn len(&self) -> f64 {
        dist(self.a, self.b)
    }
    pub fn dir(&self) -> P {
        norm(sub(self.b, self.a))
    }
    pub fn scaled(&self, f: f64) -> WSeg {
        WSeg { a: mul(self.a, f), b: mul(self.b, f), t: self.t * f, ..self.clone() }
    }
}

fn median(v: &mut [f64]) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = v.len();
    Some(if n % 2 == 1 { v[n / 2] } else { (v[n / 2 - 1] + v[n / 2]) / 2.0 })
}

/// Overlap (in px along the LLM wall) if `s` matches LLM wall `l` within `tol`.
fn match_len(l: &SemWall, s: &Segment, tol: f64) -> Option<f64> {
    let dl = norm(sub(l.b, l.a));
    let ll = dist(l.a, l.b);
    let ds = norm(sub(s.b, s.a));
    if cross(dl, ds).abs() > (8f64).to_radians().sin() || ll < 1.0 {
        return None;
    }
    let mid = lerp(s.a, s.b, 0.5);
    if cross(dl, sub(mid, l.a)).abs() > tol {
        return None;
    }
    let ta = dot(sub(s.a, l.a), dl);
    let tb = dot(sub(s.b, l.a), dl);
    let ov = tb.max(ta).min(ll) - ta.min(tb).max(0.0);
    let ls = s.len();
    (ov > 0.0 && (ov >= 0.5 * ls || ov >= 0.3 * ll)).then_some(ov)
}

fn is_h(a: P, b: P) -> bool {
    let d = sub(b, a);
    d[1].abs() < 0.09 * d[0].abs()
}
fn is_v(a: P, b: P) -> bool {
    let d = sub(b, a);
    d[0].abs() < 0.09 * d[1].abs()
}

/// Robust per-axis linear fit dst ≈ s·src + o.
fn fit_axis(pairs: &[(f64, f64)], span: f64) -> Option<(f64, f64)> {
    if pairs.is_empty() {
        return None;
    }
    let mut cur: Vec<(f64, f64)> = pairs.to_vec();
    let mut res = (1.0, 0.0);
    for _ in 0..3 {
        let n = cur.len() as f64;
        let (lo, hi) = cur.iter().fold((f64::MAX, f64::MIN), |(lo, hi), p| (lo.min(p.0), hi.max(p.0)));
        res = if cur.len() >= 2 && hi - lo > 0.25 * span {
            let mx = cur.iter().map(|p| p.0).sum::<f64>() / n;
            let my = cur.iter().map(|p| p.1).sum::<f64>() / n;
            let sxx: f64 = cur.iter().map(|p| (p.0 - mx).powi(2)).sum();
            let sxy: f64 = cur.iter().map(|p| (p.0 - mx) * (p.1 - my)).sum();
            let s = (sxy / sxx).clamp(0.85, 1.18);
            (s, my - s * mx)
        } else {
            let mut d: Vec<f64> = cur.iter().map(|p| p.1 - p.0).collect();
            (1.0, median(&mut d).unwrap())
        };
        let mut r: Vec<f64> = cur.iter().map(|p| (res.0 * p.0 + res.1 - p.1).abs()).collect();
        let mr = median(&mut r.clone()).unwrap_or(0.0).max(1.0);
        let keep: Vec<(f64, f64)> = cur.iter().zip(r.iter_mut()).filter(|(_, r)| **r <= 2.5 * mr).map(|(p, _)| *p).collect();
        if keep.len() == cur.len() || keep.is_empty() {
            break;
        }
        cur = keep;
    }
    Some(res)
}

/// Correct systematic offsets/scale of LLM coordinates using axis-aligned wall matches.
pub fn align_semantics(sem: &mut Semantics, cv: &[Segment], w: f64, h: f64) {
    let tol = 0.06 * w.max(h);
    let mut px = Vec::new();
    let mut py = Vec::new();
    for l in &sem.walls {
        let hor = is_h(l.a, l.b);
        let ver = is_v(l.a, l.b);
        if !hor && !ver {
            continue;
        }
        let best = cv
            .iter()
            .filter(|s| if hor { is_h(s.a, s.b) } else { is_v(s.a, s.b) })
            .filter_map(|s| match_len(l, s, tol).map(|ov| (s, ov)))
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
        if let Some((s, _)) = best {
            if hor {
                py.push(((l.a[1] + l.b[1]) / 2.0, (s.a[1] + s.b[1]) / 2.0));
            } else {
                px.push(((l.a[0] + l.b[0]) / 2.0, (s.a[0] + s.b[0]) / 2.0));
            }
        }
    }
    let fx = fit_axis(&px, w).unwrap_or((1.0, 0.0));
    let fy = fit_axis(&py, h).unwrap_or((1.0, 0.0));
    if (fx.0 - 1.0).abs() > 1e-6 || fx.1.abs() > 0.5 || (fy.0 - 1.0).abs() > 1e-6 || fy.1.abs() > 0.5 {
        sem.map_axes(fx.0, fx.1, fy.0, fy.1);
    }
}

/// Fuse CV segments with LLM walls. Without semantics: CV-only.
pub fn fuse(cv: &[Segment], sem: Option<&Semantics>, maxdim: f64) -> Vec<WSeg> {
    let mut ts: Vec<f64> = cv.iter().map(|s| s.thickness).collect();
    let med_t = median(&mut ts).unwrap_or(8.0);
    let Some(sem) = sem.filter(|s| !s.walls.is_empty()) else {
        return cv
            .iter()
            .map(|s| {
                let short = s.len() < 4.0 * s.thickness;
                let conf = if short { 0.35 } else { 0.45 + 0.25 * s.score as f32 };
                WSeg { a: s.a, b: s.b, t: s.thickness, conf, exterior: None }
            })
            .collect();
    };
    let mut matched = vec![false; cv.len()];
    let mut ext: Vec<Option<bool>> = vec![None; cv.len()];
    let mut out = Vec::new();
    for l in &sem.walls {
        let tol = (0.015 * maxdim).max(2.0 * l.thickness.unwrap_or(med_t)).max(12.0);
        let ll = dist(l.a, l.b);
        let mut cover = 0.0;
        for (i, s) in cv.iter().enumerate() {
            if let Some(ov) = match_len(l, s, tol) {
                matched[i] = true;
                cover += ov;
                if l.exterior.is_some() {
                    ext[i] = l.exterior;
                }
            }
        }
        if cover < 0.4 * ll {
            out.push(WSeg { a: l.a, b: l.b, t: l.thickness.unwrap_or(med_t).max(2.0), conf: 0.35, exterior: l.exterior });
        }
    }
    for (i, s) in cv.iter().enumerate() {
        if matched[i] {
            out.push(WSeg { a: s.a, b: s.b, t: s.thickness, conf: (0.85 + 0.1 * s.score as f32).min(0.95), exterior: ext[i] });
        } else if s.len() >= 6.0 * s.thickness && s.thickness >= 0.6 * med_t {
            out.push(WSeg { a: s.a, b: s.b, t: s.thickness, conf: 0.55, exterior: None });
        }
    }
    out
}

// ---- scale ---------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct ScaleEstimate {
    pub mm_per_px: f64,
    /// Best dimension: endpoints (px), value (mm), text.
    pub reference: Option<(P, P, f64, String)>,
    pub notes: Vec<String>,
    /// True if derived from dimension texts.
    pub from_dims: bool,
}

pub fn fmt_m(mm: f64) -> String {
    format!("{:.2} m", mm / 1000.0)
}

/// Snap dimension endpoints to wall faces / centres / ends along the dimension axis.
fn refine_dim(d: &SemDim, walls: &[WSeg]) -> (P, P) {
    let len = dist(d.a, d.b);
    let tol = (0.02 * len).max(6.0);
    let hor = is_h(d.a, d.b);
    let ver = is_v(d.a, d.b);
    if !hor && !ver {
        return (d.a, d.b);
    }
    let ax = if hor { 0 } else { 1 };
    let mut cands = Vec::new();
    for w in walls {
        let perp_wall = if hor { is_v(w.a, w.b) } else { is_h(w.a, w.b) };
        let par_wall = if hor { is_h(w.a, w.b) } else { is_v(w.a, w.b) };
        if perp_wall {
            let c = (w.a[ax] + w.b[ax]) / 2.0;
            cands.extend([c - w.t / 2.0, c, c + w.t / 2.0]);
        } else if par_wall {
            cands.extend([w.a[ax], w.b[ax]]);
        }
    }
    let snap = |v: f64| {
        cands
            .iter()
            .map(|&c| (c, (c - v).abs()))
            .filter(|(_, e)| *e <= tol)
            .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
            .map(|(c, _)| c)
            .unwrap_or(v)
    };
    let (mut a, mut b) = (d.a, d.b);
    a[ax] = snap(a[ax]);
    b[ax] = snap(b[ax]);
    if dist(a, b) < 0.5 * len {
        return (d.a, d.b);
    }
    (a, b)
}

/// "1:100", "M 1 : 50", "Massstab 1:100 (A4)" → 100.
pub fn parse_scale_ratio(text: &str) -> Option<f64> {
    let i = text.find(':')?;
    let before = text[..i].trim_end();
    if !before.ends_with('1') || before[..before.len() - 1].ends_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    let digits: String = text[i + 1..].trim_start().chars().take_while(|c| c.is_ascii_digit() || *c == '\'' || *c == '’').filter(|c| c.is_ascii_digit()).collect();
    let n: f64 = digits.parse().ok()?;
    (5.0..=2000.0).contains(&n).then_some(n)
}

/// Like [`estimate_from_plan`], but if the plan states its scale (e.g. 1:100) and the paper size of a pixel
/// is known (PDF upload), that printed scale is exact and wins – unless dimensions or the rough estimate
/// clearly contradict it (PDF not to scale).
pub fn estimate_scale(sem: Option<&Semantics>, walls: &[WSeg], door_widths_px: &[f64], paper_mm_per_px: Option<f64>) -> ScaleEstimate {
    let mut base = estimate_from_plan(sem, walls, door_widths_px);
    let ratio = sem.and_then(|s| s.scale_text.as_deref()).and_then(parse_scale_ratio);
    let (Some(paper), Some(n)) = (paper_mm_per_px.filter(|p| p.is_finite() && *p > 0.0), ratio) else { return base };
    let printed = paper * n;
    base.notes.retain(|t| !t.contains("ohne Bildauflösung"));
    let dev = (base.mm_per_px / printed - 1.0).abs();
    if base.from_dims {
        if dev <= 0.06 {
            base.mm_per_px = printed;
            base.notes.push(format!("Massstab 1:{n} aus dem PDF übernommen, durch die Masszahlen bestätigt."));
        } else {
            base.notes.push(format!("Massstab 1:{n} aus dem PDF passt nicht zu den Masszahlen ({:.0} % Abweichung) – Masszahlen verwendet.", dev * 100.0));
        }
        return base;
    }
    if dev > 0.35 {
        base.notes.push(format!("Massstab 1:{n} aus dem PDF weicht stark von der Schätzung ab – PDF vermutlich nicht massstäblich, Schätzung verwendet."));
        return base;
    }
    base.notes.retain(|t| !t.starts_with("Keine Masszahlen erkannt") && !t.starts_with("Kein Massstab"));
    base.notes.push(format!("Massstab 1:{n} aus der PDF-Seitengrösse übernommen – stimmt, wenn das PDF massstäblich ist. Referenzmass kurz prüfen."));
    ScaleEstimate { mm_per_px: printed, reference: None, notes: base.notes, from_dims: true }
}

/// Estimate mm/px from dimensions (median, outliers > 8 % flagged), else from wall thickness / door gaps.
pub fn estimate_from_plan(sem: Option<&Semantics>, walls: &[WSeg], door_widths_px: &[f64]) -> ScaleEstimate {
    let mut notes = Vec::new();
    if let Some(sem) = sem {
        let dims: Vec<(P, P, f64, String)> = sem
            .dims
            .iter()
            .map(|d| {
                let (a, b) = refine_dim(d, walls);
                let text = if d.text.is_empty() { fmt_m(d.value_mm) } else { d.text.clone() };
                (a, b, d.value_mm, text)
            })
            .filter(|(a, b, v, _)| dist(*a, *b) >= 20.0 && *v >= 200.0 && *v <= 60_000.0)
            .collect();
        let mut ratios: Vec<f64> = dims.iter().map(|(a, b, v, _)| v / dist(*a, *b)).collect();
        if let Some(med) = median(&mut ratios.clone()) {
            let mut good = Vec::new();
            for (i, (a, b, v, text)) in dims.iter().enumerate() {
                let r = v / dist(*a, *b);
                let dev = (r / med - 1.0).abs();
                if dev > 0.08 {
                    notes.push(format!("Masszahl «{text}» weicht {:.0} % vom ermittelten Massstab ab und wurde ignoriert.", dev * 100.0));
                } else {
                    good.push(i);
                }
            }
            ratios = good.iter().map(|&i| dims[i].2 / dist(dims[i].0, dims[i].1)).collect();
            let mm_per_px = median(&mut ratios).unwrap_or(med);
            // reference: long and close to the scale
            let best = good.iter().copied().max_by(|&i, &j| {
                let score = |k: usize| {
                    let (a, b, v, _) = &dims[k];
                    let dev = (v / dist(*a, *b) / mm_per_px - 1.0).abs();
                    dist(*a, *b) * (1.0 - 8.0 * dev)
                };
                score(i).partial_cmp(&score(j)).unwrap()
            });
            if good.len() == 1 {
                notes.push("Massstab aus nur einer Masszahl ermittelt – bitte mit dem Referenzmass prüfen.".into());
            }
            return ScaleEstimate { mm_per_px, reference: best.map(|i| dims[i].clone()), notes, from_dims: true };
        }
        if let Some(st) = &sem.scale_text {
            notes.push(format!("Massstab {st} auf dem Plan erkannt, ohne Bildauflösung aber nicht verwendbar."));
        }
    }
    // fallback: wall thickness clusters and door widths
    let mut est = Vec::new();
    let mut weighted: Vec<(f64, f64)> = walls.iter().filter(|w| w.conf >= 0.4).map(|w| (w.t, w.len())).collect();
    weighted.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    let total: f64 = weighted.iter().map(|w| w.1).sum();
    let pct = |q: f64| {
        let mut acc = 0.0;
        for (t, l) in &weighted {
            acc += l;
            if acc >= q * total {
                return *t;
            }
        }
        weighted.last().map(|w| w.0).unwrap_or(0.0)
    };
    if total > 0.0 {
        let (thin, thick) = (pct(0.2), pct(0.85));
        let t_est = if thick / thin >= 1.7 { (120.0 / thin * 300.0 / thick).sqrt() } else { 175.0 / pct(0.5) };
        est.push((t_est, 0.35));
    }
    let mut dw: Vec<f64> = door_widths_px.to_vec();
    if dw.len() >= 2 {
        let m = median(&mut dw).unwrap();
        est.push((900.0 / m, 0.65));
    }
    let mm_per_px = if est.is_empty() {
        notes.push("Kein Massstab erkennbar – Annahme 10 mm/px. Bitte Referenzmass setzen.".into());
        10.0
    } else {
        let wsum: f64 = est.iter().map(|e| e.1).sum();
        notes.push(if dw.len() >= 2 {
            "Keine Masszahlen erkannt – Massstab grob aus Türbreiten (≈ 90 cm) und Wandstärken geschätzt. Bitte Referenzmass bestätigen.".into()
        } else {
            "Keine Masszahlen erkannt – Massstab grob aus Wandstärken (aussen ≈ 30 cm, innen ≈ 12 cm) geschätzt. Bitte Referenzmass bestätigen.".into()
        });
        est.iter().map(|e| e.0 * e.1).sum::<f64>() / wsum
    };
    ScaleEstimate { mm_per_px: mm_per_px.clamp(0.5, 200.0), reference: None, notes, from_dims: false }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(a: P, b: P, t: f64) -> Segment {
        Segment { a, b, thickness: t, score: 1.0 }
    }

    #[test]
    fn parses_scale_ratio() {
        assert_eq!(parse_scale_ratio("1:100"), Some(100.0));
        assert_eq!(parse_scale_ratio("Massstab 1 : 50 (A4)"), Some(50.0));
        assert_eq!(parse_scale_ratio("M 1:1'000"), Some(1000.0));
        assert_eq!(parse_scale_ratio("11:100"), None);
        assert_eq!(parse_scale_ratio("100"), None);
    }

    #[test]
    fn printed_scale_from_pdf() {
        let walls: Vec<WSeg> = vec![];
        let sem = Semantics { scale_text: Some("1:100 (A4)".into()), ..Default::default() };
        // rough estimate from two doors ≈ 16.3 mm/px (10 % off) → printed 0.1485 mm × 100 wins
        let doors = [900.0 / 16.3, 900.0 / 16.3];
        let s = estimate_scale(Some(&sem), &walls, &doors, Some(0.1485));
        assert!((s.mm_per_px - 14.85).abs() < 1e-9, "{}", s.mm_per_px);
        assert!(s.from_dims);
        // without paper size: unchanged behaviour
        let s = estimate_scale(Some(&sem), &walls, &doors, None);
        assert!(!s.from_dims);
        // A3 printed as A4 (factor 1.41) contradicts the estimate → ignored
        let s = estimate_scale(Some(&sem), &walls, &doors, Some(0.1485 / 1.6));
        assert!(!s.from_dims);
    }

    #[test]
    fn fusion_snaps_and_flags() {
        let cv = vec![
            seg([100.0, 100.0], [900.0, 100.0], 20.0),
            seg([100.0, 100.0], [100.0, 700.0], 20.0),
            seg([400.0, 300.0], [400.0, 330.0], 3.0), // short noise
            seg([500.0, 100.0], [500.0, 700.0], 10.0), // interior, LLM missed
        ];
        let sem = Semantics {
            walls: vec![
                SemWall { a: [110.0, 108.0], b: [880.0, 104.0], thickness: Some(18.0), exterior: Some(true) },
                SemWall { a: [104.0, 110.0], b: [96.0, 690.0], thickness: None, exterior: Some(true) },
                SemWall { a: [100.0, 700.0], b: [900.0, 700.0], thickness: None, exterior: Some(true) }, // no CV
            ],
            ..Default::default()
        };
        let f = fuse(&cv, Some(&sem), 1000.0);
        let top = f.iter().find(|w| w.a == [100.0, 100.0] && w.b[1] == 100.0).unwrap();
        assert!(top.conf > 0.8 && top.exterior == Some(true));
        let llm_only = f.iter().find(|w| w.a == [100.0, 700.0]).unwrap();
        assert!(llm_only.conf <= 0.4);
        let interior = f.iter().find(|w| w.a == [500.0, 100.0]).unwrap();
        assert!(interior.conf > 0.4 && interior.conf < 0.8);
        assert!(!f.iter().any(|w| w.a == [400.0, 300.0]));
    }

    #[test]
    fn llm_offset_is_corrected() {
        let cv = vec![
            seg([100.0, 100.0], [900.0, 100.0], 20.0),
            seg([100.0, 700.0], [900.0, 700.0], 20.0),
            seg([100.0, 100.0], [100.0, 700.0], 20.0),
            seg([900.0, 100.0], [900.0, 700.0], 20.0),
            seg([500.0, 100.0], [500.0, 700.0], 10.0),
        ];
        // LLM coordinates scaled by 0.95 and shifted
        let f = |p: P| [p[0] * 0.95 + 12.0, p[1] * 0.95 - 8.0];
        let mut sem = Semantics {
            walls: cv.iter().map(|s| SemWall { a: f(s.a), b: f(s.b), thickness: None, exterior: None }).collect(),
            rooms: vec![super::super::semantics::SemRoom { name: "Zimmer".into(), pos: f([300.0, 400.0]), area_m2: None }],
            ..Default::default()
        };
        align_semantics(&mut sem, &cv, 1000.0, 800.0);
        let p = sem.rooms[0].pos;
        assert!((p[0] - 300.0).abs() < 2.0 && (p[1] - 400.0).abs() < 2.0, "{p:?}");
    }

    #[test]
    fn scale_from_dims_with_outlier() {
        let walls = vec![
            WSeg { a: [100.0, 100.0], b: [100.0, 600.0], t: 20.0, conf: 0.9, exterior: None },
            WSeg { a: [525.0, 100.0], b: [525.0, 600.0], t: 10.0, conf: 0.9, exterior: None },
        ];
        let d = |v: f64, a: P, b: P| SemDim { value_mm: v, a, b, text: String::new() };
        let sem = Semantics {
            dims: vec![
                d(4250.0, [92.0, 50.0], [523.0, 50.0]), // snaps to 90 / 525 → 435 px? (face/centre)
                d(5000.0, [50.0, 100.0], [50.0, 600.0]),
                d(9999.0, [50.0, 100.0], [50.0, 600.0]), // outlier
            ],
            ..Default::default()
        };
        let s = estimate_scale(Some(&sem), &walls, &[], None);
        assert!(s.from_dims);
        assert!((s.mm_per_px - 10.0).abs() < 0.3, "{}", s.mm_per_px);
        assert!(s.notes.iter().any(|n| n.contains("weicht")));
        assert!(s.reference.is_some());
    }

    #[test]
    fn scale_fallback_from_thickness() {
        let walls = vec![
            WSeg { a: [0.0, 0.0], b: [1000.0, 0.0], t: 30.0, conf: 0.6, exterior: None },
            WSeg { a: [0.0, 0.0], b: [0.0, 1000.0], t: 30.0, conf: 0.6, exterior: None },
            WSeg { a: [500.0, 0.0], b: [500.0, 1000.0], t: 12.0, conf: 0.6, exterior: None },
        ];
        let s = estimate_scale(None, &walls, &[], None);
        assert!(!s.from_dims);
        assert!((s.mm_per_px - 10.0).abs() < 0.5, "{}", s.mm_per_px);
    }
}
