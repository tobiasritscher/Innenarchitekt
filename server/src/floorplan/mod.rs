//! Floor plan import pipeline: preprocess → classic CV line detection → LLM semantics → fusion →
//! scale → cleanup → proposal (ops + confidence).
//! OWNER: floorplan-pipeline agent. `analyze` signature is a contract used by the backend job worker.
//!
//! Stages (all CPU work runs in `spawn_blocking`; CV and the LLM call run concurrently):
//! 1. `preprocess`: decode, perspective warp (corners) or border trim, ≤ 2048 px, binarise.
//! 2. `cv`: thick-stroke wall segments with thickness (run-length bands + diagonal components).
//! 3. `semantics`: LLM vision → rooms, doors, windows, dimensions, north arrow, coarse walls.
//! 4. `fusion`: LLM walls snapped onto CV segments; confidence per source.
//! 5. scale from dimension texts (median, outliers noted) or wall thickness / door widths.
//! 6. `cleanup` in mm: axis snapping, merging, openings from wall gaps, junctions, rooms.
//! 7. proposal ops (PutNode/PutWall/PutOpening/PutRoom), verified against an empty floor.

#![allow(dead_code)] // eval/synth helpers are used by tests and the floorplan_eval binary

pub mod cleanup;
pub mod cv;
pub mod eval;
pub mod fusion;
pub mod preprocess;
pub mod semantics;
pub mod synth;

use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use image::DynamicImage;
pub use plan_core::geom::{dist, norm, P};
use plan_core::import::{FloorplanProposal, ReferenceSuggestion};
use plan_core::{Op, Project, RefImage};
use uuid::Uuid;

use crate::llm::LlmClient;
use cleanup::{CleanInput, OCand, OKind, RoomCand, Sampler};
use cv::CvResult;
use fusion::{ScaleEstimate, WSeg};
use preprocess::{Mask, Prepared};
use semantics::Semantics;

pub struct AnalyzeInput {
    /// Raw uploaded image bytes (PNG or JPEG).
    pub image: Vec<u8>,
    /// Optional perspective corners in source-image pixels: TL, TR, BR, BL.
    pub corners: Option<[[f64; 2]; 4]>,
    /// Floor the proposal ops refer to.
    pub floor_id: Uuid,
}

pub struct AnalyzeOutput {
    /// Rectified (cropped, perspective-corrected, max 2048 px) image as PNG. The backend stores it
    /// and sets `proposal.image.url`.
    pub rectified_png: Vec<u8>,
    /// Proposal; `image.url` is left empty for the caller to fill.
    pub proposal: FloorplanProposal,
}

/// Progress callback: (fraction 0..1, German status message).
pub type Progress<'a> = &'a (dyn Fn(f32, &str) + Send + Sync);

/// Test/eval knobs.
#[derive(Clone, Debug, Default)]
pub struct Options {
    /// Force the scale (skips estimation).
    pub mm_per_px: Option<f64>,
}

const LLM_IMAGE_MAX_DEFAULT: u32 = 1568;

