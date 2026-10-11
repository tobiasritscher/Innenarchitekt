//! Import of IKEA planner designs (PAX, BESTÅ, PLATSA … «storageone» planners) from a share link like
//! `https://www.ikea.com/addon-app/storageone/pax/web/latest/ch/de/#/vpc/33ZNPWP`.
//!
//! The planner stores designs as «VPC» configurations in IKEA's DEXF API. Their ICF part lists every
//! article with world-space centre (mm, y up), dimensions, rotation and colours. We rebuild the
//! combination from simple solids (carcasses as 18 mm panels, fronts, handles, shelves, drawers) into
//! one GLB, so it is a single movable piece of furniture. Like the product import, this uses an
//! unofficial interface and may break; it is isolated here.

use std::collections::BTreeMap;

use object_store::path::Path as ObjPath;
use object_store::PutPayload;
use plan_core::assets::{Asset, AssetKind, IkeaMeta};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

/// Public DEXF API keys of the planner front-ends (shipped in IKEA's planner JavaScript).
const DEXF_KEYS: &[(&str, &str)] = &[
    ("pax", "5bf70f96-11aa-4773-a4dd-f67330b651ee"),
    ("besta", "86f2d515-3a76-4195-bdab-60d072c42336"),
    ("platsa", "4d059903-e6cc-4c12-be6a-eb33693057e0"),
    ("eket", "79961114-8ac4-4cbc-9a8b-9faa71c2fb48"),
    ("ivar", "978146f3-3069-4176-a945-460c494b7a84"),
    ("billy", "fec13e30-148b-441d-967a-5be851f923b1"),
    ("kallax", "cdb6dd5c-ec98-4e0e-8160-966dae6fa398"),
    ("boaxel", "a56392c3-013d-4dc0-b5a7-85618b1d9dc0"),
    ("smastad", "0ec63d26-ddc8-496e-a13e-3fee6338379e"),
    ("trofast", "41cd8913-c5fb-46ae-a5a2-41420f3035ca"),
    ("metod", "a4e8f4f9-3847-4c93-8387-4bb61cc48ac3"),
];

#[derive(Debug, PartialEq)]
pub struct PlannerLink {
    pub range: String,
    pub retail_unit: String,
    pub locale: String,
    pub code: String,
    pub url: String,
}

/// Recognise a planner share link (`…/addon-app/storageone/<range>/web/latest/<cc>/<lang>/…#/vpc/<CODE>` or `#/u/<CODE>`).
pub fn parse_link(input: &str) -> Option<PlannerLink> {
    let s = input.trim();
    let i = s.find("/addon-app/storageone/")?;
    let rest = &s[i + "/addon-app/storageone/".len()..];
    let range = rest.split('/').next()?.to_lowercase();
    let segs: Vec<&str> = rest.split(['#', '?']).next()?.split('/').filter(|x| !x.is_empty()).collect();
    // range / web / latest / cc / lang
    let (cc, lang) = match segs.iter().position(|x| *x == "latest") {
        Some(p) if segs.len() > p + 2 => (segs[p + 1].to_string(), segs[p + 2].to_string()),
        _ => ("ch".into(), "de".into()),
    };
    let frag = s.split('#').nth(1).unwrap_or("");
    let code = frag
        .split('/')
        .collect::<Vec<_>>()
        .windows(2)
        .find(|w| w[0] == "vpc" || w[0] == "u")
        .map(|w| w[1].split(['?', '&']).next().unwrap_or("").to_uppercase())?;
    if code.len() < 4 || !code.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    Some(PlannerLink {
        url: format!("https://www.ikea.com/addon-app/storageone/{range}/web/latest/{cc}/{lang}/#/vpc/{code}"),
        retail_unit: cc.to_uppercase(),
        locale: format!("{}-{}", lang.to_lowercase(), cc.to_uppercase()),
        range,
        code,
    })
}

fn range_label(r: &str) -> String {
    match r {
        "besta" => "BESTÅ".into(),
        "smastad" => "SMÅSTAD".into(),
        other => other.to_uppercase(),
    }
}

