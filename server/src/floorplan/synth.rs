//! Synthetic floor plans with known geometry: built with plan-core commands, rendered like a
//! typical listing plan (filled walls, door leaves + arcs, window lines, fake text, furniture,
//! dimension chains). Used by tests and `floorplan_eval --generate`.

use image::{DynamicImage, Rgb, RgbImage};
use plan_core::commands::{add_wall_chain, ChainPoint};
use plan_core::geom::*;
use plan_core::walls::footprints;
use plan_core::{Floor, Hinge, Id, Op, Opening, OpeningKind, Project, RefImage, RoomMeta, Swing, WallKind};
use uuid::Uuid;

use super::semantics::{SemDim, SemDoor, SemRoom, SemWall, SemWindow, Semantics};

pub struct SynthCase {
    pub name: &'static str,
    /// Truth floor in mm; `reference_image` holds the true mm/px and origin of the rendered image.
    pub truth: Floor,
    pub mm_per_px: f64,
    pub origin_mm: P,
    pub png: Vec<u8>,
    /// Dimension lines drawn (mm endpoints, value).
    pub dims: Vec<(P, P, f64)>,
}

struct Fb {
    p: Project,
    f: Id,
}

impl Fb {
    fn new() -> Self {
        let p = Project::new(Uuid::new_v4(), "synth");
        let f = p.floors[0].id;
        Fb { p, f }
    }
    fn chain(&mut self, pts: &[(i32, i32)], closed: bool, t: u32) {
        let cps: Vec<ChainPoint> = pts.iter().map(|&(x, y)| ChainPoint { node: None, wall: None, x, y }).collect();
        let kind = if t >= 250 { WallKind::Exterior } else { WallKind::Interior };
        let ops = add_wall_chain(&self.p, self.f, &cps, closed, t, kind).unwrap();
        plan_core::apply_batch(&mut self.p, &ops).unwrap();
    }
    fn floor(&self) -> &Floor {
        &self.p.floors[0]
    }
    /// Opening centred at `at` (must lie on a wall centre line).
    fn opening(&mut self, at: (i32, i32), width: u32, kind: OpeningKind) {
        let at = [at.0 as f64, at.1 as f64];
        let f = self.floor();
        let (w, a, b) = f
            .walls
            .iter()
            .find_map(|w| {
                let (a, b) = f.wall_points(w)?;
                let (t, _, d) = closest_on_segment(at, a, b);
                (d < 1.0 && t > 0.0 && t < 1.0).then(|| (w.clone(), a, b))
            })
            .expect("opening not on a wall");
        let l = dist(a, b);
        let along = dot(sub(at, a), norm(sub(b, a)));
        let height = if matches!(kind, OpeningKind::Window { .. }) { 1400 } else { 2050 };
        let o = Opening { id: Uuid::new_v4(), wall: w.id, kind, offset_mm: (along - width as f64 / 2.0).round() as u32, width_mm: width, height_mm: height };
        assert!(o.offset_mm as f64 + width as f64 <= l);
        plan_core::apply_batch(&mut self.p, &[Op::PutOpening { floor: self.f, opening: o }]).unwrap();
    }
    fn room(&mut self, name: &str, at: (i32, i32)) {
        let r = RoomMeta {
            id: Uuid::new_v4(),
            anchor: [at.0, at.1],
            name: name.into(),
            floor_material: if super::cleanup::is_wet_room(name) { "tile:white".into() } else { "wood:oak".into() },
            ceiling_material: "paint:white".into(),
            open_ceiling: false,
        };
        plan_core::apply_batch(&mut self.p, &[Op::PutRoom { floor: self.f, room: r }]).unwrap();
    }
}

fn door(h: Hinge, s: Swing) -> OpeningKind {
    OpeningKind::Door { hinge: h, swing: s }
}
const WIN: OpeningKind = OpeningKind::Window { sill_mm: 850 };