pub async fn analyze(input: AnalyzeInput, llm: Option<&LlmClient>, progress: Progress<'_>) -> anyhow::Result<AnalyzeOutput> {
    let started = std::time::Instant::now();
    let AnalyzeInput { image, corners, floor_id } = input;
    progress(0.02, "Bild entzerren");
    let (prep, cvres, deskew_note) = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        let prep = preprocess::preprocess(&image, corners)?;
        Ok(detect_deskewed(prep))
    })
    .await
    .context("Vorverarbeitung abgebrochen")??;
    let prep = Arc::new(prep);
    progress(0.15, "Wände erkennen");
    let mut notes: Vec<String> = deskew_note.into_iter().collect();
    let sem = match llm {
        Some(llm) => {
            progress(0.2, "KI analysiert den Plan");
            let p2 = prep.clone();
            let (png, w, h, factor) = tokio::task::spawn_blocking(move || llm_image(&p2)).await??;
            let prompt = semantics::user_prompt(w, h);
            let fut = llm.structured(semantics::SYSTEM, &prompt, Some(&png), semantics::TOOL_NAME, semantics::schema());
            tokio::pin!(fut);
            let mut tick = tokio::time::interval(Duration::from_secs(2));
            tick.tick().await;
            let mut frac = 0.2f32;
            let res = loop {
                tokio::select! {
                    r = &mut fut => break r,
                    _ = tick.tick() => {
                        frac = (frac + 0.012).min(0.7);
                        progress(frac, "KI analysiert den Plan");
                    }
                }
            };
            match res {
                Ok(v) => Some(semantics::parse(&v).scaled(factor)),
                Err(e) => {
                    tracing::warn!("floorplan LLM stage failed: {e:#}");
                    notes.push("KI-Analyse fehlgeschlagen – Ergebnis nur aus klassischer Bilderkennung (ohne Raumnamen und Masszahlen).".to_string());
                    None
                }
            }
        }
        None => None,
    };

    progress(0.75, "Zusammenführen");
    let p2 = prep.clone();
    let stage = tokio::task::spawn_blocking(move || fuse_stage(&p2, &cvres, sem, &Options::default())).await?;

    progress(0.85, "Bereinigen");
    let p3 = prep.clone();
    let mut out = tokio::task::spawn_blocking(move || clean_stage(&p3, stage, floor_id)).await??;
    out.proposal.notes.splice(0..0, notes);
    tracing::info!("floorplan analysed in {:.1}s (llm: {})", started.elapsed().as_secs_f32(), out.proposal.used_llm);
    progress(1.0, "Fertig");
    Ok(out)
}

/// Synchronous pipeline for tests / eval (semantics supplied directly instead of via the LLM).
pub fn analyze_blocking(input: &AnalyzeInput, sem: Option<Semantics>, opts: &Options) -> anyhow::Result<AnalyzeOutput> {
    let prep = preprocess::preprocess(&input.image, input.corners)?;
    let (prep, cvres, _) = detect_deskewed(prep);
    let stage = fuse_stage(&prep, &cvres, sem, opts);
    clean_stage(&prep, stage, input.floor_id)
}

/// CV wall detection; a slightly rotated plan (scan/photo, 0.2°–5°) is straightened first.
pub fn detect_deskewed(prep: Prepared) -> (Prepared, CvResult, Option<String>) {
    let cvres = cv::detect(&prep.ink);
    if let Some(skew) = cv::dominant_skew(&cvres.segments) {
        if skew.abs() >= 0.2 {
            if let Ok(p2) = preprocess::rotated(&prep, -skew) {
                let c2 = cv::detect(&p2.ink);
                let note = format!("Plan um {:.1}° gerade gerichtet.", skew.abs());
                return (p2, c2, Some(note));
            }
        }
    }
    (prep, cvres, None)
}

/// Downscaled PNG for the LLM: (png, width, height, factor rectified px / llm px).
pub fn llm_image(prep: &Prepared) -> anyhow::Result<(Vec<u8>, u32, u32, f64)> {
    let max = std::env::var("LLM_IMAGE_MAX_PX").ok().and_then(|v| v.parse::<u32>().ok()).unwrap_or(LLM_IMAGE_MAX_DEFAULT).clamp(512, 4096);
    let (w, h) = prep.rgb.dimensions();
    let s = (max as f64 / w.max(h) as f64).min(1.0);
    let (nw, nh) = (((w as f64 * s).round() as u32).max(1), ((h as f64 * s).round() as u32).max(1));
    let img = if s < 1.0 { image::imageops::resize(&prep.rgb, nw, nh, image::imageops::FilterType::Triangle) } else { prep.rgb.clone() };
    let png = preprocess::encode_png(&DynamicImage::ImageRgb8(img))?;
    Ok((png, nw, nh, w as f64 / nw as f64))
}

pub struct Stage {
    walls_mm: Vec<WSeg>,
    openings: Vec<OCand>,
    rooms: Vec<RoomCand>,
    scale: ScaleEstimate,
    north: Option<f32>,
    used_llm: bool,
}

