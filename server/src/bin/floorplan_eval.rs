//! Floor plan pipeline evaluation against a test set (see `server/testset/README.md`).
//!
//! Usage: floorplan_eval [--generate] [--dir DIR] [--no-llm] [--case NAME] [--overlay OUTDIR]
//!   --generate        (re)write the synthetic cases (synth_*) into DIR first
//!   --dir DIR         test set directory (default: server/testset)
//!   --no-llm          ignore LLM_* env (CV only)
//!   --case NAME       only cases whose directory name contains NAME
//!   --overlay OUTDIR  write <case>.png overlays (result walls red, openings blue/orange) and <case>.json proposals
//!   --image FILE      just run the pipeline on FILE (no truth) and print a summary (+ overlay); repeatable
//!
//! The pipeline modules are compiled into this binary via #[path] (the server has no lib target).

#[allow(dead_code)]
#[path = "../llm.rs"]
mod llm;
#[path = "../floorplan/mod.rs"]
mod floorplan;

use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{bail, Context};
use floorplan::eval::{align, compare, Metrics};
use floorplan::AnalyzeInput;
use image::{Rgb, RgbImage};
use plan_core::{Floor, OpeningKind};
use uuid::Uuid;

struct Args {
    generate: bool,
    dir: PathBuf,
    no_llm: bool,
    case: Option<String>,
    overlay: Option<PathBuf>,
    images: Vec<PathBuf>,
}

fn parse_args() -> anyhow::Result<Args> {
    let mut a = Args {
        generate: false,
        dir: Path::new(env!("CARGO_MANIFEST_DIR")).join("testset"),
        no_llm: false,
        case: None,
        overlay: None,
        images: vec![],
    };
    let mut it = std::env::args().skip(1);
    while let Some(x) = it.next() {
        match x.as_str() {
            "--generate" => a.generate = true,
            "--no-llm" => a.no_llm = true,
            "--dir" => a.dir = it.next().context("--dir DIR")?.into(),
            "--case" => a.case = Some(it.next().context("--case NAME")?),
            "--overlay" => a.overlay = Some(it.next().context("--overlay OUTDIR")?.into()),
            "--image" => a.images.push(it.next().context("--image FILE")?.into()),
            "-h" | "--help" => {
                println!("floorplan_eval [--generate] [--dir DIR] [--no-llm] [--case NAME] [--overlay OUTDIR]");
                std::process::exit(0);
            }
            other => bail!("unbekanntes Argument {other}"),
        }
    }
    Ok(a)
}

fn generate(dir: &Path) -> anyhow::Result<()> {
    for c in floorplan::synth::cases() {
        let d = dir.join(c.name);
        std::fs::create_dir_all(&d)?;
        std::fs::write(d.join("plan.png"), &c.png)?;
        std::fs::write(d.join("truth.json"), serde_json::to_string_pretty(&c.truth)?)?;
        println!("geschrieben: {}", d.display());
    }
    Ok(())
}

fn load_truth(p: &Path) -> anyhow::Result<Floor> {
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(p)?)?;
    // accept a Floor or a whole Project (first floor)
    let v = if v.get("floors").is_some() { v["floors"][0].clone() } else { v };
    Ok(serde_json::from_value(v)?)
}

fn find_plan(d: &Path) -> Option<PathBuf> {
    ["plan.png", "plan.jpg", "plan.jpeg", "plan.PNG", "plan.JPG"].iter().map(|n| d.join(n)).find(|p| p.exists())
}