/// Two rooms + bathroom, 9 × 6 m, exterior 300 mm, interior 120 mm.
fn case_a() -> (Floor, f64, Vec<(P, P, f64)>) {
    let mut b = Fb::new();
    b.chain(&[(0, 0), (9000, 0), (9000, 6000), (0, 6000)], true, 300);
    b.chain(&[(5000, 0), (5000, 6000)], false, 120);
    b.chain(&[(5000, 3500), (9000, 3500)], false, 120);
    b.opening((5000, 1500), 900, door(Hinge::Start, Swing::Left));
    b.opening((7000, 3500), 800, door(Hinge::End, Swing::Right));
    b.opening((2000, 6000), 1000, door(Hinge::Start, Swing::Right));
    b.opening((1800, 0), 1600, WIN);
    b.opening((7100, 0), 1200, WIN);
    b.opening((9000, 1700), 1200, WIN);
    b.opening((0, 2600), 1200, WIN);
    b.room("Wohnen", (2500, 3000));
    b.room("Zimmer", (7000, 1700));
    b.room("Bad", (7000, 4800));
    let dims = vec![([-150.0, -900.0], [9150.0, -900.0], 9300.0), ([-900.0, -150.0], [-900.0, 6150.0], 6300.0)];
    (b.floor().clone(), 9.0, dims)
}

/// Corridor flat, 11 × 8 m, exterior 365 mm, five rooms with T and cross junctions.
fn case_b() -> (Floor, f64, Vec<(P, P, f64)>) {
    let mut b = Fb::new();
    b.chain(&[(0, 0), (11000, 0), (11000, 8000), (0, 8000)], true, 365);
    // corridor between y=3600 and y=4800
    b.chain(&[(0, 3600), (11000, 3600)], false, 120);
    b.chain(&[(3500, 4800), (11000, 4800)], false, 120);
    b.chain(&[(4500, 0), (4500, 3600)], false, 150);
    b.chain(&[(7800, 0), (7800, 3600)], false, 120);
    b.chain(&[(3500, 3600), (3500, 8000)], false, 120);
    b.chain(&[(7000, 4800), (7000, 8000)], false, 120);
    b.opening((2200, 3600), 900, door(Hinge::Start, Swing::Left));
    b.opening((6100, 3600), 800, door(Hinge::End, Swing::Left));
    b.opening((9400, 3600), 800, door(Hinge::Start, Swing::Left));
    b.opening((5200, 4800), 900, door(Hinge::End, Swing::Right));
    b.opening((9000, 4800), 800, door(Hinge::Start, Swing::Right));
    b.opening((11000, 4200), 1000, door(Hinge::Start, Swing::Left));
    b.opening((2200, 0), 2000, WIN);
    b.opening((6100, 0), 1400, WIN);
    b.opening((9400, 0), 1200, WIN);
    b.opening((1700, 8000), 2400, WIN);
    b.opening((5200, 8000), 1400, WIN);
    b.opening((0, 6000), 1200, WIN);
    b.room("Wohnen/Essen", (1700, 5800));
    b.room("Zimmer 1", (2200, 1800));
    b.room("Zimmer 2", (6100, 1800));
    b.room("Küche", (9400, 1800));
    b.room("Bad", (5200, 6400));
    b.room("Reduit", (9000, 6400));
    let dims = vec![([-180.0, -1000.0], [11180.0, -1000.0], 11360.0), ([-180.0, -600.0], [4500.0, -600.0], 4680.0)];
    (b.floor().clone(), 7.0, dims)
}