/// Door widths (px) from gaps between collinear CV walls that show a drawn door leaf.
fn door_gaps_px(walls: &[WSeg], mask: &Mask) -> Vec<f64> {
    let s = Sampler { mask, px_per_mm: 1.0 };
    let mut out = Vec::new();
    for (i, a) in walls.iter().enumerate() {
        for (j, b) in walls.iter().enumerate() {
            if i == j || plan_core::geom::cross(a.dir(), b.dir()).abs() > 0.03 {
                continue;
            }
            let d = a.dir();
            let off = plan_core::geom::cross(d, plan_core::geom::sub(plan_core::geom::lerp(b.a, b.b, 0.5), a.a));
            if off.abs() > 0.5 * a.t.max(b.t) {
                continue;
            }
            let ta = plan_core::geom::dot(plan_core::geom::sub(b.a, a.a), d).min(plan_core::geom::dot(plan_core::geom::sub(b.b, a.a), d));
            let gap = ta - a.len();
            let tm = a.t.min(b.t);
            if gap < 2.5 * tm || gap > 15.0 * tm.max(4.0) {
                continue;
            }
            let p0 = plan_core::geom::add(a.a, plan_core::geom::mul(d, a.len()));
            let p1 = plan_core::geom::add(a.a, plan_core::geom::mul(d, ta));
            let (cov, _) = s.gap_stats(p0, p1, tm / 2.0);
            if cov <= 0.35 && s.door_leaf(p0, p1, tm / 2.0).is_some() {
                out.push(gap);
            }
        }
    }
    out
}

pub fn fuse_stage(prep: &Prepared, cvres: &CvResult, mut sem: Option<Semantics>, opts: &Options) -> Stage {
    let (w, h) = (prep.ink.w as f64, prep.ink.h as f64);
    if let Some(s) = sem.as_mut() {
        fusion::align_semantics(s, &cvres.segments, w, h);
    }
    let walls_px = fusion::fuse(&cvres.segments, sem.as_ref(), w.max(h));
    let scale = match opts.mm_per_px {
        Some(m) => ScaleEstimate { mm_per_px: m, reference: None, notes: vec![], from_dims: true },
        None => {
            let mut doors = door_gaps_px(&walls_px, &prep.light);
            if let Some(s) = &sem {
                doors.extend(s.doors.iter().map(|d| d.width));
            }
            fusion::estimate_scale(sem.as_ref(), &walls_px, &doors)
        }
    };
    let m = scale.mm_per_px;
    let walls_mm = walls_px.iter().map(|s| s.scaled(m)).collect();
    let mut openings = Vec::new();
    let mut rooms = Vec::new();
    let mut north = None;
    if let Some(s) = &sem {
        let mm = |p: P| [p[0] * m, p[1] * m];
        for d in &s.doors {
            openings.push(OCand { center: mm(d.pos), width: d.width * m, kind: OKind::Door, hinge: d.hinge.map(mm), into: d.opens_into.map(mm), conf: 0.5, llm: true });
        }
        for wdw in &s.windows {
            openings.push(OCand { center: mm(wdw.pos), width: wdw.width * m, kind: OKind::Window, hinge: None, into: None, conf: 0.5, llm: true });
        }
        for r in &s.rooms {
            rooms.push(RoomCand { name: r.name.clone(), pos: mm(r.pos) });
        }
        north = s.north_deg.map(|d| d as f32);
    }
    Stage { walls_mm, openings, rooms, scale, north, used_llm: sem.is_some() }
}

