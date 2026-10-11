//! Minimal GLB inspection: validate the container and compute the model's bounding box from the
//! JSON chunk only (accessor `min`/`max` of POSITION, transformed by node world matrices incl. TRS).
//! Works for Draco/meshopt-compressed files too, since POSITION min/max are mandatory in glTF 2.0.

use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;

const MAGIC: &[u8; 4] = b"glTF";
const CHUNK_JSON: u32 = 0x4E4F_534A;

/// Axis-aligned bounding box in model units (glTF: metres, Y up).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BBox {
    pub min: [f64; 3],
    pub max: [f64; 3],
}

impl BBox {
    pub fn size(&self) -> [f64; 3] {
        [self.max[0] - self.min[0], self.max[1] - self.min[1], self.max[2] - self.min[2]]
    }
    /// Size in millimetres as (width = X, depth = Z, height = Y).
    pub fn wdh_mm(&self) -> [u32; 3] {
        let s = self.size();
        let mm = |v: f64| (v * 1000.0).round().max(0.0) as u32;
        [mm(s[0]), mm(s[2]), mm(s[1])]
    }
    fn extend(&mut self, p: [f64; 3]) {
        for i in 0..3 {
            self.min[i] = self.min[i].min(p[i]);
            self.max[i] = self.max[i].max(p[i]);
        }
    }
    fn empty() -> Self {
        BBox { min: [f64::INFINITY; 3], max: [f64::NEG_INFINITY; 3] }
    }
    fn is_valid(&self) -> bool {
        (0..3).all(|i| self.min[i].is_finite() && self.max[i].is_finite() && self.max[i] >= self.min[i])
    }
}

pub fn is_glb(bytes: &[u8]) -> bool {
    bytes.len() >= 12 && &bytes[0..4] == MAGIC
}