/// L-shaped flat with a 45° corner wall, 12 mm/px (small image → upscaled).
fn case_c() -> (Floor, f64, Vec<(P, P, f64)>) {
    let mut b = Fb::new();
    b.chain(&[(0, 0), (8000, 0), (8000, 3000), (10000, 5000), (10000, 9000), (0, 9000)], true, 300);
    b.chain(&[(0, 4500), (9500, 4500)], false, 120);
    b.chain(&[(4500, 4500), (4500, 9000)], false, 120);
    b.opening((2500, 4500), 900, door(Hinge::End, Swing::Left));
    b.opening((7000, 4500), 900, door(Hinge::Start, Swing::Right));
    b.opening((4500, 7000), 800, door(Hinge::Start, Swing::Left));
    b.opening((4000, 0), 2400, WIN);
    b.opening((2200, 9000), 1400, WIN);
    b.opening((7200, 9000), 1400, WIN);
    b.opening((9000, 4000), 1200, WIN);
    b.room("Wohnen", (4000, 2200));
    b.room("Schlafen", (2200, 6700));
    b.room("Zimmer", (7200, 6700));
    let dims = vec![([-150.0, -900.0], [8150.0, -900.0], 8300.0)];
    (b.floor().clone(), 12.0, dims)
}

pub fn cases() -> Vec<SynthCase> {
    let mut out = Vec::new();
    for (i, (name, (floor, mpp, dims))) in [("synth_zwei_zimmer", case_a()), ("synth_korridor", case_b()), ("synth_l_form", case_c())].into_iter().enumerate() {
        let (img, origin) = render(&floor, mpp, &dims, 1000 + i as u64);
        let mut truth = floor;
        truth.reference_image = Some(RefImage {
            url: "plan.png".into(),
            width_px: img.width(),
            height_px: img.height(),
            origin_mm: origin,
            mm_per_px: mpp,
            opacity: 0.5,
            visible: true,
        });
        let png = super::preprocess::encode_png(&DynamicImage::ImageRgb8(img)).unwrap();
        out.push(SynthCase { name, truth, mm_per_px: mpp, origin_mm: origin, png, dims });
    }
    out
}

// ---- rendering --------------------------------------------------------------------------------------

struct Canvas {
    img: RgbImage,
    origin: P,
    mpp: f64,
}

const INK: Rgb<u8> = Rgb([25, 25, 25]);
const LINE: Rgb<u8> = Rgb([60, 60, 60]);
const WHITE: Rgb<u8> = Rgb([255, 255, 255]);

impl Canvas {
    fn px(&self, p: P) -> P {
        [(p[0] - self.origin[0]) / self.mpp, (p[1] - self.origin[1]) / self.mpp]
    }
    fn mm(&self, x: f64, y: f64) -> P {
        [x * self.mpp + self.origin[0], y * self.mpp + self.origin[1]]
    }
    fn fill_poly(&mut self, poly: &[P], c: Rgb<u8>) {
        let pts: Vec<P> = poly.iter().map(|p| self.px(*p)).collect();
        let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for p in &pts {
            x0 = x0.min(p[0]);
            y0 = y0.min(p[1]);
            x1 = x1.max(p[0]);
            y1 = y1.max(p[1]);
        }
        let (w, h) = (self.img.width() as i64, self.img.height() as i64);
        for y in (y0.floor() as i64).max(0)..(y1.ceil() as i64).min(h) {
            for x in (x0.floor() as i64).max(0)..(x1.ceil() as i64).min(w) {
                if point_in_polygon([x as f64 + 0.5, y as f64 + 0.5], &pts) {
                    self.img.put_pixel(x as u32, y as u32, c);
                }
            }
        }
    }
    fn dot_px(&mut self, p: P, r: f64, c: Rgb<u8>) {
        let (w, h) = (self.img.width() as i64, self.img.height() as i64);
        let ri = r.ceil() as i64;
        for dy in -ri..=ri {
            for dx in -ri..=ri {
                let (x, y) = (p[0].floor() as i64 + dx, p[1].floor() as i64 + dy);
                if x >= 0 && y >= 0 && x < w && y < h && ((dx * dx + dy * dy) as f64) <= r * r + 0.01 {
                    self.img.put_pixel(x as u32, y as u32, c);
                }
            }
        }
    }
    /// Line in mm with width in px.
    fn line(&mut self, a: P, b: P, width_px: f64, c: Rgb<u8>) {
        let (pa, pb) = (self.px(a), self.px(b));
        let n = (dist(pa, pb) * 2.0).ceil().max(1.0) as usize;
        for i in 0..=n {
            let p = lerp(pa, pb, i as f64 / n as f64);
            self.dot_px(p, (width_px - 1.0) / 2.0, c);
        }
    }
    fn arc(&mut self, center: P, u: P, v: P, r: f64, c: Rgb<u8>) {
        let steps = (r / self.mpp * 2.0).ceil().max(8.0) as usize;
        let mut prev = add(center, mul(u, r));
        for i in 1..=steps {
            let th = std::f64::consts::FRAC_PI_2 * i as f64 / steps as f64;
            let p = add(center, add(mul(u, r * th.cos()), mul(v, r * th.sin())));
            self.line(prev, p, 1.0, c);
            prev = p;
        }
    }
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((self.0 >> 33) as f64) / ((1u64 << 31) as f64)
    }
}