pub fn clean_stage(prep: &Prepared, st: Stage, floor_id: Uuid) -> anyhow::Result<AnalyzeOutput> {
    let m = st.scale.mm_per_px;
    let out = cleanup::cleanup(CleanInput {
        walls: st.walls_mm,
        openings: st.openings,
        rooms: st.rooms,
        mask: &prep.light,
        mm_per_px: m,
        floor_id,
        used_llm: st.used_llm,
    });
    let f = &out.floor;
    let mut ops = Vec::new();
    for n in &f.nodes {
        ops.push(Op::PutNode { floor: floor_id, node: *n });
    }
    for w in &f.walls {
        ops.push(Op::PutWall { floor: floor_id, wall: w.clone() });
    }
    for o in &f.openings {
        ops.push(Op::PutOpening { floor: floor_id, opening: o.clone() });
    }
    for r in &f.rooms {
        ops.push(Op::PutRoom { floor: floor_id, room: r.clone() });
    }
    // must apply cleanly to an empty floor
    let mut check = Project::new(Uuid::new_v4(), "check");
    check.floors = vec![plan_core::Floor::new(floor_id, "EG")];
    plan_core::apply_batch(&mut check, &ops).map_err(|(i, e)| anyhow::anyhow!("Interner Fehler: Vorschlag nicht anwendbar (Op {i}: {e})"))?;

    let mut notes = st.scale.notes.clone();
    notes.extend(out.notes.iter().cloned());
    let low = out.confidence.values().filter(|c| **c < 0.5).count();
    notes.push(format!(
        "Erkannt: {} Wände, {} Öffnungen, {} Räume{}.",
        f.walls.len(),
        f.openings.len(),
        f.rooms.len(),
        if low > 0 { format!(" ({low} Elemente unsicher – bitte prüfen)") } else { String::new() }
    ));
    let reference = st.scale.reference.as_ref().map(|(a, b, v, _text)| ReferenceSuggestion {
        a: [a[0] * m, a[1] * m],
        b: [b[0] * m, b[1] * m],
        length_mm: *v,
        label: format!("Masslinie {}", fusion::fmt_m(*v)),
    });
    let image = RefImage {
        url: String::new(),
        width_px: prep.rgb.width(),
        height_px: prep.rgb.height(),
        origin_mm: [0.0, 0.0],
        mm_per_px: m,
        opacity: 0.5,
        visible: true,
    };
    Ok(AnalyzeOutput {
        rectified_png: prep.png.clone(),
        proposal: FloorplanProposal {
            ops,
            confidence: out.confidence,
            reference,
            north_angle_deg: st.north,
            image,
            mm_per_px: m,
            notes,
            used_llm: st.used_llm,
        },
    })
}