/// Extract and parse the JSON chunk of a GLB.
pub fn json_chunk(bytes: &[u8]) -> Result<Value> {
    if !is_glb(bytes) {
        bail!("keine GLB-Datei (Magic «glTF» fehlt)");
    }
    let u32_at = |o: usize| -> Result<u32> {
        bytes
            .get(o..o + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .ok_or_else(|| anyhow!("GLB abgeschnitten"))
    };
    let version = u32_at(4)?;
    if version != 2 {
        bail!("GLB-Version {version} nicht unterstützt (nur glTF 2.0)");
    }
    let len = u32_at(12)? as usize;
    let ty = u32_at(16)?;
    if ty != CHUNK_JSON {
        bail!("erster GLB-Chunk ist kein JSON");
    }
    let data = bytes.get(20..20 + len).ok_or_else(|| anyhow!("GLB-JSON-Chunk abgeschnitten"))?;
    serde_json::from_slice(data).context("GLB-JSON ungültig")
}

pub type Mat4 = [f64; 16]; // column-major, like glTF

pub const IDENTITY: Mat4 = [1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.];

pub fn mul(a: &Mat4, b: &Mat4) -> Mat4 {
    let mut r = [0.0; 16];
    for c in 0..4 {
        for row in 0..4 {
            r[c * 4 + row] = (0..4).map(|k| a[k * 4 + row] * b[c * 4 + k]).sum();
        }
    }
    r
}

fn transform(m: &Mat4, p: [f64; 3]) -> [f64; 3] {
    let w = m[3] * p[0] + m[7] * p[1] + m[11] * p[2] + m[15];
    let w = if w.abs() < 1e-12 { 1.0 } else { w };
    [
        (m[0] * p[0] + m[4] * p[1] + m[8] * p[2] + m[12]) / w,
        (m[1] * p[0] + m[5] * p[1] + m[9] * p[2] + m[13]) / w,
        (m[2] * p[0] + m[6] * p[1] + m[10] * p[2] + m[14]) / w,
    ]
}

fn nums<const N: usize>(v: Option<&Value>) -> Option<[f64; N]> {
    let arr = v?.as_array()?;
    if arr.len() != N {
        return None;
    }
    let mut out = [0.0; N];
    for (i, x) in arr.iter().enumerate() {
        out[i] = x.as_f64()?;
    }
    Some(out)
}

fn local_matrix(node: &Value) -> Mat4 {
    if let Some(m) = nums::<16>(node.get("matrix")) {
        return m;
    }
    let t = nums::<3>(node.get("translation")).unwrap_or([0.0; 3]);
    let [x, y, z, w] = nums::<4>(node.get("rotation")).unwrap_or([0.0, 0.0, 0.0, 1.0]);
    let s = nums::<3>(node.get("scale")).unwrap_or([1.0; 3]);
    // R from unit quaternion (column-major)
    let r = [
        1.0 - 2.0 * (y * y + z * z),
        2.0 * (x * y + z * w),
        2.0 * (x * z - y * w),
        0.0,
        2.0 * (x * y - z * w),
        1.0 - 2.0 * (x * x + z * z),
        2.0 * (y * z + x * w),
        0.0,
        2.0 * (x * z + y * w),
        2.0 * (y * z - x * w),
        1.0 - 2.0 * (x * x + y * y),
        0.0,
        0.0,
        0.0,
        0.0,
        1.0,
    ];
    let sm = [s[0], 0., 0., 0., 0., s[1], 0., 0., 0., 0., s[2], 0., 0., 0., 0., 1.];
    let tm = [1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., t[0], t[1], t[2], 1.];
    mul(&tm, &mul(&r, &sm))
}

/// Normalisation divisor for `normalized: true` integer accessors (KHR_mesh_quantization).
fn norm_div(component_type: u64) -> f64 {
    match component_type {
        5120 => 127.0,
        5121 => 255.0,
        5122 => 32767.0,
        5123 => 65535.0,
        _ => 1.0,
    }
}

fn accessor_bounds(json: &Value, idx: usize) -> Option<([f64; 3], [f64; 3])> {
    let acc = json.get("accessors")?.get(idx)?;
    let mut min = nums::<3>(acc.get("min"))?;
    let mut max = nums::<3>(acc.get("max"))?;
    if acc.get("normalized").and_then(Value::as_bool).unwrap_or(false) {
        let d = norm_div(acc.get("componentType").and_then(Value::as_u64).unwrap_or(0));
        let signed = matches!(acc.get("componentType").and_then(Value::as_u64), Some(5120 | 5122));
        for v in min.iter_mut().chain(max.iter_mut()) {
            *v /= d;
            if signed {
                *v = v.max(-1.0);
            }
        }
    }
    Some((min, max))
}

fn visit(json: &Value, node_idx: usize, parent: &Mat4, bbox: &mut BBox, depth: usize) {
    if depth > 64 {
        return;
    }
    let Some(node) = json.get("nodes").and_then(|n| n.get(node_idx)) else { return };
    let world = mul(parent, &local_matrix(node));
    if let Some(mesh_idx) = node.get("mesh").and_then(Value::as_u64) {
        let prims = json
            .get("meshes")
            .and_then(|m| m.get(mesh_idx as usize))
            .and_then(|m| m.get("primitives"))
            .and_then(Value::as_array);
        for prim in prims.into_iter().flatten() {
            let Some(pos) = prim.get("attributes").and_then(|a| a.get("POSITION")).and_then(Value::as_u64) else {
                continue;
            };
            if let Some((mn, mx)) = accessor_bounds(json, pos as usize) {
                for i in 0..8 {
                    let c = [
                        if i & 1 == 0 { mn[0] } else { mx[0] },
                        if i & 2 == 0 { mn[1] } else { mx[1] },
                        if i & 4 == 0 { mn[2] } else { mx[2] },
                    ];
                    bbox.extend(transform(&world, c));
                }
            }
        }
    }
    if let Some(children) = node.get("children").and_then(Value::as_array) {
        for c in children.iter().filter_map(Value::as_u64) {
            visit(json, c as usize, &world, bbox, depth + 1);
        }
    }
}

/// Bounding box of the default scene (or all root nodes if no scene is declared).
pub fn bbox_from_json(json: &Value) -> Result<BBox> {
    let node_count = json.get("nodes").and_then(Value::as_array).map(|a| a.len()).unwrap_or(0);
    let scene_idx = json.get("scene").and_then(Value::as_u64).unwrap_or(0) as usize;
    let roots: Vec<usize> = match json.get("scenes").and_then(|s| s.get(scene_idx)).and_then(|s| s.get("nodes")).and_then(Value::as_array) {
        Some(n) => n.iter().filter_map(Value::as_u64).map(|x| x as usize).collect(),
        None => {
            let mut is_child = vec![false; node_count];
            for n in json.get("nodes").and_then(Value::as_array).into_iter().flatten() {
                for c in n.get("children").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_u64) {
                    if let Some(slot) = is_child.get_mut(c as usize) {
                        *slot = true;
                    }
                }
            }
            (0..node_count).filter(|i| !is_child[*i]).collect()
        }
    };
    let mut bbox = BBox::empty();
    for r in roots {
        visit(json, r, &IDENTITY, &mut bbox, 0);
    }
    if !bbox.is_valid() {
        bail!("GLB enthält keine Geometrie mit POSITION-Grenzen");
    }
    Ok(bbox)
}

