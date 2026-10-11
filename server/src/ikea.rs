//! IKEA product import (isolated – the website is not an official API and may change).
//!
//! Sources, in order of preference:
//! 1. Product page (`/xx/yy/p/<slug>-<article>/`): JSON-LD `Product` (name, category, price, images,
//!    width/depth/height), JSON-LD `3DModel` (all GLB variants on web-api.ikea.com/dimma), and the
//!    embedded hydration JSON (`"product":{"itemNo"…}`, `"measurements":[{"measure"…}]`).
//! 2. The rotera data API used by IKEA's own «In 3D ansehen» viewer:
//!    `GET https://web-api.ikea.com/{cc}/{lang}/rotera/data/model/{itemNo}/` (header `x-client-id`)
//!    → `modelUrl` (`…/rotera/static/models/{itemNo}-mini.glb`, Draco + WebP) + measurements in mm.
//! 3. LLM fallback (name + dims only) if the page can't be parsed.

use std::sync::OnceLock;
use std::time::Duration;

use plan_core::assets::{Asset, AssetKind, AssetLight, IkeaMeta};
use plan_core::primitives::PrimitiveShape;
use plan_core::LightKind;
use regex::Regex;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};
use crate::glb;
use crate::state::AppState;

pub const BROWSER_UA: &str =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36";
const MAX_GLB_BYTES: usize = 60 * 1024 * 1024;
const MAX_PAGE_BYTES: usize = 8 * 1024 * 1024;
/// Relative deviation BBox vs. product dimensions that triggers a warning.
const TOLERANCE: f64 = 0.03;

pub const UPLOAD_HINT: &str = "Alternativ das Möbel als GLB-Datei hochladen oder eine Basisform verwenden.";

// ---------------------------------------------------------------------------------------------
// Pure parsing (unit-tested against fixtures)
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProductDims {
    pub width: Option<u32>,
    pub depth: Option<u32>,
    pub height: Option<u32>,
    /// Largest diameter (lamps, round tables …).
    pub diameter: Option<u32>,
}