pub async fn import(st: &AppState, link: PlannerLink) -> ApiResult<Asset> {
    let article = format!("VPC-{}", link.code);
    if let Some(a) = crate::assets::find_by_article(&st.db, &article).await? {
        return Ok(a);
    }
    let key = DEXF_KEYS.iter().find(|(r, _)| *r == link.range).or(DEXF_KEYS.first()).map(|(_, k)| *k).unwrap();
    let api = format!("https://api.dexf.ikea.com/vpc/v1/configurations/retailunit/{}/locale/{}/{}", link.retail_unit, link.locale, link.code);
    let resp = st
        .ikea_http
        .get(&api)
        .header("dexf-api-key", key)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| ApiError::unprocessable(format!("IKEA-Planer nicht erreichbar ({}).", e.without_url())))?;
    let status = resp.status();
    if status.as_u16() == 404 || status.as_u16() == 400 {
        return Err(ApiError::unprocessable("Planer-Code nicht gefunden – ist der Link vollständig (…#/vpc/CODE)?"));
    }
    if !status.is_success() {
        return Err(ApiError::unprocessable(format!("IKEA-Planer antwortet mit HTTP {} – Schnittstelle evtl. geändert.", status.as_u16())));
    }
    let cfg: Value = resp.json().await.map_err(|e| ApiError::unprocessable(format!("Planer-Daten nicht lesbar: {e}")))?;
    let arts = cfg.pointer("/icf/content/articles").and_then(|a| a.as_array()).ok_or_else(|| ApiError::unprocessable("Planer-Design ohne Teileliste (ICF)"))?;
    let solids = solids(arts);
    if solids.is_empty() {
        return Err(ApiError::unprocessable("Im Planer-Design keine darstellbaren Teile gefunden"));
    }
    let glb = write_glb(&solids);
    let mut warnings = vec!["Aus dem IKEA-Planer nachgebaut (vereinfachte Geometrie, Farben aus dem Planer).".to_string()];
    let parts = arts.len();
    if parts > 0 {
        warnings.push(format!("{} Teile, {} Artikel", parts, cfg.pointer("/itemList/item").and_then(|i| i.as_array()).map_or(0, |a| a.len())));
    }
    // thumbnail: IKEA's rendering of the design
    let mut thumb = None;
    let mut image_url = None;
    if let Some(img) = cfg.get("image").and_then(|i| i.as_array()).and_then(|a| a.iter().find(|x| x["typeCode"] == "main-design-image")).and_then(|x| x["url"].as_str()) {
        image_url = Some(img.to_string());
        if let Ok(r) = st.ikea_http.get(img).send().await {
            if r.status().is_success() {
                if let Ok(b) = r.bytes().await {
                    let k = format!("ikea-planner/{}.jpg", link.code);
                    if st.store.put(&ObjPath::from(k.as_str()), PutPayload::from(b)).await.is_ok() {
                        thumb = Some(format!("/api/files/{k}"));
                    }
                }
            }
        }
    }
    let bbox = crate::glb::bbox(&glb).map_err(|e| ApiError::internal(format!("GLB: {e:#}")))?;
    let id = Uuid::new_v4();
    crate::assets::put_model(st, id, glb).await?;
    let name = format!(
        "{} Kombination {}",
        cfg.get("applicationName").and_then(|n| n.as_str()).map(str::to_string).unwrap_or_else(|| range_label(&link.range)),
        link.code
    );
    let a = Asset {
        id,
        kind: AssetKind::Ikea,
        name,
        dims_mm: bbox.wdh_mm(),
        model_url: Some(format!("/api/assets/{id}/model.glb")),
        thumbnail_url: thumb,
        primitive: None,
        ikea: Some(IkeaMeta { url: link.url, article_no: article, price: None, currency: None, image_url }),
        light: None,
        warnings,
        created_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    };
    crate::assets::insert_ikea(st, a).await
}

// ------------------------------------------------------------------ geometry

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mat {
    pub color: [f32; 3],
    pub metallic: f32,
    pub roughness: f32,
}