pub fn bbox(bytes: &[u8]) -> Result<BBox> {
    bbox_from_json(&json_chunk(bytes)?)
}

/// Binary chunk of a GLB (empty if absent).
pub fn bin_chunk(bytes: &[u8]) -> Result<&[u8]> {
    let u32_at = |o: usize| bytes.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize);
    let jlen = u32_at(12).ok_or_else(|| anyhow!("GLB abgeschnitten"))?;
    let o = 20 + jlen;
    let Some(blen) = u32_at(o) else { return Ok(&[]) };
    bytes.get(o + 8..o + 8 + blen).ok_or_else(|| anyhow!("GLB-BIN-Chunk abgeschnitten"))
}

fn shift(v: &mut Value, key: &str, by: usize) {
    if let Some(i) = v.get(key).and_then(Value::as_u64) {
        v[key] = Value::from(i as usize + by);
    }
}

/// Texture references in a material: every object under a key ending in «Texture» carries an `index`.
fn shift_textures(v: &mut Value, by: usize) {
    match v {
        Value::Object(m) => {
            for (k, x) in m.iter_mut() {
                if k.ends_with("Texture") && x.is_object() {
                    shift(x, "index", by);
                }
                shift_textures(x, by);
            }
        }
        Value::Array(a) => a.iter_mut().for_each(|x| shift_textures(x, by)),
        _ => {}
    }
}