impl ProductDims {
    fn fill_from(&mut self, o: &ProductDims) {
        self.width = self.width.or(o.width);
        self.depth = self.depth.or(o.depth);
        self.height = self.height.or(o.height);
        self.diameter = self.diameter.or(o.diameter);
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProductInfo {
    /// 8 digits, no dots, no `s` prefix.
    pub article_no: String,
    pub name: String,
    pub type_name: Option<String>,
    pub category: Option<String>,
    pub breadcrumbs: Vec<String>,
    pub dims: ProductDims,
    pub price: Option<f64>,
    pub currency: Option<String>,
    pub image_url: Option<String>,
    /// GLB URLs from the JSON-LD `3DModel` (fallback if rotera is unavailable).
    pub model_urls: Vec<String>,
}

impl ProductInfo {
    /// All descriptive text, lowercased (for shape/lamp heuristics).
    fn haystack(&self) -> String {
        let mut s = vec![self.name.clone()];
        s.extend(self.type_name.clone());
        s.extend(self.category.clone());
        s.extend(self.breadcrumbs.iter().cloned());
        s.join(" | ").to_lowercase()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct IkeaUrl {
    pub url: String,
    pub country: String,
    pub lang: String,
    pub article: Option<String>,
}

fn re(cell: &'static OnceLock<Regex>, pat: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pat).expect("regex"))
}

pub fn is_ikea_host(host: &str) -> bool {
    let h = host.to_ascii_lowercase();
    h == "ikea.com" || h.ends_with(".ikea.com")
}

/// Normalise user input: full product URL, URL without scheme, or a bare article number.
pub fn parse_input(input: &str, default_country: &str, default_lang: &str) -> Result<IkeaUrl, String> {
    static ART: OnceLock<Regex> = OnceLock::new();
    let input = input.trim();
    let bare = input.replace(['.', ' '], "");
    if re(&ART, r"^[sS]?\d{8}$").is_match(&bare) {
        let art = bare.trim_start_matches(['s', 'S']).to_string();
        return Ok(IkeaUrl {
            url: format!("https://www.ikea.com/{default_country}/{default_lang}/p/-{art}/"),
            country: default_country.into(),
            lang: default_lang.into(),
            article: Some(art),
        });
    }
    let with_scheme = if input.starts_with("http://") || input.starts_with("https://") { input.to_string() } else { format!("https://{input}") };
    let url = reqwest::Url::parse(&with_scheme).map_err(|_| "Das ist keine gültige URL.".to_string())?;
    if !url.host_str().map(is_ikea_host).unwrap_or(false) {
        return Err("Nur Produkt-Links von ikea.com werden unterstützt.".into());
    }
    let segs: Vec<&str> = url.path_segments().map(|s| s.filter(|x| !x.is_empty()).collect()).unwrap_or_default();
    let (country, lang) = match segs.as_slice() {
        [c, l, ..] if c.len() == 2 && (l.len() == 2 || l.len() == 5) => (c.to_lowercase(), l.to_lowercase()),
        _ => (default_country.to_string(), default_lang.to_string()),
    };
    if !segs.contains(&"p") {
        return Err("Das ist kein IKEA-Produktlink (erwartet …/p/<produkt>-<artikelnummer>/).".into());
    }
    let article = segs.last().and_then(|last| article_from_slug(last));
    let mut clean = url.clone();
    clean.set_query(None);
    clean.set_fragment(None);
    clean.set_scheme("https").ok();
    Ok(IkeaUrl { url: clean.to_string(), country, lang, article })
}

pub fn article_from_slug(slug: &str) -> Option<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    re(&RE, r"(?:^|-)[sS]?(\d{8})$").captures(slug).map(|c| c[1].to_string())
}

/// "80 cm" → 800, "31.5 cm" → 315, "1,5 m" → 1500, "450 mm" → 450.
pub fn parse_measure_mm(s: &str) -> Option<u32> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let c = re(&RE, r"(?i)(\d+(?:[.,]\d+)?)\s*(mm|cm|m)\b").captures(s)?;
    let v: f64 = c[1].replace(',', ".").parse().ok()?;
    let mm = match c[2].to_lowercase().as_str() {
        "mm" => v,
        "cm" => v * 10.0,
        _ => v * 1000.0,
    };
    (mm > 0.0 && mm < 100_000.0).then(|| mm.round() as u32)
}

/// Extract a balanced JSON array/object starting at `start` (string-aware).
pub fn balanced_json(s: &str, start: usize) -> Option<&str> {
    let bytes = s.as_bytes();
    let (open, close) = match bytes.get(start)? {
        b'[' => (b'[', b']'),
        b'{' => (b'{', b'}'),
        _ => return None,
    };
    let (mut depth, mut in_str, mut esc) = (0i32, false, false);
    for (i, &c) in bytes.iter().enumerate().skip(start) {
        if in_str {
            if esc {
                esc = false;
            } else if c == b'\\' {
                esc = true;
            } else if c == b'"' {
                in_str = false;
            }
            continue;
        }
        if c == b'"' {
            in_str = true;
        } else if c == open || c == b'[' || c == b'{' {
            depth += 1;
        } else if c == close || c == b']' || c == b'}' {
            depth -= 1;
            if depth == 0 {
                return s.get(start..=i);
            }
        }
    }
    None
}

/// All occurrences of `"key":<json>` located by `marker` (which starts with `"key":`), parsed.
fn all_json_after<'a>(html: &'a str, marker: &'a str) -> impl Iterator<Item = (usize, Value)> + 'a {
    html.match_indices(marker).filter_map(move |(pos, _)| {
        let start = pos + html[pos..].find(':')? + 1;
        Some((pos, serde_json::from_str(balanced_json(html, start)?).ok()?))
    })
}

fn floor_boundary(s: &str, mut i: usize) -> usize {
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// The `productId` the hydration props declare closest before `pos` (identifies whose
/// measurements block this is – pages also embed blocks of recommended products).
fn product_id_before(html: &str, pos: usize) -> Option<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let from = floor_boundary(html, pos.saturating_sub(20_000));
    re(&RE, r#""productId":"s?(\d{8})""#).captures_iter(&html[from..pos]).last().map(|c| c[1].to_string())
}

fn norm_article(s: &str) -> String {
    s.replace('.', "").trim().trim_start_matches(['s', 'S']).to_string()
}

fn json_ld_blocks(html: &str) -> Vec<Value> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let r = re(&RE, r#"(?is)<script[^>]*type\s*=\s*["']application/ld\+json["'][^>]*>(.*?)</script>"#);
    let mut out = vec![];
    for c in r.captures_iter(html) {
        let Ok(v) = serde_json::from_str::<Value>(c[1].trim()) else { continue };
        let mut push = |v: Value| {
            if let Some(g) = v.get("@graph").and_then(Value::as_array) {
                out.extend(g.iter().cloned());
            } else {
                out.push(v);
            }
        };
        match v {
            Value::Array(a) => a.into_iter().for_each(&mut push),
            v => push(v),
        }
    }
    out
}

fn ld_type_is(v: &Value, ty: &str) -> bool {
    match v.get("@type") {
        Some(Value::String(s)) => s == ty,
        Some(Value::Array(a)) => a.iter().any(|x| x.as_str() == Some(ty)),
        _ => false,
    }
}

fn str_or_num(v: Option<&Value>) -> Option<f64> {
    match v? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.replace([',', '\''], ".").trim().parse().ok(),
        _ => None,
    }
}

/// Swiss German: «ss» instead of «ß».
pub fn swiss(s: &str) -> String {
    s.replace('ß', "ss").split_whitespace().collect::<Vec<_>>().join(" ")
}

#[derive(Deserialize)]
struct HydMeasure {
    measure: String,
    #[serde(default)]
    name: String,
    #[serde(default, rename = "type")]
    ty: String,
}

/// Map IKEA product measurements (type codes are language-independent, names as fallback).
fn dims_from_measurements(ms: &[HydMeasure]) -> ProductDims {
    let mut d = ProductDims::default();
    let (mut length, mut headboard) = (None, None);
    for m in ms {
        let Some(mm) = parse_measure_mm(&m.measure) else { continue };
        let name = m.name.to_lowercase();
        match (m.ty.as_str(), name.as_str()) {
            ("00047", _) | (_, "breite") | (_, "width") => d.width = d.width.or(Some(mm)),
            ("00044", _) | (_, "tiefe") | (_, "depth") => d.depth = d.depth.or(Some(mm)),
            ("00041", _) | (_, "höhe") | (_, "height") => d.height = d.height.or(Some(mm)),
            ("00001", _) | (_, "länge") | (_, "length") => length = length.or(Some(mm)),
            ("00110", _) => headboard = headboard.or(Some(mm)),
            (_, n) if n.contains("durchmesser") || n.contains("diameter") => d.diameter = Some(d.diameter.unwrap_or(0).max(mm)),
            _ => {}
        }
    }
    d.depth = d.depth.or(length);
    d.height = d.height.or(headboard);
    d
}

/// Parse a product page. `None` if it doesn't look like a product page at all.
/// `expected_article` (from the URL) pins the main product; pages also embed data of
/// recommended products, so hydration snippets are only used if they belong to it.
pub fn parse_product_page(html: &str, expected_article: Option<&str>) -> Option<ProductInfo> {
    let ld = json_ld_blocks(html);
    let product = ld.iter().find(|v| ld_type_is(v, "Product"));
    let ld_article = product.and_then(|p| p.get("sku").or(p.get("mpn"))?.as_str().map(norm_article));
    let main = expected_article.map(norm_article).or(ld_article);
    let item_no = |v: &Value| v.get("itemNo").and_then(Value::as_str).map(norm_article);
    let hyd_products: Vec<Value> = all_json_after(html, r#""product":{"itemNo""#).map(|(_, v)| v).collect();
    let hyd_product = match &main {
        Some(a) => hyd_products.into_iter().find(|p| item_no(p).as_deref() == Some(a.as_str())),
        None => hyd_products.into_iter().next(),
    };
    if product.is_none() && hyd_product.is_none() {
        return None;
    }
    let mut info = ProductInfo::default();
    info.article_no = main.or_else(|| hyd_product.as_ref().and_then(item_no)).unwrap_or_default();

    // Name
    let hyd_name = hyd_product.as_ref().and_then(|p| {
        let n = p.get("name")?.as_str()?;
        let t = p.get("typeName").and_then(Value::as_str).unwrap_or("");
        Some(format!("{n} {t}").trim().to_string())
    });
    info.type_name = hyd_product.as_ref().and_then(|p| p.get("typeName")?.as_str().map(swiss));
    info.name = swiss(
        &product
            .and_then(|p| p.get("name")?.as_str().map(str::to_string))
            .or(hyd_name)
            .unwrap_or_default(),
    );

    if let Some(p) = product {
        info.category = p.get("category").and_then(Value::as_str).map(swiss);
        let offer = match p.get("offers") {
            Some(Value::Array(a)) => a.first(),
            o => o,
        };
        if let Some(o) = offer {
            info.price = str_or_num(o.get("price")).or_else(|| str_or_num(o.get("lowPrice")));
            info.currency = o.get("priceCurrency").and_then(Value::as_str).map(str::to_string);
        }
        info.image_url = match p.get("image") {
            Some(Value::Array(a)) => a.iter().find_map(|i| i.get("contentUrl").or(Some(i)).and_then(Value::as_str).map(str::to_string)),
            Some(Value::String(s)) => Some(s.clone()),
            Some(o) => o.get("contentUrl").and_then(Value::as_str).map(str::to_string),
            None => None,
        };
        info.dims = ProductDims {
            width: p.get("width").and_then(Value::as_str).and_then(parse_measure_mm),
            depth: p.get("depth").and_then(Value::as_str).and_then(parse_measure_mm),
            height: p.get("height").and_then(Value::as_str).and_then(parse_measure_mm),
            diameter: None,
        };
    }
    if info.price.is_none() {
        info.price = hyd_product.as_ref().and_then(|p| str_or_num(p.get("price")));
        info.currency = info.currency.or_else(|| hyd_product.as_ref().and_then(|p| p.get("currencyCode")?.as_str().map(str::to_string)));
    }

    // Detailed measurements from the hydration JSON take precedence over JSON-LD.
    let blocks: Vec<(Option<String>, Value)> =
        all_json_after(html, r#""measurements":[{"measure""#).map(|(pos, v)| (product_id_before(html, pos), v)).collect();
    let chosen = blocks
        .iter()
        .find(|(pid, _)| pid.is_some() && pid.as_deref() == Some(info.article_no.as_str()))
        .or_else(|| (blocks.len() == 1 && blocks[0].0.is_none()).then(|| &blocks[0]))
        .map(|(_, v)| v.clone());
    if let Some(ms) = chosen.and_then(|v| serde_json::from_value::<Vec<HydMeasure>>(v).ok()) {
        let mut d = dims_from_measurements(&ms);
        d.fill_from(&info.dims);
        info.dims = d;
    }

    if let Some(bc) = ld.iter().find(|v| ld_type_is(v, "BreadcrumbList")) {
        info.breadcrumbs = bc
            .get("itemListElement")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|i| i.get("name")?.as_str().map(swiss)).collect())
            .unwrap_or_default();
    }

    for m in ld.iter().filter(|v| ld_type_is(v, "3DModel")) {
        for e in m.get("encoding").and_then(Value::as_array).into_iter().flatten() {
            let fmt = e.get("encodingFormat").and_then(Value::as_str).unwrap_or("");
            if let Some(u) = e.get("contentUrl").and_then(Value::as_str) {
                if fmt == "model/gltf-binary" || u.split('?').next().unwrap_or("").ends_with(".glb") {
                    info.model_urls.push(u.to_string());
                }
            }
        }
    }
    if info.name.is_empty() && info.article_no.is_empty() {
        return None;
    }
    Some(info)
}

/// Pick the JSON-LD GLB closest to what the 3D viewer uses: realtime-quality (rqp), Draco, mid LOD.
pub fn pick_jsonld_model(urls: &[String]) -> Option<String> {
    let score = |u: &str| -> i32 {
        let l = u.to_lowercase();
        let mut s = 0;
        if l.contains("/rqp3/") {
            s += 30;
        } else if l.contains("/rqp2/") {
            s += 25;
        } else if l.contains("/rqp1/") {
            s += 20;
        } else if l.contains("/iqp1/") {
            s += 10;
        } else if l.contains("/iqp2/") {
            s += 8;
        } else if l.contains("/iqp3/") {
            s += 6;
        }
        if l.contains("glb_draco") {
            s += 5;
        }
        if l.contains("geomagical") {
            s -= 20;
        }
        s
    };
    urls.iter().max_by_key(|u| score(u)).cloned()
}

#[derive(Debug, Clone, PartialEq)]
pub struct RoteraModel {
    pub model_url: String,
    pub dims: ProductDims,
}

/// Parse `rotera/data/model/{itemNo}/` (measurements already in mm).
pub fn parse_rotera(v: &Value) -> Option<RoteraModel> {
    let model_url = v.get("modelUrl")?.as_str()?.to_string();
    let mut d = ProductDims::default();
    let mut length = None;
    for m in v.get("measurements").and_then(Value::as_array).into_iter().flatten() {
        let ty = m.get("measurementType").and_then(Value::as_str).unwrap_or("").to_lowercase();
        let Some(val) = m.get("value").and_then(Value::as_f64).filter(|x| *x > 0.0 && *x < 100_000.0) else { continue };
        let val = val.round() as u32;
        match ty.as_str() {
            "width" => d.width = Some(val),
            "depth" => d.depth = Some(val),
            "height" => d.height = Some(val),
            "length" => length = Some(val),
            t if t.contains("diameter") || t == "dia." || t == "diam." => d.diameter = Some(d.diameter.unwrap_or(0).max(val)),
            _ => {}
        }
    }
    d.depth = d.depth.or(length);
    Some(RoteraModel { model_url, dims: d })
}

pub fn is_lamp(text: &str) -> bool {
    let t = text.to_lowercase();
    t.contains("leuchte") || t.contains("lampe") || t.contains("lamp")
}

/// Placeholder shape from the product type (German compounds: the rightmost keyword is the head noun).
pub fn guess_primitive(text: &str) -> PrimitiveShape {
    const KW: &[(&str, PrimitiveShape)] = &[
        ("sofa", PrimitiveShape::Sofa),
        ("couch", PrimitiveShape::Sofa),
        ("bett", PrimitiveShape::Bed),
        ("bed", PrimitiveShape::Bed),
        ("tisch", PrimitiveShape::Table),
        ("table", PrimitiveShape::Table),
        ("desk", PrimitiveShape::Table),
        ("stuhl", PrimitiveShape::Chair),
        ("sessel", PrimitiveShape::Chair),
        ("hocker", PrimitiveShape::Chair),
        ("chair", PrimitiveShape::Chair),
        ("regal", PrimitiveShape::Shelf),
        ("shelf", PrimitiveShape::Shelf),
        ("bookcase", PrimitiveShape::Shelf),
        ("schrank", PrimitiveShape::Wardrobe),
        ("kommode", PrimitiveShape::Wardrobe),
        ("vitrine", PrimitiveShape::Wardrobe),
        ("wardrobe", PrimitiveShape::Wardrobe),
        ("cabinet", PrimitiveShape::Wardrobe),
        ("leuchte", PrimitiveShape::Cylinder),
        ("lampe", PrimitiveShape::Cylinder),
        ("lamp", PrimitiveShape::Cylinder),
    ];
    let t = text.to_lowercase();
    // Only the head of the type ("Bettgestell mit Aufbewahrung" → "bettgestell").
    let head = t.split([',', '|']).next().unwrap_or("");
    let head = head.split(" mit ").next().unwrap_or(head);
    let head = head.split(" für ").next().unwrap_or(head);
    let find = |s: &str| KW.iter().filter_map(|(k, shape)| s.rfind(k).map(|p| (p + k.len(), *shape))).max_by_key(|(end, _)| *end);
    find(head).or_else(|| find(&t)).map(|(_, s)| s).unwrap_or(PrimitiveShape::Box)
}

/// Light for lamps: point light near the top (90 % of height), 2700 K.
pub fn lamp_light(text: &str, height_mm: u32) -> AssetLight {
    let t = text.to_lowercase();
    let floor = t.contains("stand") || t.contains("steh") || t.contains("floor");
    let table = t.contains("tisch") || t.contains("table") || t.contains("schreibtisch");
    let lumen = if floor {
        800
    } else if table {
        400
    } else if height_mm >= 1000 {
        800
    } else {
        400
    };
    AssetLight { kind: LightKind::Point, offset_mm: [0, 0, (height_mm as f64 * 0.9).round() as i32], lumen, color_temp_k: 2700 }
}

/// Final dims [w, d, h] (mm): explicit product dims > model bbox > diameter > shape default.
/// Returns which axes are explicit product values (those get the plausibility check).
pub fn resolve_dims(product: &ProductDims, bbox_wdh: Option<[u32; 3]>, fallback: [u32; 3]) -> ([u32; 3], [bool; 3]) {
    let explicit = [product.width, product.depth, product.height];
    let mut out = [0u32; 3];
    for i in 0..3 {
        out[i] = explicit[i]
            .or_else(|| bbox_wdh.map(|b| b[i]).filter(|v| *v > 0))
            .or(if i < 2 { product.diameter } else { None })
            .unwrap_or(fallback[i])
            .max(1);
    }
    (out, [explicit[0].is_some(), explicit[1].is_some(), explicit[2].is_some()])
}

fn fmt_mm(v: u32) -> String {
    let s = v.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push('\'');
        }
        out.push(c);
    }
    out
}