/// Oriented box: centre (mm, y up), half extents (mm), rotation (deg, applied z, then x, then y).
#[derive(Clone, Debug)]
pub struct Solid {
    pub c: [f64; 3],
    pub half: [f64; 3],
    pub rot: [f64; 3],
    pub mat: Mat,
}

fn hex(c: &str) -> Option<[f32; 3]> {
    let c = c.trim().strip_prefix('#')?;
    let v = u32::from_str_radix(c.get(..6)?, 16).ok()?;
    let f = |x: u32| ((x & 255) as f32 / 255.0).powf(2.2); // sRGB → linear (glTF factors are linear)
    Some([f(v >> 16), f(v >> 8), f(v)])
}

fn v3(v: &Value) -> [f64; 3] {
    [v["x"].as_f64().unwrap_or(0.0), v["y"].as_f64().unwrap_or(0.0), v["z"].as_f64().unwrap_or(0.0)]
}

const SKIP: &[&str] = &["hinge", "light", "led", "cord", "driver", "mat", "rail-for", "power", "insert", "organizer", "div-"];

fn material_for(cat: &str, colors: &[String]) -> Mat {
    let c0 = colors.first().and_then(|c| hex(c));
    if cat.contains("door") && colors.len() > 1 && colors[0].eq_ignore_ascii_case("#fafafa") {
        return Mat { color: [0.78, 0.82, 0.84], metallic: 1.0, roughness: 0.04 }; // mirror
    }
    if cat.contains("handle") || cat.contains("knob") || cat.contains("rail") || cat.contains("basket") {
        return Mat { color: c0.unwrap_or([0.3, 0.3, 0.3]), metallic: 0.7, roughness: 0.35 };
    }
    if cat.contains("glass") {
        return Mat { color: [0.8, 0.88, 0.9], metallic: 0.0, roughness: 0.05 };
    }
    Mat { color: c0.unwrap_or(hex("#ece8e1").unwrap()), metallic: 0.0, roughness: 0.6 }
}