/// Floor reconstructed from proposal ops (for eval / tests).
pub fn proposal_floor(p: &FloorplanProposal, floor_id: Uuid) -> plan_core::Floor {
    let mut prj = Project::new(Uuid::new_v4(), "eval");
    prj.floors = vec![plan_core::Floor::new(floor_id, "EG")];
    let _ = plan_core::apply_batch(&mut prj, &p.ops);
    prj.floors.remove(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use plan_core::rooms::derive_rooms;

    #[test]
    fn synthetic_cv_only_within_5cm() {
        for case in synth::cases() {
            let floor_id = Uuid::new_v4();
            let input = AnalyzeInput { image: case.png.clone(), corners: None, floor_id };
            let out = analyze_blocking(&input, None, &Options { mm_per_px: Some(case.mm_per_px) }).unwrap();
            assert!(!out.proposal.used_llm);
            let res = proposal_floor(&out.proposal, floor_id);
            let pp = preprocess::preprocess(&case.png, None).unwrap();
            let (aligned, _) = eval::align(&case.truth, &res, out.proposal.mm_per_px, pp.src_offset, pp.src_scale);
            let m = eval::compare(&case.truth, &aligned, 50.0);
            eprintln!("{}: {m:?}", case.name);
            assert_eq!(m.walls_ok, m.walls_truth, "{}: walls {m:?}", case.name);
            assert_eq!(derive_rooms(&res).len(), derive_rooms(&case.truth).len(), "{}: rooms", case.name);
            assert!(m.open_ok * 10 >= m.open_truth * 7, "{}: openings {m:?}", case.name);
        }
    }

    #[test]
    fn synthetic_with_mocked_llm() {
        let case = &synth::cases()[0];
        let floor_id = Uuid::new_v4();
        let sem = synth::mock_semantics(case);
        let input = AnalyzeInput { image: case.png.clone(), corners: None, floor_id };
        let out = analyze_blocking(&input, Some(sem), &Options::default()).unwrap();
        let p = &out.proposal;
        assert!(p.used_llm);
        let pp = preprocess::preprocess(&case.png, None).unwrap();
        let true_mpp = case.mm_per_px / pp.src_scale;
        assert!((p.mm_per_px / true_mpp - 1.0).abs() < 0.02, "scale {} vs {}", p.mm_per_px, true_mpp);
        assert!(p.reference.as_ref().unwrap().label.starts_with("Masslinie"));
        assert_eq!(p.north_angle_deg, Some(30.0));
        let res = proposal_floor(p, floor_id);
        let names: Vec<&str> = res.rooms.iter().map(|r| r.name.as_str()).collect();
        assert!(names.contains(&"Bad"), "{names:?}");
        let bad = res.rooms.iter().find(|r| r.name == "Bad").unwrap();
        assert_eq!(bad.floor_material, "tile:white");
        // anchors resolve to derived rooms
        let derived = derive_rooms(&res);
        for r in &res.rooms {
            assert!(derived.iter().any(|d| d.meta == Some(r.id)), "{} not inside a room", r.name);
        }
        let (aligned, err) = eval::align(&case.truth, &res, p.mm_per_px, pp.src_offset, pp.src_scale);
        let m = eval::compare(&case.truth, &aligned, 50.0);
        assert!(err.unwrap().abs() < 0.02);
        assert!(m.walls_ok * 10 >= m.walls_truth * 8, "{m:?}");
        assert!(p.ops.iter().all(|o| matches!(o, Op::PutNode { .. } | Op::PutWall { .. } | Op::PutOpening { .. } | Op::PutRoom { .. })));
    }
}

#[cfg(test)]
mod robustness_tests {
    use super::*;

    /// JPEG artefacts, blur and a slight rotation (≈ 0.6°, like a scan) must not break the CV path.
    #[test]
    fn degraded_jpeg_rotated() {
        let case = &synth::cases()[1];
        let img = image::load_from_memory(&case.png).unwrap().to_rgb8();
        let img = imageproc::geometric_transformations::rotate_about_center(
            &img,
            0.6f32.to_radians(),
            imageproc::geometric_transformations::Interpolation::Bilinear,
            image::Rgb([255, 255, 255]),
        );
        let img = image::imageops::blur(&img, 0.8);
        let mut jpg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpg, 55).encode_image(&img).unwrap();
        if let Ok(p) = std::env::var("DEGRADED_OUT") { std::fs::write(p, &jpg).unwrap(); }
        let floor_id = Uuid::new_v4();
        let pp = preprocess::preprocess(&jpg, None).unwrap();
        let out = analyze_blocking(&AnalyzeInput { image: jpg.clone(), corners: None, floor_id }, None, &Options { mm_per_px: Some(case.mm_per_px / pp.src_scale) }).unwrap();
        let res = proposal_floor(&out.proposal, floor_id);
        // rotation breaks the exact truth mapping → bbox/ICP alignment (no reference image)
        let mut truth = case.truth.clone();
        truth.reference_image = None;
        let (aligned, _) = eval::align(&truth, &res, out.proposal.mm_per_px, pp.src_offset, pp.src_scale);
        let m = eval::compare(&truth, &aligned, 50.0);
        eprintln!("degraded: {m:?}");
        assert!(m.wall_share() >= 0.9, "{m:?}");
        assert_eq!(m.rooms_result, m.rooms_truth, "{m:?}");
    }

    /// The async entry point (CV only) reports German progress and yields an applicable proposal.
    #[tokio::test]
    async fn analyze_async_cv_only() {
        let case = &synth::cases()[0];
        let msgs = std::sync::Mutex::new(Vec::<String>::new());
        let cb = |f: f32, m: &str| {
            assert!((0.0..=1.0).contains(&f));
            msgs.lock().unwrap().push(m.to_string());
        };
        let floor_id = Uuid::new_v4();
        let out = analyze(AnalyzeInput { image: case.png.clone(), corners: None, floor_id }, None, &cb).await.unwrap();
        let msgs = msgs.into_inner().unwrap();
        for m in ["Bild entzerren", "Wände erkennen", "Zusammenführen", "Bereinigen"] {
            assert!(msgs.iter().any(|x| x == m), "{msgs:?}");
        }
        assert!(!out.proposal.used_llm && out.proposal.image.url.is_empty());
        assert_eq!(out.proposal.image.width_px, image::load_from_memory(&out.rectified_png).unwrap().width());
        assert!(!out.proposal.ops.is_empty());
    }
}