/// Compare explicit product dims with the model bbox (X = width, Z = depth, Y = height; X/Z may be
/// swapped if the model is rotated by 90°). Returns German warnings for deviations > 3 %.
pub fn plausibility(dims: [u32; 3], explicit: [bool; 3], bbox_wdh: [u32; 3]) -> Vec<String> {
    let dev = |model: u32, product: u32| (model as f64 - product as f64) / product.max(1) as f64;
    let orient = |b: [u32; 3]| -> (f64, Vec<(usize, u32, f64)>) {
        let mut worst: f64 = 0.0;
        let mut list = vec![];
        for i in 0..3 {
            if explicit[i] {
                let d = dev(b[i], dims[i]);
                worst = worst.max(d.abs());
                list.push((i, b[i], d));
            }
        }
        (worst, list)
    };
    let straight = orient(bbox_wdh);
    let swapped = orient([bbox_wdh[1], bbox_wdh[0], bbox_wdh[2]]);
    let rotated = swapped.0 + 1e-9 < straight.0;
    let (worst, list) = if rotated { swapped } else { straight };
    if worst <= TOLERANCE {
        return vec![];
    }
    let names = ["Breite", "Tiefe", "Höhe"];
    let parts: Vec<String> = list
        .into_iter()
        .filter(|(_, _, d)| d.abs() > TOLERANCE)
        .map(|(i, m, d)| format!("{} {} mm statt {} mm ({:+.1} %)", names[i], fmt_mm(m), fmt_mm(dims[i]), d * 100.0))
        .collect();
    let mut w = vec![format!("3D-Modell weicht von den Produktmassen ab: {}", parts.join(", "))];
    if rotated {
        w.push("3D-Modell scheint um 90° gedreht zu sein (Breite/Tiefe vertauscht)".into());
    }
    w
}