/// Articles → solids in IKEA world coordinates (mm).
pub fn solids(arts: &[Value]) -> Vec<Solid> {
    let by_id: BTreeMap<i64, &Value> = arts.iter().filter_map(|a| Some((a["id"].as_i64()?, a))).collect();
    let roots: Vec<&Value> = arts.iter().filter(|a| a["parent_ids"].as_array().is_none_or(|p| p.is_empty())).collect();
    // reference centre: median of root centres (some articles carry positions in another frame)
    let mut xs: Vec<f64> = roots.iter().map(|a| v3(&a["transform"]["position"])[0]).collect();
    let mut zs: Vec<f64> = roots.iter().map(|a| v3(&a["transform"]["position"])[2]).collect();
    xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    zs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mid = [xs.get(xs.len() / 2).copied().unwrap_or(0.0), zs.get(zs.len() / 2).copied().unwrap_or(0.0)];
    let mut out = Vec::new();
    for a in arts {
        let cat = a["category"].as_str().unwrap_or("").to_lowercase();
        let is_door = cat.contains("door");
        let is_carcass = cat.ends_with("frame") || cat.contains("addon");
        if !is_door && !is_carcass && SKIP.iter().any(|s| cat.contains(s)) {
            continue;
        }
        let dims = v3(&a["dimensions"]);
        if dims.iter().any(|d| *d < 1.0) {
            continue;
        }
        let rot = v3(&a["transform"]["rotation"]);
        let mut c = v3(&a["transform"]["position"]);
        let colors: Vec<String> = a["colors"].as_array().map(|c| c.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default();
        // outlier position: rebuild from the children's centres (e.g. corner add-ons)
        if (c[0] - mid[0]).hypot(c[2] - mid[1]) > 3000.0 {
            let kids: Vec<[f64; 3]> = a["child_ids"]
                .as_array()
                .map(|k| k.iter().filter_map(|i| by_id.get(&i.as_i64()?)).map(|k| v3(&k["transform"]["position"])).collect())
                .unwrap_or_default();
            let kids: Vec<[f64; 3]> = kids.into_iter().filter(|k| (k[0] - mid[0]).hypot(k[2] - mid[1]) < 3000.0).collect();
            if kids.is_empty() {
                continue;
            }
            let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
            for k in &kids {
                lo = [lo[0].min(k[0]), lo[1].min(k[2])];
                hi = [hi[0].max(k[0]), hi[1].max(k[2])];
            }
            c = [(lo[0] + hi[0]) / 2.0, dims[1] / 2.0, (lo[1] + hi[1]) / 2.0];
        }
        let mat = material_for(&cat, &colors);
        if is_carcass {
            // carcass as panels (local: x width, y height, z depth, front = +z)
            let (w, h, d) = (dims[0] / 2.0, dims[1] / 2.0, dims[2] / 2.0);
            let t = 9.0;
            let frame_mat = Mat { color: colors.first().and_then(|c| hex(c)).unwrap_or(hex("#ece8e1").unwrap()), ..mat };
            for (lc, lh) in [
                ([-w + t, 0.0, 0.0], [t, h, d]),
                ([w - t, 0.0, 0.0], [t, h, d]),
                ([0.0, h - t, 0.0], [w - 2.0 * t, t, d]),
                ([0.0, -h + t + 40.0, 0.0], [w - 2.0 * t, t, d]),
                ([0.0, -h + 20.0, d - 10.0], [w - 2.0 * t, 20.0, 10.0]), // plinth
                ([0.0, 0.0, -d + 3.0], [w - 2.0 * t, h, 3.0]),
            ] {
                let p = rotate(lc, rot);
                out.push(Solid { c: [c[0] + p[0], c[1] + p[1], c[2] + p[2]], half: lh, rot, mat: frame_mat });
            }
            continue;
        }
        out.push(Solid { c, half: [dims[0] / 2.0, dims[1] / 2.0, dims[2] / 2.0], rot, mat });
    }
    out
}

/// Rotation z, then x, then y (degrees).
fn rotate(p: [f64; 3], r: [f64; 3]) -> [f64; 3] {
    let (sx, cx) = r[0].to_radians().sin_cos();
    let (sy, cy) = r[1].to_radians().sin_cos();
    let (sz, cz) = r[2].to_radians().sin_cos();
    let p = [p[0] * cz - p[1] * sz, p[0] * sz + p[1] * cz, p[2]];
    let p = [p[0], p[1] * cx - p[2] * sx, p[1] * sx + p[2] * cx];
    [p[0] * cy + p[2] * sy, p[1], -p[0] * sy + p[2] * cy]
}

/// Solids (mm) → GLB (metres), recentred: footprint centre at the origin, bottom at y = 0.
pub fn write_glb(solids: &[Solid]) -> Vec<u8> {
    // corners of all boxes for the bounding box
    let mut lo = [f64::MAX; 3];
    let mut hi = [f64::MIN; 3];
    let corners = |s: &Solid| {
        let mut v = Vec::with_capacity(8);
        for i in 0..8 {
            let l = [
                if i & 1 == 0 { -s.half[0] } else { s.half[0] },
                if i & 2 == 0 { -s.half[1] } else { s.half[1] },
                if i & 4 == 0 { -s.half[2] } else { s.half[2] },
            ];
            let r = rotate(l, s.rot);
            v.push([s.c[0] + r[0], s.c[1] + r[1], s.c[2] + r[2]]);
        }
        v
    };
    for s in solids {
        for p in corners(s) {
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
    }
    let off = [(lo[0] + hi[0]) / 2.0, lo[1], (lo[2] + hi[2]) / 2.0];
    // group by material
    let mut groups: Vec<(Mat, Vec<f32>, Vec<f32>, Vec<u32>)> = Vec::new();
    const FACES: [([f64; 3], [usize; 4]); 6] = [
        ([1.0, 0.0, 0.0], [1, 3, 7, 5]),
        ([-1.0, 0.0, 0.0], [0, 4, 6, 2]),
        ([0.0, 1.0, 0.0], [2, 6, 7, 3]),
        ([0.0, -1.0, 0.0], [0, 1, 5, 4]),
        ([0.0, 0.0, 1.0], [4, 5, 7, 6]),
        ([0.0, 0.0, -1.0], [0, 2, 3, 1]),
    ];
    for s in solids {
        let gi = match groups.iter().position(|g| g.0 == s.mat) {
            Some(i) => i,
            None => {
                groups.push((s.mat, vec![], vec![], vec![]));
                groups.len() - 1
            }
        };
        let cs = corners(s);
        let g = &mut groups[gi];
        for (n, q) in FACES {
            let nr = rotate(n, s.rot);
            let base = (g.1.len() / 3) as u32;
            for &qi in &q {
                let p = cs[qi];
                g.1.extend([((p[0] - off[0]) / 1000.0) as f32, ((p[1] - off[1]) / 1000.0) as f32, ((p[2] - off[2]) / 1000.0) as f32]);
                g.2.extend([nr[0] as f32, nr[1] as f32, nr[2] as f32]);
            }
            g.3.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        }
    }
    // binary buffer: per group positions, normals, indices
    let mut bin: Vec<u8> = Vec::new();
    let mut views = Vec::new();
    let mut accessors = Vec::new();
    let mut prims = Vec::new();
    let mut materials = Vec::new();
    for (mi, (mat, pos, nrm, idx)) in groups.iter().enumerate() {
        let (mut mn, mut mx) = ([f32::MAX; 3], [f32::MIN; 3]);
        for p in pos.chunks(3) {
            for k in 0..3 {
                mn[k] = mn[k].min(p[k]);
                mx[k] = mx[k].max(p[k]);
            }
        }
        let mut push = |bytes: Vec<u8>, target: u32| {
            while bin.len() % 4 != 0 {
                bin.push(0);
            }
            let off = bin.len();
            bin.extend_from_slice(&bytes);
            views.push(json!({"buffer": 0, "byteOffset": off, "byteLength": bytes.len(), "target": target}));
            views.len() - 1
        };
        let pv = push(pos.iter().flat_map(|f| f.to_le_bytes()).collect(), 34962);
        let nv = push(nrm.iter().flat_map(|f| f.to_le_bytes()).collect(), 34962);
        let iv = push(idx.iter().flat_map(|f| f.to_le_bytes()).collect(), 34963);
        let n = pos.len() / 3;
        accessors.push(json!({"bufferView": pv, "componentType": 5126, "count": n, "type": "VEC3", "min": mn, "max": mx}));
        accessors.push(json!({"bufferView": nv, "componentType": 5126, "count": n, "type": "VEC3"}));
        accessors.push(json!({"bufferView": iv, "componentType": 5125, "count": idx.len(), "type": "SCALAR"}));
        let a0 = accessors.len() - 3;
        prims.push(json!({"attributes": {"POSITION": a0, "NORMAL": a0 + 1}, "indices": a0 + 2, "material": mi}));
        materials.push(json!({"pbrMetallicRoughness": {"baseColorFactor": [mat.color[0], mat.color[1], mat.color[2], 1.0], "metallicFactor": mat.metallic, "roughnessFactor": mat.roughness}}));
    }
    while bin.len() % 4 != 0 {
        bin.push(0);
    }
    let gltf = json!({
        "asset": {"version": "2.0", "generator": "innenarchitekt ikea-planner"},
        "scene": 0, "scenes": [{"nodes": [0]}], "nodes": [{"mesh": 0, "name": "planner"}],
        "meshes": [{"primitives": prims}], "materials": materials,
        "buffers": [{"byteLength": bin.len()}], "bufferViews": views, "accessors": accessors,
    });
    let mut j = serde_json::to_vec(&gltf).unwrap();
    while j.len() % 4 != 0 {
        j.push(b' ');
    }
    let total = 12 + 8 + j.len() + 8 + bin.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(j.len() as u32).to_le_bytes());
    out.extend_from_slice(&0x4E4F534Au32.to_le_bytes());
    out.extend_from_slice(&j);
    out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    out.extend_from_slice(&0x004E4942u32.to_le_bytes());
    out.extend_from_slice(&bin);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_planner_links() {
        let l = parse_link("https://www.ikea.com/addon-app/storageone/pax/web/latest/ch/de/?vpcSource=clipboard#/vpc/33ZNPWP").unwrap();
        assert_eq!((l.range.as_str(), l.retail_unit.as_str(), l.locale.as_str(), l.code.as_str()), ("pax", "CH", "de-CH", "33ZNPWP"));
        let l = parse_link("https://www.ikea.com/addon-app/storageone/besta/web/latest/de/de/#/u/ab12cd").unwrap();
        assert_eq!((l.range.as_str(), l.code.as_str(), l.locale.as_str()), ("besta", "AB12CD", "de-DE"));
        assert!(parse_link("https://www.ikea.com/ch/de/p/billy-buecherregal-weiss-00263850/").is_none());
    }

    #[test]
    fn rotation_order_keeps_handles_vertical() {
        // handle 300 long along local x, door turned 90° about y, handle 270° about z → stays vertical
        let p = rotate([150.0, 0.0, 0.0], [0.0, 90.0, 270.0]);
        assert!(p[1].abs() > 149.0 && p[0].abs() < 1.0 && p[2].abs() < 1.0, "{p:?}");
    }

    #[test]
    fn builds_glb_from_articles() {
        let arts: Vec<Value> = serde_json::from_value(json!([
            {"id": 1, "parent_ids": [], "child_ids": [2], "category": "pax-hinge-frame", "colors": [], "dimensions": {"x": 1000, "y": 2360, "z": 580},
             "transform": {"position": {"x": 0, "y": 1180, "z": 0}, "rotation": {"x": 0, "y": 0, "z": 0}}},
            {"id": 2, "parent_ids": [1], "child_ids": [], "category": "pax-hinge-door-softclosing", "colors": ["#D6BDA1"], "dimensions": {"x": 495, "y": 2294, "z": 16},
             "transform": {"position": {"x": -250, "y": 1213, "z": 306}, "rotation": {"x": 0, "y": 0, "z": 0}}},
            {"id": 3, "parent_ids": [1], "child_ids": [], "category": "pax-soft-closing-hinge", "colors": [], "dimensions": {"x": 10, "y": 10, "z": 10},
             "transform": {"position": {"x": 0, "y": 0, "z": 0}, "rotation": {"x": 0, "y": 0, "z": 0}}}
        ]))
        .unwrap();
        let s = solids(&arts);
        assert_eq!(s.len(), 6 + 1, "frame panels + door, hinge skipped");
        assert!(solids(&serde_json::from_value::<Vec<Value>>(json!([{"id": 9, "parent_ids": [], "child_ids": [], "category": "frame-organizer", "colors": [],
            "dimensions": {"x": 390, "y": 190, "z": 480}, "transform": {"position": {"x": 0, "y": 0, "z": 0}, "rotation": {"x": 0, "y": 0, "z": 0}}}])).unwrap()).is_empty());
        let glb = write_glb(&s);
        if let Ok(p) = std::env::var("IA_PLANNER_JSON") {
            // manual check with a real design: IA_PLANNER_JSON=vpc.json cargo test builds_glb -- --nocapture
            let cfg: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
            let real = solids(cfg.pointer("/icf/content/articles").unwrap().as_array().unwrap());
            let g = write_glb(&real);
            std::fs::write(format!("{p}.glb"), &g).unwrap();
            println!("{} solids, bbox {:?}", real.len(), crate::glb::bbox(&g).unwrap().wdh_mm());
        }
        let bb = crate::glb::bbox(&glb).unwrap();
        let wdh = bb.wdh_mm();
        assert_eq!(wdh[2], 2360, "{wdh:?}");
        assert!((wdh[0] as i32 - 1000).abs() <= 1 && wdh[1] >= 580, "{wdh:?}");
    }
}