/// Merge GLBs into one: each source is placed once per matrix (metres, column-major). Meshes, materials and
/// textures are shared between instances; compressed data (Draco, WebP …) is copied as is. The result is
/// recentred like the other furniture models: footprint centre at the origin, bottom at y = 0.
pub fn merge(sources: &[(&[u8], Vec<Mat4>)]) -> Result<Vec<u8>> {
    let mut out: serde_json::Map<String, Value> = serde_json::Map::new();
    let mut lists: std::collections::BTreeMap<&str, Vec<Value>> = Default::default();
    let mut used: std::collections::BTreeSet<String> = Default::default();
    let mut required: std::collections::BTreeSet<String> = Default::default();
    let mut bin: Vec<u8> = Vec::new();
    let mut instances: Vec<usize> = Vec::new();
    const KEYS: [&str; 8] = ["bufferViews", "accessors", "images", "samplers", "textures", "materials", "meshes", "nodes"];
    for (bytes, mats) in sources {
        if mats.is_empty() {
            continue;
        }
        let j = json_chunk(bytes)?;
        let src_bin = bin_chunk(bytes)?;
        let base: std::collections::BTreeMap<&str, usize> = KEYS.iter().map(|k| (*k, lists.get(k).map_or(0, |l| l.len()))).collect();
        while bin.len() % 8 != 0 {
            bin.push(0);
        }
        let bin_base = bin.len();
        bin.extend_from_slice(src_bin);
        for e in j.get("extensionsUsed").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
            used.insert(e.into());
        }
        for e in j.get("extensionsRequired").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
            required.insert(e.into());
        }
        let arr = |k: &str| j.get(k).and_then(Value::as_array).cloned().unwrap_or_default();
        for mut v in arr("bufferViews") {
            v["buffer"] = Value::from(0);
            let off = v.get("byteOffset").and_then(Value::as_u64).unwrap_or(0) as usize;
            v["byteOffset"] = Value::from(off + bin_base);
            lists.entry("bufferViews").or_default().push(v);
        }
        for mut v in arr("accessors") {
            shift(&mut v, "bufferView", base["bufferViews"]);
            if let Some(sp) = v.get_mut("sparse") {
                for k in ["indices", "values"] {
                    if let Some(x) = sp.get_mut(k) {
                        shift(x, "bufferView", base["bufferViews"]);
                    }
                }
            }
            lists.entry("accessors").or_default().push(v);
        }
        for mut v in arr("images") {
            if v.get("uri").is_some() {
                bail!("GLB mit externen Bildern wird nicht unterstützt");
            }
            shift(&mut v, "bufferView", base["bufferViews"]);
            lists.entry("images").or_default().push(v);
        }
        lists.entry("samplers").or_default().extend(arr("samplers"));
        for mut v in arr("textures") {
            shift(&mut v, "sampler", base["samplers"]);
            shift(&mut v, "source", base["images"]);
            if let Some(Value::Object(ext)) = v.get_mut("extensions") {
                for x in ext.values_mut() {
                    shift(x, "source", base["images"]);
                }
            }
            lists.entry("textures").or_default().push(v);
        }
        for mut v in arr("materials") {
            shift_textures(&mut v, base["textures"]);
            lists.entry("materials").or_default().push(v);
        }
        for mut m in arr("meshes") {
            for p in m.get_mut("primitives").and_then(Value::as_array_mut).into_iter().flatten() {
                if let Some(Value::Object(at)) = p.get_mut("attributes") {
                    for x in at.values_mut() {
                        *x = Value::from(x.as_u64().unwrap_or(0) as usize + base["accessors"]);
                    }
                }
                for t in p.get_mut("targets").and_then(Value::as_array_mut).into_iter().flatten() {
                    if let Value::Object(at) = t {
                        for x in at.values_mut() {
                            *x = Value::from(x.as_u64().unwrap_or(0) as usize + base["accessors"]);
                        }
                    }
                }
                shift(p, "indices", base["accessors"]);
                shift(p, "material", base["materials"]);
                if let Some(d) = p.get_mut("extensions").and_then(|e| e.get_mut("KHR_draco_mesh_compression")) {
                    shift(d, "bufferView", base["bufferViews"]);
                }
            }
            lists.entry("meshes").or_default().push(m);
        }
        // node tree once per instance (a glTF node has a single parent); meshes are shared
        let nodes = arr("nodes");
        let roots: Vec<usize> = j
            .get("scenes")
            .and_then(|s| s.get(j.get("scene").and_then(Value::as_u64).unwrap_or(0) as usize))
            .and_then(|s| s.get("nodes"))
            .and_then(Value::as_array)
            .map(|n| n.iter().filter_map(Value::as_u64).map(|x| x as usize).collect())
            .unwrap_or_else(|| (0..nodes.len()).collect());
        for m in mats {
            let nb = lists.get("nodes").map_or(0, |l| l.len());
            for n in &nodes {
                let mut n = n.clone();
                if let Some(o) = n.as_object_mut() {
                    o.remove("skin");
                    o.remove("camera");
                }
                shift(&mut n, "mesh", base["meshes"]);
                if let Some(c) = n.get_mut("children").and_then(Value::as_array_mut) {
                    for x in c.iter_mut() {
                        *x = Value::from(x.as_u64().unwrap_or(0) as usize + nb);
                    }
                }
                lists.entry("nodes").or_default().push(n);
            }
            let wrapper = serde_json::json!({ "matrix": m.to_vec(), "children": roots.iter().map(|r| r + nb).collect::<Vec<_>>() });
            lists.entry("nodes").or_default().push(wrapper);
            instances.push(lists["nodes"].len() - 1);
        }
    }
    if instances.is_empty() {
        bail!("nichts zum Zusammenführen");
    }
    while bin.len() % 4 != 0 {
        bin.push(0);
    }
    let root = lists.get("nodes").map_or(0, |l| l.len());
    lists.entry("nodes").or_default().push(serde_json::json!({ "name": "planner", "children": instances }));
    out.insert("asset".into(), serde_json::json!({ "version": "2.0", "generator": "innenarchitekt merge" }));
    out.insert("scene".into(), Value::from(0));
    out.insert("scenes".into(), serde_json::json!([{ "nodes": [root] }]));
    out.insert("buffers".into(), serde_json::json!([{ "byteLength": bin.len() }]));
    for (k, v) in lists {
        if !v.is_empty() {
            out.insert(k.into(), Value::Array(v));
        }
    }
    if !used.is_empty() {
        out.insert("extensionsUsed".into(), serde_json::json!(used));
    }
    if !required.is_empty() {
        out.insert("extensionsRequired".into(), serde_json::json!(required));
    }
    let mut gltf = Value::Object(out);
    let bb = bbox_from_json(&gltf)?;
    gltf["nodes"][root]["translation"] = serde_json::json!([-(bb.min[0] + bb.max[0]) / 2.0, -bb.min[1], -(bb.min[2] + bb.max[2]) / 2.0]);
    let mut j = serde_json::to_vec(&gltf)?;
    while j.len() % 4 != 0 {
        j.push(b' ');
    }
    let total = 12 + 8 + j.len() + 8 + bin.len();
    let mut o = Vec::with_capacity(total);
    o.extend_from_slice(MAGIC);
    o.extend_from_slice(&2u32.to_le_bytes());
    o.extend_from_slice(&(total as u32).to_le_bytes());
    o.extend_from_slice(&(j.len() as u32).to_le_bytes());
    o.extend_from_slice(&CHUNK_JSON.to_le_bytes());
    o.extend_from_slice(&j);
    o.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    o.extend_from_slice(&0x004E_4942u32.to_le_bytes());
    o.extend_from_slice(&bin);
    Ok(o)
}