/// Visible page text for the LLM fallback (scripts/styles removed, whitespace collapsed).
pub fn page_text(html: &str, max_chars: usize) -> String {
    static STRIP: OnceLock<Regex> = OnceLock::new();
    let stripped = re(&STRIP, r"(?is)<script\b.*?</script>|<style\b.*?</style>|<noscript\b.*?</noscript>|<svg\b.*?</svg>").replace_all(html, " ");
    let doc = scraper::Html::parse_document(&stripped);
    let text: String = doc.root_element().text().collect::<Vec<_>>().join(" ");
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.chars().take(max_chars).collect()
}

// ---------------------------------------------------------------------------------------------
// Network + orchestration
// ---------------------------------------------------------------------------------------------

pub fn http_client() -> reqwest::Client {
    let policy = reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() >= 5 {
            attempt.error("zu viele Weiterleitungen")
        } else if attempt.url().host_str().map(is_ikea_host).unwrap_or(false) && attempt.url().scheme() == "https" {
            attempt.follow()
        } else {
            attempt.stop()
        }
    });
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(reqwest::header::ACCEPT_LANGUAGE, reqwest::header::HeaderValue::from_static("de-CH,de;q=0.9,en;q=0.5"));
    reqwest::Client::builder()
        .user_agent(BROWSER_UA)
        .default_headers(headers)
        .redirect(policy)
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(10))
        .build()
        .expect("http client")
}