fn overlay(png: &[u8], f: &Floor, mpp: f64, out: &Path) -> anyhow::Result<()> {
    let mut img: RgbImage = image::load_from_memory(png)?.to_rgb8();
    // lighten
    for p in img.pixels_mut() {
        for c in p.0.iter_mut() {
            *c = 155 + (*c as u32 * 100 / 255) as u8;
        }
    }
    let px = |x: i32, y: i32| (x as f32 / mpp as f32, y as f32 / mpp as f32);
    for w in &f.walls {
        let (Some(a), Some(b)) = (f.node(w.a), f.node(w.b)) else { continue };
        let (pa, pb) = (px(a.x_mm, a.y_mm), px(b.x_mm, b.y_mm));
        for o in [-1.0f32, 0.0, 1.0] {
            imageproc::drawing::draw_line_segment_mut(&mut img, (pa.0 + o, pa.1), (pb.0 + o, pb.1), Rgb([220, 0, 0]));
            imageproc::drawing::draw_line_segment_mut(&mut img, (pa.0, pa.1 + o), (pb.0, pb.1 + o), Rgb([220, 0, 0]));
        }
    }
    for n in &f.nodes {
        let p = px(n.x_mm, n.y_mm);
        imageproc::drawing::draw_filled_circle_mut(&mut img, (p.0 as i32, p.1 as i32), 4, Rgb([0, 150, 0]));
    }
    for o in &f.openings {
        let Some(w) = f.wall(o.wall) else { continue };
        let Some((a, b)) = f.wall_points(w) else { continue };
        let d = plan_core::geom::norm(plan_core::geom::sub(b, a));
        let s = plan_core::geom::add(a, plan_core::geom::mul(d, o.offset_mm as f64));
        let e = plan_core::geom::add(a, plan_core::geom::mul(d, (o.offset_mm + o.width_mm) as f64));
        let c = if matches!(o.kind, OpeningKind::Window { .. }) { Rgb([0, 90, 255]) } else { Rgb([255, 150, 0]) };
        for k in -3..=3 {
            let k = k as f32;
            imageproc::drawing::draw_line_segment_mut(
                &mut img,
                ((s[0] / mpp) as f32 + k * d[1] as f32, (s[1] / mpp) as f32 - k * d[0] as f32),
                ((e[0] / mpp) as f32 + k * d[1] as f32, (e[1] / mpp) as f32 - k * d[0] as f32),
                c,
            );
        }
    }
    img.save(out)?;
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = parse_args()?;
    if args.generate {
        generate(&args.dir)?;
    }
    let llm = if args.no_llm { None } else { llm::LlmClient::from_env() };
    println!("LLM: {}", llm.as_ref().map(|l| l.model.as_str()).unwrap_or("aus (nur CV)"));
    if !args.images.is_empty() {
        if let Some(o) = &args.overlay {
            std::fs::create_dir_all(o)?;
        }
        for (i, p) in args.images.iter().enumerate() {
            let bytes = std::fs::read(p)?;
            let floor_id = Uuid::new_v4();
            let t0 = Instant::now();
            let out = floorplan::analyze(AnalyzeInput { image: bytes, corners: None, floor_id }, llm.as_ref(), &|_, _| {}).await?;
            let res = floorplan::proposal_floor(&out.proposal, floor_id);
            let rooms = plan_core::rooms::derive_rooms(&res);
            println!(
                "{}: {} Wände, {} Öffnungen, {} Räume (abgeleitet {}), {:.2} mm/px, KI {}, {:.1}s",
                p.display(),
                res.walls.len(),
                res.openings.len(),
                res.rooms.len(),
                rooms.len(),
                out.proposal.mm_per_px,
                out.proposal.used_llm,
                t0.elapsed().as_secs_f32()
            );
            for n in &out.proposal.notes {
                println!("    · {n}");
            }
            if let Some(o) = &args.overlay {
                overlay(&out.rectified_png, &res, out.proposal.mm_per_px, &o.join(format!("image{}.png", i + 1)))?;
                std::fs::write(o.join(format!("image{}.json", i + 1)), serde_json::to_string_pretty(&out.proposal)?)?;
            }
        }
        return Ok(());
    }
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&args.dir)
        .with_context(|| format!("Testset {} nicht gefunden (--generate erzeugt synthetische Fälle)", args.dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir() && p.join("truth.json").exists() && find_plan(p).is_some())
        .filter(|p| args.case.as_ref().is_none_or(|c| p.file_name().unwrap().to_string_lossy().contains(c.as_str())))
        .collect();
    dirs.sort();
    if dirs.is_empty() {
        bail!("keine Fälle in {} (je Ordner plan.png|jpg + truth.json)", args.dir.display());
    }
    if let Some(o) = &args.overlay {
        std::fs::create_dir_all(o)?;
    }
    println!(
        "{:<24} {:>11} {:>6} {:>5} {:>5} {:>5} {:>11} {:>5} {:>8} {:>7} {:>4} {:>6}",
        "Fall", "Wände ±5cm", "%", "fehl", "extra", "falsch", "Öffnungen", "Korr.", "Räume", "Mst.%", "KI", "Zeit"
    );
    let mut total = Metrics::default();
    let mut errs = Vec::new();
    for d in &dirs {
        let name = d.file_name().unwrap().to_string_lossy().to_string();
        let truth = load_truth(&d.join("truth.json")).with_context(|| format!("{name}: truth.json"))?;
        let bytes = std::fs::read(find_plan(d).unwrap())?;
        let corners: Option<[[f64; 2]; 4]> = std::fs::read_to_string(d.join("corners.json")).ok().and_then(|s| serde_json::from_str(&s).ok());
        let floor_id = Uuid::new_v4();
        let t0 = Instant::now();
        let out = floorplan::analyze(AnalyzeInput { image: bytes.clone(), corners, floor_id }, llm.as_ref(), &|_, _| {}).await;
        let out = match out {
            Ok(o) => o,
            Err(e) => {
                println!("{name:<24} FEHLER: {e:#}");
                continue;
            }
        };
        let secs = t0.elapsed().as_secs_f32();
        let res = floorplan::proposal_floor(&out.proposal, floor_id);
        let prep = floorplan::preprocess::preprocess(&bytes, corners)?;
        let (aligned, err) = align(&truth, &res, out.proposal.mm_per_px, prep.src_offset, prep.src_scale);
        let m = compare(&truth, &aligned, 50.0);
        if let Some(e) = err {
            errs.push(e.abs());
        }
        println!(
            "{:<24} {:>11} {:>5.0}% {:>5} {:>5} {:>6} {:>11} {:>5} {:>8} {:>7} {:>4} {:>5.1}s",
            name,
            format!("{}/{}", m.walls_ok, m.walls_truth),
            m.wall_share() * 100.0,
            m.walls_missing,
            m.walls_extra,
            m.walls_misplaced,
            format!("{}/{}", m.open_ok, m.open_truth),
            m.corrections(),
            format!("{}/{}", m.rooms_result, m.rooms_truth),
            err.map(|e| format!("{:+.1}", e * 100.0)).unwrap_or_else(|| "–".into()),
            if out.proposal.used_llm { "ja" } else { "nein" },
            secs
        );
        for n in &out.proposal.notes {
            println!("    · {n}");
        }
        if let Some(o) = &args.overlay {
            overlay(&out.rectified_png, &res, out.proposal.mm_per_px, &o.join(format!("{name}.png")))?;
            std::fs::write(o.join(format!("{name}.json")), serde_json::to_string_pretty(&out.proposal)?)?;
        }
        total.add(&m);
    }
    println!(
        "{:<24} {:>11} {:>5.0}% {:>5} {:>5} {:>6} {:>11} {:>5} {:>8} {:>7}",
        "TOTAL",
        format!("{}/{}", total.walls_ok, total.walls_truth),
        total.wall_share() * 100.0,
        total.walls_missing,
        total.walls_extra,
        total.walls_misplaced,
        format!("{}/{}", total.open_ok, total.open_truth),
        total.corrections(),
        format!("{}/{}", total.rooms_result, total.rooms_truth),
        if errs.is_empty() { "–".to_string() } else { format!("{:.1}", errs.iter().sum::<f64>() / errs.len() as f64 * 100.0) }
    );
    Ok(())
}