#[cfg(test)]
pub(crate) fn build_glb(json: &Value) -> Vec<u8> {
    let mut j = serde_json::to_vec(json).unwrap();
    while j.len() % 4 != 0 {
        j.push(b' ');
    }
    let total = 12 + 8 + j.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(j.len() as u32).to_le_bytes());
    out.extend_from_slice(&CHUNK_JSON.to_le_bytes());
    out.extend_from_slice(&j);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn approx(a: [f64; 3], b: [f64; 3]) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() < 1e-6)
    }

    #[test]
    fn rejects_non_glb() {
        assert!(bbox(b"not a glb at all").is_err());
        assert!(!is_glb(b"glT"));
    }

    #[test]
    fn simple_mesh() {
        let glb = build_glb(&json!({
            "asset": {"version": "2.0"},
            "scene": 0,
            "scenes": [{"nodes": [0]}],
            "nodes": [{"mesh": 0}],
            "meshes": [{"primitives": [{"attributes": {"POSITION": 0}}]}],
            "accessors": [{"componentType": 5126, "count": 3, "type": "VEC3", "min": [-0.4, 0.0, -0.14], "max": [0.4, 2.02, 0.14]}]
        }));
        let b = bbox(&glb).unwrap();
        assert!(approx(b.min, [-0.4, 0.0, -0.14]));
        assert_eq!(b.wdh_mm(), [800, 280, 2020]);
    }

    #[test]
    fn trs_hierarchy_and_draco_extension() {
        // Parent: translate (1,0,0), rotate 90° about Y. Child: scale 2 on X.
        // Unit cube [0,1]^3 → child scale → [0,2]x[0,1]x[0,1] → rotY(90°): (x,y,z) → (z,y,-x)
        // → x∈[0,1], z∈[-2,0] → translate → x∈[1,2].
        let s = std::f64::consts::FRAC_1_SQRT_2;
        let glb = build_glb(&json!({
            "asset": {"version": "2.0"},
            "extensionsUsed": ["KHR_draco_mesh_compression"],
            "scenes": [{"nodes": [0]}],
            "nodes": [
                {"translation": [1.0, 0.0, 0.0], "rotation": [0.0, s, 0.0, s], "children": [1]},
                {"scale": [2.0, 1.0, 1.0], "mesh": 0}
            ],
            "meshes": [{"primitives": [{"attributes": {"POSITION": 0},
                "extensions": {"KHR_draco_mesh_compression": {"bufferView": 0, "attributes": {"POSITION": 0}}}}]}],
            "accessors": [{"componentType": 5126, "count": 8, "type": "VEC3", "min": [0, 0, 0], "max": [1, 1, 1]}]
        }));
        let b = bbox(&glb).unwrap();
        assert!(approx(b.min, [1.0, 0.0, -2.0]), "{:?}", b);
        assert!(approx(b.max, [2.0, 1.0, 0.0]), "{:?}", b);
    }

    #[test]
    fn matrix_and_no_scene() {
        // No scenes → roots = nodes not referenced as children. Matrix = uniform scale 0.001 (mm → m).
        let glb = build_glb(&json!({
            "asset": {"version": "2.0"},
            "nodes": [{"matrix": [0.001,0,0,0, 0,0.001,0,0, 0,0,0.001,0, 0,0,0,1], "mesh": 0}],
            "meshes": [{"primitives": [{"attributes": {"POSITION": 0}}]}],
            "accessors": [{"componentType": 5126, "count": 2, "type": "VEC3", "min": [-500, 0, -250], "max": [500, 750, 250]}]
        }));
        assert_eq!(bbox(&glb).unwrap().wdh_mm(), [1000, 500, 750]);
    }

    #[test]
    fn normalized_quantized() {
        let glb = build_glb(&json!({
            "asset": {"version": "2.0"},
            "scenes": [{"nodes": [0]}],
            "nodes": [{"mesh": 0, "scale": [2.0, 2.0, 2.0]}],
            "meshes": [{"primitives": [{"attributes": {"POSITION": 0}}]}],
            "accessors": [{"componentType": 5123, "normalized": true, "count": 2, "type": "VEC3", "min": [0, 0, 0], "max": [65535, 32767.5, 65535]}]
        }));
        let b = bbox(&glb).unwrap();
        assert!(approx(b.max, [2.0, 1.0, 2.0]), "{:?}", b);
    }

    #[test]
    fn no_geometry_is_error() {
        let glb = build_glb(&json!({"asset": {"version": "2.0"}, "scenes": [{"nodes": []}], "nodes": []}));
        assert!(bbox(&glb).is_err());
    }
}