async fn fetch_limited(http: &reqwest::Client, url: &str, accept: &str, limit: usize) -> Result<(u16, Vec<u8>), String> {
    let mut res = http.get(url).header(reqwest::header::ACCEPT, accept).send().await.map_err(|e| format!("{e}"))?;
    let status = res.status().as_u16();
    let mut buf = Vec::new();
    while let Some(chunk) = res.chunk().await.map_err(|e| format!("{e}"))? {
        buf.extend_from_slice(&chunk);
        if buf.len() > limit {
            return Err(format!("Antwort zu gross (> {} MB)", limit / 1024 / 1024));
        }
    }
    Ok((status, buf))
}

async fn fetch_rotera(st: &AppState, u: &IkeaUrl, article: &str) -> Option<RoteraModel> {
    let url = format!("https://web-api.ikea.com/{}/{}/rotera/data/model/{}/", u.country, u.lang, article);
    let res = st
        .ikea_http
        .get(&url)
        .header(reqwest::header::ACCEPT, "application/json;version=2")
        .header("x-client-id", &st.cfg.ikea_rotera_client_id)
        .timeout(Duration::from_secs(15))
        .send()
        .await;
    match res {
        Ok(r) if r.status().is_success() => {
            let v: Value = r.json().await.ok()?;
            let m = parse_rotera(&v)?;
            let host_ok = reqwest::Url::parse(&m.model_url).ok().and_then(|x| x.host_str().map(is_ikea_host)).unwrap_or(false);
            host_ok.then_some(m)
        }
        Ok(r) => {
            tracing::info!(article, status = %r.status(), "rotera: kein 3D-Modell");
            None
        }
        Err(e) => {
            tracing::warn!(article, "rotera nicht erreichbar: {e}");
            None
        }
    }
}

/// IKEA's 3D model (GLB) of an article, if there is one.
pub async fn article_glb(st: &AppState, country: &str, lang: &str, article: &str) -> Option<Vec<u8>> {
    let u = IkeaUrl { url: String::new(), country: country.into(), lang: lang.into(), article: Some(article.into()) };
    let m = fetch_rotera(st, &u, article).await?;
    match download_glb(st, &m.model_url).await {
        Ok(b) => Some(b),
        Err(e) => {
            tracing::warn!(article, "Modell nicht ladbar: {e}");
            None
        }
    }
}

async fn download_glb(st: &AppState, url: &str) -> Result<Vec<u8>, String> {
    let parsed = reqwest::Url::parse(url).map_err(|_| "ungültige Modell-URL".to_string())?;
    if !parsed.host_str().map(is_ikea_host).unwrap_or(false) {
        return Err("Modell-URL nicht auf ikea.com".into());
    }
    let (status, bytes) = fetch_limited(&st.ikea_http, url, "model/gltf-binary,*/*", MAX_GLB_BYTES).await?;
    if status != 200 {
        return Err(format!("HTTP {status}"));
    }
    if !glb::is_glb(&bytes) {
        return Err("keine GLB-Datei".into());
    }
    Ok(bytes)
}

#[derive(Deserialize)]
struct LlmProduct {
    name: Option<String>,
    width_mm: Option<f64>,
    depth_mm: Option<f64>,
    height_mm: Option<f64>,
    article_no: Option<String>,
}