/// Fake text: strokes 1–2 px in boxes of `h_mm` height.
fn text(cv: &mut Canvas, at: P, chars: usize, h_mm: f64, rng: &mut Rng) {
    let w = h_mm * 0.6;
    let start = [at[0] - chars as f64 * w * 0.6, at[1] - h_mm / 2.0];
    for i in 0..chars {
        let x = start[0] + i as f64 * w * 1.2;
        let y = start[1];
        let k = (rng.next() * 5.0) as usize;
        let tl = [x, y];
        let tr = [x + w, y];
        let bl = [x, y + h_mm];
        let br = [x + w, y + h_mm];
        let ml = [x, y + h_mm / 2.0];
        let mr = [x + w, y + h_mm / 2.0];
        let lw = 1.0 + (rng.next() * 1.5).floor();
        match k {
            0 => {
                cv.line(tl, bl, lw, LINE);
                cv.line(bl, br, lw, LINE);
            }
            1 => {
                cv.line(tl, bl, lw, LINE);
                cv.line(tr, br, lw, LINE);
                cv.line(ml, mr, lw, LINE);
            }
            2 => {
                cv.line(tl, tr, lw, LINE);
                cv.line(tr, bl, lw, LINE);
                cv.line(bl, br, lw, LINE);
            }
            3 => {
                cv.line(tl, br, lw, LINE);
                cv.line(tr, bl, lw, LINE);
            }
            _ => {
                cv.line(tl, tr, lw, LINE);
                cv.line(tl, bl, lw, LINE);
                cv.line(bl, br, lw, LINE);
                cv.line(br, mr, lw, LINE);
                cv.line(mr, ml, lw, LINE);
            }
        }
    }
}