async fn llm_fallback(st: &AppState, html: &str, u: &IkeaUrl) -> Result<ProductInfo, String> {
    let llm = st.llm.clone().ok_or("Die Seite enthält keine lesbaren Produktdaten (Link prüfen – oder IKEA hat die Seite geändert), und die KI-Auswertung ist nicht konfiguriert.")?;
    let text = page_text(html, 15_000);
    if text.len() < 50 {
        return Err("Produktseite enthält keinen lesbaren Text.".into());
    }
    let schema = json!({
        "type": "object",
        "properties": {
            "name": {"type": "string", "description": "Produktname inkl. Typ, z. B. «BILLY Bücherregal»"},
            "width_mm": {"type": ["number", "null"], "description": "Breite in mm"},
            "depth_mm": {"type": ["number", "null"], "description": "Tiefe (bzw. Länge) in mm"},
            "height_mm": {"type": ["number", "null"], "description": "Höhe in mm"},
            "article_no": {"type": ["string", "null"], "description": "IKEA-Artikelnummer, 8 Ziffern"}
        },
        "required": ["name", "width_mm", "depth_mm", "height_mm", "article_no"]
    });
    let user = format!("URL: {}\n\nSeitentext:\n{text}", u.url);
    // Spawned so a panic inside the client can't take the request handler down.
    let task = tokio::spawn(async move {
        llm.structured(
            "Du extrahierst Produktdaten aus einer IKEA-Produktseite. Masse in Millimetern (Möbelmasse, nicht Verpackung). Unbekannt = null.",
            &user,
            None,
            "product_info",
            schema,
        )
        .await
    });
    let v = match task.await {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => return Err(format!("KI-Auswertung fehlgeschlagen: {e:#}")),
        Err(_) => return Err("KI-Auswertung fehlgeschlagen (interner Fehler)".into()),
    };
    let p: LlmProduct = serde_json::from_value(v).map_err(|e| format!("KI-Antwort unbrauchbar: {e}"))?;
    let mm = |x: Option<f64>| x.filter(|v| v.is_finite() && *v > 0.0 && *v < 100_000.0).map(|v| v.round() as u32);
    let article = p
        .article_no
        .map(|a| a.replace(['.', ' '], "").trim_start_matches(['s', 'S']).to_string())
        .filter(|a| a.len() == 8 && a.chars().all(|c| c.is_ascii_digit()))
        .or_else(|| u.article.clone())
        .ok_or("Artikelnummer nicht gefunden")?;
    Ok(ProductInfo {
        article_no: article,
        name: swiss(p.name.as_deref().unwrap_or("IKEA-Produkt")),
        dims: ProductDims { width: mm(p.width_mm), depth: mm(p.depth_mm), height: mm(p.height_mm), diameter: None },
        ..Default::default()
    })
}

fn fail(msg: impl std::fmt::Display) -> ApiError {
    ApiError::unprocessable(format!("{msg} {UPLOAD_HINT}"))
}

/// Full import: cache lookup → page → rotera/JSON-LD model → bbox check → store.
pub async fn import(st: &AppState, input: &str) -> ApiResult<Asset> {
    let u = parse_input(input, &st.cfg.ikea_country, &st.cfg.ikea_lang).map_err(ApiError::unprocessable)?;
    if let Some(a) = &u.article {
        if let Some(asset) = crate::assets::find_by_article(&st.db, a).await? {
            return Ok(asset);
        }
    }
    let (status, body) = fetch_limited(&st.ikea_http, &u.url, "text/html,application/xhtml+xml", MAX_PAGE_BYTES)
        .await
        .map_err(|e| fail(format!("IKEA-Seite nicht erreichbar ({e}).")))?;
    if status == 404 {
        return Err(fail("Produkt nicht gefunden (IKEA meldet 404). Ist der Link korrekt?"));
    }
    if status != 200 {
        return Err(fail(format!("IKEA-Seite nicht abrufbar (HTTP {status}).")));
    }
    let html = String::from_utf8_lossy(&body).into_owned();
    let mut warnings = vec![];
    let mut info = match parse_product_page(&html, u.article.as_deref()) {
        Some(i) if !i.article_no.is_empty() => i,
        parsed => {
            tracing::warn!(url = %u.url, "IKEA-Parser: keine Produktdaten, versuche KI-Fallback");
            let mut i = llm_fallback(st, &html, &u).await.map_err(|e| fail(format!("Produktdaten nicht lesbar: {e}")))?;
            if let Some(p) = parsed {
                i.image_url = p.image_url;
                i.price = p.price;
                i.currency = p.currency;
            }
            warnings.push("Produktdaten per KI ermittelt – Masse bitte prüfen".to_string());
            i
        }
    };
    if info.name.is_empty() {
        info.name = format!("IKEA {}", info.article_no);
    }
    if let Some(asset) = crate::assets::find_by_article(&st.db, &info.article_no).await? {
        return Ok(asset);
    }

    // 3D model: rotera («In 3D ansehen») first, JSON-LD 3DModel as fallback.
    let rotera = fetch_rotera(st, &u, &info.article_no).await;
    if let Some(r) = &rotera {
        info.dims.fill_from(&r.dims);
    }
    let mut candidates: Vec<String> = rotera.iter().map(|r| r.model_url.clone()).collect();
    candidates.extend(pick_jsonld_model(&info.model_urls));
    let mut model: Option<(Vec<u8>, glb::BBox)> = None;
    for url in candidates {
        match download_glb(st, &url).await {
            Ok(bytes) => match glb::bbox(&bytes) {
                Ok(bb) => {
                    model = Some((bytes, bb));
                    break;
                }
                Err(e) => tracing::warn!(%url, "IKEA-GLB unbrauchbar: {e:#}"),
            },
            Err(e) => tracing::warn!(%url, "IKEA-GLB nicht ladbar: {e}"),
        }
    }

    let text = info.haystack();
    let lamp = is_lamp(&text);
    let shape = if lamp { PrimitiveShape::Cylinder } else { guess_primitive(info.type_name.as_deref().unwrap_or(&info.name)) };
    let bbox_wdh = model.as_ref().map(|(_, b)| b.wdh_mm());
    let (dims, explicit) = resolve_dims(&info.dims, bbox_wdh, shape.default_dims());
    if let Some(b) = bbox_wdh {
        warnings.extend(plausibility(dims, explicit, b));
    } else {
        warnings.push("Kein 3D-Modell verfügbar – Basisform als Platzhalter".into());
    }
    if bbox_wdh.is_none() && !explicit.iter().all(|e| *e) {
        warnings.push("Nicht alle Masse auf der Produktseite gefunden – fehlende Werte geschätzt".into());
    }

    let id = Uuid::new_v4();
    let model_url = if let Some((bytes, _)) = model {
        crate::assets::put_model(st, id, bytes).await?;
        Some(format!("/api/assets/{id}/model.glb"))
    } else {
        None
    };
    let thumb = info.image_url.as_ref().map(|u| if u.contains('?') { u.clone() } else { format!("{u}?f=s") });
    let asset = Asset {
        id,
        kind: AssetKind::Ikea,
        name: info.name.clone(),
        dims_mm: dims,
        model_url: model_url.clone(),
        thumbnail_url: thumb,
        primitive: if model_url.is_none() { Some(shape) } else { None },
        ikea: Some(IkeaMeta {
            url: u.url.clone(),
            article_no: info.article_no.clone(),
            price: info.price,
            currency: info.currency.clone(),
            image_url: info.image_url.clone(),
        }),
        light: lamp.then(|| lamp_light(&text, dims[2])),
        warnings,
        created_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    };
    crate::assets::insert_ikea(st, asset).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
    }

    #[test]
    fn input_parsing() {
        let u = parse_input("https://www.ikea.com/ch/de/p/billy-buecherregal-weiss-00263850/?utm=x#y", "ch", "de").unwrap();
        assert_eq!(u.article.as_deref(), Some("00263850"));
        assert_eq!((u.country.as_str(), u.lang.as_str()), ("ch", "de"));
        assert_eq!(u.url, "https://www.ikea.com/ch/de/p/billy-buecherregal-weiss-00263850/");
        let u = parse_input("ikea.com/de/de/p/klippan-2er-sofa-vissle-grau-s79010614", "ch", "de").unwrap();
        assert_eq!(u.article.as_deref(), Some("79010614"));
        assert_eq!(u.country, "de");
        let u = parse_input("002.638.50", "ch", "fr").unwrap();
        assert_eq!(u.url, "https://www.ikea.com/ch/fr/p/-00263850/");
        assert!(parse_input("https://evil.example/ch/de/p/x-00263850/", "ch", "de").is_err());
        assert!(parse_input("https://www.ikea.com.evil.example/ch/de/p/x-00263850/", "ch", "de").is_err());
        assert!(parse_input("https://www.ikea.com/ch/de/cat/regale/", "ch", "de").is_err());
    }

    #[test]
    fn measures() {
        assert_eq!(parse_measure_mm("80 cm"), Some(800));
        assert_eq!(parse_measure_mm("31.5 cm"), Some(315));
        assert_eq!(parse_measure_mm("1,5 m"), Some(1500));
        assert_eq!(parse_measure_mm("450 mm"), Some(450));
        assert_eq!(parse_measure_mm("30 kg"), None);
        assert_eq!(parse_measure_mm("100 W"), None);
    }

    #[test]
    fn balanced() {
        let s = r#"xx "a":[{"b":"]}\"["},[1,2]] tail"#;
        let start = s.find('[').unwrap();
        assert_eq!(balanced_json(s, start), Some(r#"[{"b":"]}\"["},[1,2]]"#));
    }

    #[test]
    fn billy_fixture() {
        let p = parse_product_page(&fixture("ikea_billy.html"), Some("00263850")).unwrap();
        assert_eq!(p.article_no, "00263850");
        assert_eq!(p.name, "BILLY Bücherregal - weiss 80x28x202 cm");
        assert_eq!(p.type_name.as_deref(), Some("Bücherregal"));
        assert_eq!(p.dims, ProductDims { width: Some(800), depth: Some(280), height: Some(2020), diameter: None });
        assert_eq!(p.price, Some(59.95));
        assert_eq!(p.currency.as_deref(), Some("CHF"));
        assert!(p.image_url.as_deref().unwrap().starts_with("https://www.ikea.com/ch/de/images/products/billy-buecherregal-weiss__"));
        assert!(p.model_urls.len() > 5);
        let m = pick_jsonld_model(&p.model_urls).unwrap();
        assert!(m.contains("/rqp3/glb_draco/"), "{m}");
        assert_eq!(guess_primitive(p.type_name.as_deref().unwrap()), PrimitiveShape::Shelf);
        assert!(!is_lamp(&p.haystack()));
    }

    #[test]
    fn decoy_products_are_ignored() {
        // Recommended product data before the main product must not leak into the result.
        let html = r#"<script type="application/ld+json">{"@type":"Product","name":"MAIN Regal","sku":"111.111.11"}</script>
            <script>{"product":{"itemNo":"99999999","name":"DECOY","typeName":"Sessel","price":1}}
            {"productDescriptionProps":{"productId":"99999999"},"measurementsProps":{"measurements":[{"measure":"58 cm","name":"Breite","type":"00047"}]}}
            {"product":{"itemNo":"11111111","name":"MAIN","typeName":"Regal","price":10}}
            {"productDescriptionProps":{"productId":"11111111"},"measurementsProps":{"measurements":[{"measure":"80 cm","name":"Breite","type":"00047"}]}}</script>"#;
        let p = parse_product_page(html, None).unwrap();
        assert_eq!(p.article_no, "11111111");
        assert_eq!(p.type_name.as_deref(), Some("Regal"));
        assert_eq!(p.dims.width, Some(800));
        // URL article wins over everything else
        let p = parse_product_page(html, Some("99999999")).unwrap();
        assert_eq!(p.article_no, "99999999");
        assert_eq!(p.dims.width, Some(580));
    }

    #[test]
    fn klippan_fixture_combination_article() {
        let p = parse_product_page(&fixture("ikea_klippan.html"), Some("79010614")).unwrap();
        assert_eq!(p.article_no, "79010614");
        assert_eq!(p.dims, ProductDims { width: Some(1800), depth: Some(880), height: Some(660), diameter: None });
        assert_eq!(p.price, Some(249.0));
        assert_eq!(guess_primitive(p.type_name.as_deref().unwrap()), PrimitiveShape::Sofa);
    }

    #[test]
    fn malm_fixture_length_and_headboard() {
        let p = parse_product_page(&fixture("ikea_malm.html"), None).unwrap();
        assert_eq!(p.article_no, "90404799");
        // Länge → depth, Kopfteilhöhe → height (no plain «Höhe» for beds)
        assert_eq!(p.dims, ProductDims { width: Some(1550), depth: Some(2100), height: Some(1000), diameter: None });
        assert_eq!(guess_primitive(p.type_name.as_deref().unwrap()), PrimitiveShape::Bed);
    }

    #[test]
    fn lamp_fixtures() {
        let p = parse_product_page(&fixture("ikea_hektar.html"), Some("00215307")).unwrap();
        assert_eq!(p.article_no, "00215307");
        assert_eq!(p.dims, ProductDims { width: None, depth: None, height: Some(1810), diameter: Some(340) });
        assert!(is_lamp(&p.haystack()));
        let l = lamp_light(&p.haystack(), 1810);
        assert_eq!((l.lumen, l.offset_mm[2], l.color_temp_k), (800, 1629, 2700));

        let t = parse_product_page(&fixture("ikea_arstid_tisch.html"), None).unwrap();
        assert_eq!(t.dims.height, Some(550));
        assert_eq!(t.dims.diameter, Some(220));
        assert_eq!(lamp_light(&t.haystack(), 550).lumen, 400);
        // Width/depth come from the model bbox if present, else from the diameter.
        let (d, e) = resolve_dims(&t.dims, Some([222, 223, 540]), [1, 1, 1]);
        assert_eq!((d, e), ([222, 223, 550], [false, false, true]));
        let (d, _) = resolve_dims(&t.dims, None, [1, 1, 1]);
        assert_eq!(d, [220, 220, 550]);
    }

    #[test]
    fn rotera_parsing() {
        let v: Value = serde_json::from_str(&fixture("rotera_malm.json")).unwrap();
        let r = parse_rotera(&v).unwrap();
        assert_eq!(r.model_url, "https://web-api.ikea.com/ch/de/rotera/static/models/90404799-mini.glb");
        assert_eq!(r.dims, ProductDims { width: Some(1550), depth: Some(2100), height: None, diameter: None });
        let v: Value = serde_json::from_str(&fixture("rotera_hektar.json")).unwrap();
        let r = parse_rotera(&v).unwrap();
        assert_eq!(r.dims, ProductDims { width: None, depth: None, height: Some(1810), diameter: Some(340) });
    }

    #[test]
    fn plausibility_checks() {
        // KLIPPAN: model 1801×894×673 vs 1800×880×660 → within 3 %
        assert!(plausibility([1800, 880, 660], [true; 3], [1801, 894, 673]).is_empty());
        // HEKTAR-like: depth way off
        let w = plausibility([340, 340, 1810], [true, true, true], [358, 627, 1834]);
        assert_eq!(w.len(), 1);
        assert!(w[0].contains("Tiefe 627 mm statt 340 mm"), "{w:?}");
        // Rotated model: X/Z swapped → matched sensibly, note added
        let w = plausibility([2000, 900, 750], [true; 3], [900, 2000, 900]);
        assert!(w[0].contains("Höhe 900 mm statt 750 mm"), "{w:?}");
        assert!(w[1].contains("gedreht"));
        // Non-explicit axes are not checked
        assert!(plausibility([222, 223, 550], [false, false, true], [222, 223, 540]).is_empty());
        assert!(plausibility([1000, 1000, 1000], [true; 3], [1100, 1000, 1000])[0].contains("Breite 1'100 mm statt 1'000 mm (+10.0 %)"));
    }

    #[test]
    fn shapes() {
        assert_eq!(guess_primitive("Sofatisch"), PrimitiveShape::Table);
        assert_eq!(guess_primitive("Bettsofa"), PrimitiveShape::Sofa);
        assert_eq!(guess_primitive("Schreibtischstuhl"), PrimitiveShape::Chair);
        assert_eq!(guess_primitive("Kleiderschrank"), PrimitiveShape::Wardrobe);
        assert_eq!(guess_primitive("Bettgestell mit Aufbewahrung"), PrimitiveShape::Bed);
        assert_eq!(guess_primitive("Regal mit Türen"), PrimitiveShape::Shelf);
        assert_eq!(guess_primitive("Kissenbezug"), PrimitiveShape::Box);
    }

    #[test]
    fn non_product_pages() {
        assert!(parse_product_page("<html><body><h1>Hallo</h1></body></html>", None).is_none());
        // JSON-LD only (no hydration) still works
        let html = r#"<script type="application/ld+json">{"@type":"Product","name":"TEST Tisch","sku":"123.456.78","width":"120 cm","depth":"60 cm","height":"74 cm","offers":{"price":"99.00","priceCurrency":"CHF"}}</script>"#;
        let p = parse_product_page(html, None).unwrap();
        assert_eq!(p.article_no, "12345678");
        assert_eq!(p.dims, ProductDims { width: Some(1200), depth: Some(600), height: Some(740), diameter: None });
        assert_eq!(page_text("<html><head><script>var x=1;</script><style>a{}</style></head><body><p>Breite:  80&nbsp;cm</p></body></html>", 100), "Breite: 80 cm");
    }
}