pub fn render(f: &Floor, mpp: f64, dims: &[(P, P, f64)], seed: u64) -> (RgbImage, P) {
    let mut rng = Rng(seed);
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for n in &f.nodes {
        x0 = x0.min(n.x_mm as f64);
        y0 = y0.min(n.y_mm as f64);
        x1 = x1.max(n.x_mm as f64);
        y1 = y1.max(n.y_mm as f64);
    }
    let margin = 1600.0;
    let origin = [x0 - margin, y0 - margin];
    let w = ((x1 - x0 + 2.0 * margin) / mpp).ceil() as u32;
    let h = ((y1 - y0 + 2.0 * margin) / mpp).ceil() as u32;
    let mut cv = Canvas { img: RgbImage::from_pixel(w, h, WHITE), origin, mpp };
    // furniture + labels first (walls drawn over them)
    for r in &f.rooms {
        let c = [r.anchor[0] as f64, r.anchor[1] as f64];
        let (fw, fd) = (1200.0 + rng.next() * 600.0, 700.0 + rng.next() * 300.0);
        let fc = add(c, [0.0, 900.0]);
        let p = [add(fc, [-fw / 2.0, -fd / 2.0]), add(fc, [fw / 2.0, -fd / 2.0]), add(fc, [fw / 2.0, fd / 2.0]), add(fc, [-fw / 2.0, fd / 2.0])];
        for i in 0..4 {
            cv.line(p[i], p[(i + 1) % 4], 1.0, LINE);
        }
        cv.arc(add(c, [-900.0, -900.0]), [1.0, 0.0], [0.0, 1.0], 350.0, LINE);
        cv.arc(add(c, [-900.0, -900.0]), [-1.0, 0.0], [0.0, -1.0], 350.0, LINE);
        cv.arc(add(c, [-900.0, -900.0]), [0.0, 1.0], [-1.0, 0.0], 350.0, LINE);
        cv.arc(add(c, [-900.0, -900.0]), [0.0, -1.0], [1.0, 0.0], 350.0, LINE);
        text(&mut cv, c, r.name.chars().count().min(8), 220.0, &mut rng);
        text(&mut cv, add(c, [0.0, 330.0]), 5, 160.0, &mut rng);
    }
    // walls
    for fp in footprints(f) {
        cv.fill_poly(&fp.corners, INK);
    }
    // plan-core footprints leave a triangular notch in the through-wall at T-junctions:
    // fill it by extending walls that continue collinearly at a node.
    for w in &f.walls {
        let Some((a, b)) = f.wall_points(w) else { continue };
        let d = norm(sub(b, a));
        let h = w.thickness_mm as f64 / 2.0;
        let collinear_at = |node: Id| {
            f.walls_at(node).filter(|o| o.id != w.id).any(|o| {
                f.wall_points(o).is_some_and(|(p, q)| cross(norm(sub(q, p)), d).abs() < 1e-6)
            })
        };
        let ea = if collinear_at(w.a) { h } else { 0.0 };
        let eb = if collinear_at(w.b) { h } else { 0.0 };
        if ea == 0.0 && eb == 0.0 {
            continue;
        }
        let n = left_normal(d);
        let (p0, p1) = (sub(a, mul(d, ea)), add(b, mul(d, eb)));
        cv.fill_poly(&[add(p0, mul(n, h)), add(p1, mul(n, h)), sub(p1, mul(n, h)), sub(p0, mul(n, h))], INK);
    }
    // openings
    for o in &f.openings {
        let Some(w) = f.wall(o.wall) else { continue };
        let Some((a, b)) = f.wall_points(w) else { continue };
        let d = norm(sub(b, a));
        let n = left_normal(d);
        let t = w.thickness_mm as f64;
        let s0 = o.offset_mm as f64;
        let s1 = s0 + o.width_mm as f64;
        let e = t / 2.0 + 1.5 * mpp;
        let q = |s: f64, c: f64| add(add(a, mul(d, s)), mul(n, c));
        cv.fill_poly(&[q(s0, -e), q(s1, -e), q(s1, e), q(s0, e)], WHITE);
        let face = t / 2.0 - 0.5 * mpp;
        match o.kind {
            OpeningKind::Window { .. } => {
                for c in [-face, -t / 8.0, t / 8.0, face] {
                    cv.line(q(s0, c), q(s1, c), 1.0, LINE);
                }
                cv.line(q(s0, -face), q(s0, face), 1.0, LINE);
                cv.line(q(s1, -face), q(s1, face), 1.0, LINE);
            }
            OpeningKind::Door { hinge, swing } => {
                let side = if swing == Swing::Left { 1.0 } else { -1.0 };
                let (hs, other) = if hinge == Hinge::Start { (s0, s1) } else { (s1, s0) };
                let hp = q(hs, side * t / 2.0);
                let wd = o.width_mm as f64;
                let nn = mul(n, side);
                cv.line(hp, add(hp, mul(nn, wd)), 2.0, LINE);
                let toward = if other > hs { d } else { mul(d, -1.0) };
                cv.arc(hp, nn, toward, wd, LINE);
            }
            OpeningKind::Passage => {}
        }
    }
    // dimension chains
    for (a, b, v) in dims {
        cv.line(*a, *b, 1.0, LINE);
        let d = norm(sub(*b, *a));
        let n = perp(d);
        for p in [*a, *b] {
            cv.line(add(p, mul(add(d, n), 120.0)), sub(p, mul(add(d, n), 120.0)), 2.0, LINE);
            cv.line(add(p, mul(n, 250.0)), sub(p, mul(n, 250.0)), 1.0, LINE);
        }
        let mid = lerp(*a, *b, 0.5);
        let _ = v;
        text(&mut cv, add(mid, mul(n, -250.0)), 4, 200.0, &mut rng);
    }
    // a north arrow (thin)
    let na = cv.mm(w as f64 - 60.0, 80.0);
    cv.line(na, add(na, [0.0, 600.0]), 1.0, LINE);
    cv.line(na, add(na, [-150.0, 250.0]), 1.0, LINE);
    cv.line(na, add(na, [150.0, 250.0]), 1.0, LINE);
    (cv.img, origin)
}

/// LLM answer as a perfect-ish model would give it (px of the rectified image, small jitter).
pub fn mock_semantics(case: &SynthCase) -> Semantics {
    let f = &case.truth;
    let px = |p: P| [(p[0] - case.origin_mm[0]) / case.mm_per_px + 3.0, (p[1] - case.origin_mm[1]) / case.mm_per_px - 2.0];
    // the preprocess step trims white borders and rescales: map into the rectified image
    let prep = super::preprocess::preprocess(&case.png, None).unwrap();
    let shift = (prep.src_offset[0], prep.src_offset[1], prep.src_scale);
    let px = |p: P| {
        let q = px(p);
        [(q[0] - shift.0) * shift.2, (q[1] - shift.1) * shift.2]
    };
    let mut s = Semantics { north_deg: Some(30.0), scale_text: Some("1:100".into()), ..Default::default() };
    for r in &f.rooms {
        s.rooms.push(SemRoom { name: r.name.clone(), pos: px([r.anchor[0] as f64, r.anchor[1] as f64]), area_m2: None });
    }
    for w in &f.walls {
        let (a, b) = f.wall_points(w).unwrap();
        s.walls.push(SemWall { a: px(a), b: px(b), thickness: Some(w.thickness_mm as f64 / case.mm_per_px * shift.2), exterior: Some(w.kind == WallKind::Exterior) });
    }
    for o in &f.openings {
        let w = f.wall(o.wall).unwrap();
        let (a, b) = f.wall_points(w).unwrap();
        let d = norm(sub(b, a));
        let c = add(a, mul(d, o.offset_mm as f64 + o.width_mm as f64 / 2.0));
        let width = o.width_mm as f64 / case.mm_per_px * shift.2;
        match o.kind {
            OpeningKind::Window { .. } => s.windows.push(SemWindow { pos: px(c), width }),
            OpeningKind::Door { hinge, swing } => {
                let hs = if hinge == Hinge::Start { o.offset_mm as f64 } else { (o.offset_mm + o.width_mm) as f64 };
                let side = if swing == Swing::Left { 1.0 } else { -1.0 };
                let into = add(c, mul(left_normal(d), side * 600.0));
                s.doors.push(SemDoor { pos: px(c), width, hinge: Some(px(add(a, mul(d, hs)))), opens_into: Some(px(into)) });
            }
            OpeningKind::Passage => {}
        }
    }
    for (a, b, v) in &case.dims {
        // LLM endpoints a bit sloppy (±3 px)
        let mut pa = px(*a);
        let mut pb = px(*b);
        pa[0] -= 2.0;
        pb[0] += 3.0;
        s.dims.push(SemDim { value_mm: *v, a: pa, b: pb, text: format!("{:.2}", v / 1000.0) });
    }
    s
}
