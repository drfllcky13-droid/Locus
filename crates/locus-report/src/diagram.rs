//! Diagrams printed to scale. The stored diagram (project metres, see
//! `app/src/diagram2d/model.ts`) is converted to paper millimetres here,
//! `paper = (world − origin) × 1000 / scale`, with y turned down for the page, and the
//! template only places what it is given. So a 10 m line at 1:100 is exactly 100 mm long,
//! which the tests check on the laid-out document.

use crate::{render_with, Rendered};
use serde::{Deserialize, Serialize};
use std::f64::consts::TAU;

const TEMPLATE: &str = include_str!("../templates/diagram.typ");

pub type Pt = [f64; 2];

/// The diagram document as stored (the fields printing needs).
#[derive(Debug, Clone, Deserialize)]
pub struct Diagram {
    pub layers: Vec<Layer>,
    pub entities: Vec<Entity>,
}

impl Diagram {
    /// Project files (with their recorded SHA-256) of the underlays on visible layers.
    pub fn underlays(&self) -> Vec<(&str, &str)> {
        self.entities
            .iter()
            .filter(|e| self.visible(e))
            .filter_map(|e| match e {
                Entity::Underlay { file, sha256, .. } => Some((file.as_str(), sha256.as_str())),
                _ => None,
            })
            .collect()
    }

    fn visible(&self, e: &Entity) -> bool {
        self.layers
            .iter()
            .find(|l| l.id == e.layer())
            .is_none_or(|l| l.visible)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Layer {
    pub id: String,
    pub visible: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Entity {
    Line {
        layer: String,
        a: Pt,
        b: Pt,
    },
    Polyline {
        layer: String,
        points: Vec<Pt>,
        closed: bool,
    },
    Arc {
        layer: String,
        center: Pt,
        radius: f64,
        start: f64,
        end: f64,
    },
    Dimension {
        layer: String,
        a: Pt,
        b: Pt,
        offset: f64,
    },
    Text {
        layer: String,
        at: Pt,
        text: String,
        height: f64,
        rotation: f64,
    },
    Symbol {
        layer: String,
        symbol: String,
        at: Pt,
        rotation: f64,
        scale: f64,
    },
    Marker {
        layer: String,
        number: u32,
        at: Pt,
    },
    Point {
        layer: String,
        at: Pt,
        label: String,
        measurement: Option<serde_json::Value>,
    },
    North {
        layer: String,
        at: Pt,
        rotation: f64,
    },
    Scalebar {
        layer: String,
        at: Pt,
        length: f64,
    },
    Legend {
        layer: String,
        at: Pt,
    },
    /// Built items (app/src/diagram2d/builders.ts): printed from the geometry stored with them.
    Room {
        layer: String,
        geometry: Built,
    },
    Road {
        layer: String,
        geometry: Built,
    },
    /// An image under the drawing (app/src/diagram2d/underlay.ts): project file, hash, and
    /// the similarity placing its pixels in the project frame.
    Underlay {
        layer: String,
        file: String,
        sha256: String,
        width: f64,
        height: f64,
        placement: Placement,
        opacity: f64,
    },
}

#[derive(Debug, Clone, Deserialize)]
pub struct Placement {
    pub origin: Pt,
    pub pixel: f64,
    pub rotation: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Built {
    pub segments: Vec<BuiltSegment>,
    pub arcs: Vec<BuiltArc>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BuiltSegment {
    pub a: Pt,
    pub b: Pt,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BuiltArc {
    pub center: Pt,
    pub radius: f64,
    pub start: f64,
    pub end: f64,
}

impl Entity {
    fn layer(&self) -> &str {
        match self {
            Entity::Line { layer, .. }
            | Entity::Polyline { layer, .. }
            | Entity::Arc { layer, .. }
            | Entity::Dimension { layer, .. }
            | Entity::Text { layer, .. }
            | Entity::Symbol { layer, .. }
            | Entity::Marker { layer, .. }
            | Entity::Point { layer, .. }
            | Entity::North { layer, .. }
            | Entity::Scalebar { layer, .. }
            | Entity::Legend { layer, .. }
            | Entity::Room { layer, .. }
            | Entity::Road { layer, .. }
            | Entity::Underlay { layer, .. } => layer,
        }
    }
}

/// A symbol from the library: its SVG and its default size on the ground (m).
pub struct SymbolDef {
    pub id: &'static str,
    pub name: &'static str,
    pub size: f64,
    pub svg: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Paper {
    A4,
    A3,
}

#[derive(Debug, Clone)]
pub struct PrintOptions {
    /// 100 for 1:100.
    pub scale: f64,
    pub paper: Paper,
    pub landscape: bool,
    pub title: String,
    /// Lines for the title block, already worded ("Project", "Riverside"), …
    pub details: Vec<(String, String)>,
}

impl PrintOptions {
    /// Page size (mm).
    fn page(&self) -> (f64, f64) {
        let (w, h) = match self.paper {
            Paper::A4 => (210.0, 297.0),
            Paper::A3 => (297.0, 420.0),
        };
        if self.landscape {
            (h, w)
        } else {
            (w, h)
        }
    }
}

/// Margins and title block (mm).
const MARGIN: f64 = 12.0;
const TITLE_BLOCK: f64 = 34.0;

/// Text on the sheet. `centred`: the point is the middle of the text's baseline (dimension
/// values); otherwise it is the text's top-left corner.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Label {
    pub x: f64,
    pub y: f64,
    /// Size (mm).
    pub size: f64,
    /// Degrees, clockwise on the page.
    pub rotation: f64,
    pub text: String,
    pub centred: bool,
}

/// A legend row: label, and a symbol file or glyph name (marker, point, measured).
pub type LegendRow = (String, String);

/// An underlay on the page: top-left corner (mm), size (mm), rotation (degrees, clockwise on
/// the page) about that corner.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Underlay {
    pub file: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub rotation: f64,
    /// 0–1; printed by laying white over the image.
    pub opacity: f64,
}

/// What the template draws, all in page millimetres (origin top left, y down).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Sheet {
    pub page: [f64; 2],
    pub frame: [f64; 4],
    pub scale_label: String,
    pub title: String,
    pub details: Vec<[String; 2]>,
    /// Underlay images, drawn first and clipped to the frame.
    pub images: Vec<Underlay>,
    /// [x1, y1, x2, y2, stroke width].
    pub lines: Vec<[f64; 5]>,
    /// Polylines (arcs are sampled finely enough to be within 0.01 mm of the circle).
    pub paths: Vec<Vec<[f64; 2]>>,
    pub texts: Vec<Label>,
    /// Symbol images: file name, centre x, y, width (mm), rotation (degrees).
    pub symbols: Vec<(String, f64, f64, f64, f64)>,
    /// Evidence markers: x, y, number.
    pub markers: Vec<(f64, f64, u32)>,
    /// Reference and measured points: x, y, label, measured.
    pub points: Vec<(f64, f64, String, bool)>,
    pub norths: Vec<(f64, f64, f64)>,
    /// Scale bars: x, y (left end), length on paper (mm), the length it stands for (label).
    pub scalebars: Vec<(f64, f64, f64, String)>,
    /// Legends: x, y, rows (label, symbol file or glyph name).
    pub legends: Vec<(f64, f64, Vec<LegendRow>)>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PrintError {
    Empty,
    /// The drawing is this big on paper (mm) and the frame is that big.
    DoesNotFit {
        drawing: [f64; 2],
        frame: [f64; 2],
    },
}

impl std::fmt::Display for PrintError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PrintError::Empty => write!(f, "the diagram has nothing to print"),
            PrintError::DoesNotFit { drawing, frame } => write!(
                f,
                "at this scale the drawing is {:.0} × {:.0} mm but the sheet's frame is {:.0} × {:.0} mm; \
                 choose a larger paper or a smaller scale (a larger 1:n)",
                drawing[0], drawing[1], frame[0], frame[1]
            ),
        }
    }
}

fn arc_points(c: Pt, r: f64, start: f64, end: f64, scale_mm: f64) -> Vec<Pt> {
    let mut sweep = end - start;
    while sweep <= 0.0 {
        sweep += TAU;
    }
    // Chord error r(1 − cos(θ/2)) under 0.01 mm on paper.
    let r_mm = r * scale_mm;
    let step = if r_mm > 0.02 {
        2.0 * (1.0 - 0.01 / r_mm).acos()
    } else {
        sweep
    };
    let n = ((sweep / step).ceil() as usize).clamp(2, 4096);
    (0..=n)
        .map(|i| {
            let t = start + sweep * i as f64 / n as f64;
            [c[0] + r * t.cos(), c[1] + r * t.sin()]
        })
        .collect()
}

/// Lay the diagram out on the sheet at the chosen scale.
pub fn sheet(d: &Diagram, symbols: &[SymbolDef], o: &PrintOptions) -> Result<Sheet, PrintError> {
    let visible: Vec<&Entity> = d.entities.iter().filter(|e| d.visible(e)).collect();
    let mm_per_m = 1000.0 / o.scale;
    let sym = |id: &str| symbols.iter().find(|s| s.id == id);

    // Extent of the drawing in world metres (points of every entity).
    let mut pts: Vec<Pt> = vec![];
    let mut underlay_pts: Vec<Pt> = vec![];
    for e in &visible {
        match e {
            Entity::Line { a, b, .. } | Entity::Dimension { a, b, .. } => pts.extend([*a, *b]),
            Entity::Polyline { points, .. } => pts.extend(points),
            Entity::Arc {
                center,
                radius,
                start,
                end,
                ..
            } => pts.extend(arc_points(*center, *radius, *start, *end, mm_per_m)),
            Entity::Text { at, .. }
            | Entity::Marker { at, .. }
            | Entity::Point { at, .. }
            | Entity::North { at, .. }
            | Entity::Legend { at, .. } => pts.push(*at),
            Entity::Symbol {
                at, symbol, scale, ..
            } => {
                let half = sym(symbol).map_or(0.0, |s| s.size * scale / 2.0);
                pts.extend([[at[0] - half, at[1] - half], [at[0] + half, at[1] + half]]);
            }
            Entity::Scalebar { at, length, .. } => pts.extend([*at, [at[0] + length, at[1]]]),
            // An underlay (often far bigger than the drawing) is clipped to the frame instead,
            // unless there is nothing else: then the underlay is the drawing.
            Entity::Underlay {
                width,
                height,
                placement: pl,
                ..
            } => {
                let (c, s) = (pl.rotation.cos(), pl.rotation.sin());
                for (u, v) in [(0.0, 0.0), (*width, 0.0), (0.0, *height), (*width, *height)] {
                    underlay_pts.push([
                        pl.origin[0] + pl.pixel * (u * c + v * s),
                        pl.origin[1] + pl.pixel * (u * s - v * c),
                    ]);
                }
            }
            Entity::Room { geometry, .. } | Entity::Road { geometry, .. } => {
                for g in &geometry.segments {
                    pts.extend([g.a, g.b]);
                }
                for a in &geometry.arcs {
                    pts.extend(arc_points(a.center, a.radius, a.start, a.end, mm_per_m));
                }
            }
        }
    }
    if pts.is_empty() {
        pts = underlay_pts;
    }
    if pts.is_empty() {
        return Err(PrintError::Empty);
    }
    let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    for p in &pts {
        for k in 0..2 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    let (pw, ph) = o.page();
    let frame = [MARGIN, MARGIN, pw - MARGIN, ph - MARGIN - TITLE_BLOCK];
    let (fw, fh) = (frame[2] - frame[0], frame[3] - frame[1]);
    let drawing = [(hi[0] - lo[0]) * mm_per_m, (hi[1] - lo[1]) * mm_per_m];
    // Room for text, markers and the legend around the geometry: 10 mm each side.
    if drawing[0] + 20.0 > fw || drawing[1] + 20.0 > fh {
        return Err(PrintError::DoesNotFit {
            drawing,
            frame: [fw, fh],
        });
    }
    // Centre the drawing in the frame.
    let centre = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0];
    let (cx, cy) = ((frame[0] + frame[2]) / 2.0, (frame[1] + frame[3]) / 2.0);
    let p = |w: Pt| -> [f64; 2] {
        [
            cx + (w[0] - centre[0]) * mm_per_m,
            cy - (w[1] - centre[1]) * mm_per_m,
        ]
    };
    let deg = |r: f64| -r.to_degrees(); // anticlockwise in the world is clockwise on the page's y-down

    let mut s = Sheet {
        page: [pw, ph],
        frame,
        scale_label: format!("1:{}", o.scale),
        title: o.title.clone(),
        details: o
            .details
            .iter()
            .map(|(k, v)| [k.clone(), v.clone()])
            .collect(),
        images: vec![],
        lines: vec![],
        paths: vec![],
        texts: vec![],
        symbols: vec![],
        markers: vec![],
        points: vec![],
        norths: vec![],
        scalebars: vec![],
        legends: vec![],
    };
    let mut used_symbols: Vec<&str> = vec![];
    let (mut n_markers, mut n_ref, mut n_measured) = (vec![], 0, 0);
    for e in &visible {
        match e {
            Entity::Line { a, b, .. } => {
                let (a, b) = (p(*a), p(*b));
                s.lines.push([a[0], a[1], b[0], b[1], 0.35]);
            }
            Entity::Underlay {
                file,
                width,
                height,
                placement: pl,
                opacity,
                ..
            } => {
                let at = p(pl.origin);
                s.images.push(Underlay {
                    file: file.clone(),
                    x: at[0],
                    y: at[1],
                    width: width * pl.pixel * mm_per_m,
                    height: height * pl.pixel * mm_per_m,
                    rotation: deg(pl.rotation),
                    opacity: opacity.clamp(0.0, 1.0),
                });
            }
            Entity::Room { geometry, .. } | Entity::Road { geometry, .. } => {
                for g in &geometry.segments {
                    let (a, b) = (p(g.a), p(g.b));
                    s.lines.push([a[0], a[1], b[0], b[1], 0.35]);
                }
                for a in &geometry.arcs {
                    let v = arc_points(a.center, a.radius, a.start, a.end, mm_per_m);
                    s.paths.push(v.into_iter().map(p).collect());
                }
            }
            Entity::Polyline { points, closed, .. } => {
                let mut v: Vec<[f64; 2]> = points.iter().map(|q| p(*q)).collect();
                if *closed && v.len() > 2 {
                    v.push(v[0]);
                }
                s.paths.push(v);
            }
            Entity::Arc {
                center,
                radius,
                start,
                end,
                ..
            } => {
                s.paths.push(
                    arc_points(*center, *radius, *start, *end, mm_per_m)
                        .into_iter()
                        .map(p)
                        .collect(),
                );
            }
            Entity::Dimension { a, b, offset, .. } => {
                let len = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
                if len == 0.0 {
                    continue;
                }
                let n = [-(b[1] - a[1]) / len * offset, (b[0] - a[0]) / len * offset];
                let (a2, b2) = ([a[0] + n[0], a[1] + n[1]], [b[0] + n[0], b[1] + n[1]]);
                for (u, v) in [(*a, a2), (*b, b2), (a2, b2)] {
                    let (u, v) = (p(u), p(v));
                    s.lines.push([u[0], u[1], v[0], v[1], 0.18]);
                }
                // 45° ticks, about 2 mm long, at both ends of the dimension line.
                let (pa, pb) = (p(a2), p(b2));
                let (ux, uy) = (
                    (pb[0] - pa[0]) / (len * mm_per_m),
                    (pb[1] - pa[1]) / (len * mm_per_m),
                );
                let k = std::f64::consts::FRAC_1_SQRT_2;
                let (tx, ty) = ((ux - uy) * k, (uy + ux) * k);
                for q in [pa, pb] {
                    s.lines
                        .push([q[0] - tx, q[1] - ty, q[0] + tx, q[1] + ty, 0.25]);
                }
                let m = p([(a2[0] + b2[0]) / 2.0, (a2[1] + b2[1]) / 2.0]);
                let mut angle = deg((b[1] - a[1]).atan2(b[0] - a[0]));
                // Keep the value readable: never upside down.
                if angle > 90.0 {
                    angle -= 180.0;
                } else if angle < -90.0 {
                    angle += 180.0;
                }
                s.texts.push(Label {
                    x: m[0],
                    y: m[1],
                    size: 2.5,
                    rotation: angle,
                    text: format!("{len:.3} m"),
                    centred: true,
                });
            }
            Entity::Text {
                at,
                text,
                height,
                rotation,
                ..
            } => {
                let q = p(*at);
                s.texts.push(Label {
                    x: q[0],
                    y: q[1],
                    size: *height,
                    rotation: deg(*rotation),
                    text: text.clone(),
                    centred: false,
                });
            }
            Entity::Symbol {
                at,
                symbol,
                rotation,
                scale,
                ..
            } => {
                if let Some(def) = sym(symbol) {
                    let q = p(*at);
                    s.symbols.push((
                        format!("symbols/{}.svg", def.id),
                        q[0],
                        q[1],
                        def.size * scale * mm_per_m,
                        deg(*rotation),
                    ));
                    if !used_symbols.contains(&def.id) {
                        used_symbols.push(def.id);
                    }
                }
            }
            Entity::Marker { at, number, .. } => {
                let q = p(*at);
                s.markers.push((q[0], q[1], *number));
                n_markers.push(*number);
            }
            Entity::Point {
                at,
                label,
                measurement,
                ..
            } => {
                let q = p(*at);
                let measured = measurement.as_ref().is_some_and(|m| !m.is_null());
                s.points.push((q[0], q[1], label.clone(), measured));
                if measured {
                    n_measured += 1;
                } else {
                    n_ref += 1;
                }
            }
            Entity::North { at, rotation, .. } => {
                let q = p(*at);
                s.norths.push((q[0], q[1], deg(*rotation)));
            }
            Entity::Scalebar { at, length, .. } => {
                let q = p(*at);
                s.scalebars
                    .push((q[0], q[1], length * mm_per_m, format!("{length} m")));
            }
            Entity::Legend { .. } => {}
        }
    }
    // Legends list what is drawn (the same rules as the editor's legend).
    let mut rows: Vec<(String, String)> = symbols
        .iter()
        .filter(|d| used_symbols.contains(&d.id))
        .map(|d| (d.name.to_string(), format!("symbols/{}.svg", d.id)))
        .collect();
    if !n_markers.is_empty() {
        n_markers.sort_unstable();
        n_markers.dedup();
        rows.push((
            format!("Evidence markers {}", ranges(&n_markers)),
            "marker".into(),
        ));
    }
    if n_ref > 0 {
        rows.push(("Reference point".into(), "point".into()));
    }
    if n_measured > 0 {
        rows.push(("Measured point (tape)".into(), "measured".into()));
    }
    for e in &visible {
        if let Entity::Legend { at, .. } = e {
            let q = p(*at);
            s.legends.push((q[0], q[1], rows.clone()));
        }
    }
    Ok(s)
}

fn ranges(s: &[u32]) -> String {
    let mut out = vec![];
    let mut i = 0;
    while i < s.len() {
        let mut j = i;
        while j + 1 < s.len() && s[j + 1] == s[j] + 1 {
            j += 1;
        }
        out.push(if i == j {
            s[i].to_string()
        } else {
            format!("{}–{}", s[i], s[j])
        });
        i = j + 1;
    }
    out.join(", ")
}

/// Render the diagram to PDF at scale. `symbols` supplies the symbol SVGs as files.
/// Print a diagram. `underlays` holds each underlay's image bytes by its project file name.
pub fn pdf(
    d: &Diagram,
    symbols: &[SymbolDef],
    o: &PrintOptions,
    underlays: Vec<(String, Vec<u8>)>,
) -> Result<Rendered, String> {
    let s = sheet(d, symbols, o).map_err(|e| e.to_string())?;
    let data = serde_json::to_vec(&s).map_err(|e| e.to_string())?;
    let mut files: Vec<(String, Vec<u8>)> = symbols
        .iter()
        .map(|d| (format!("symbols/{}.svg", d.id), d.svg.as_bytes().to_vec()))
        .collect();
    files.extend(underlays);
    render_with(TEMPLATE, data, files)
}

/// The symbol library (assets/symbols: original, drawn for Locus), as the editor has it.
pub fn symbols() -> Vec<SymbolDef> {
    macro_rules! sym {
        ($id:literal, $name:literal, $size:expr) => {
            SymbolDef {
                id: $id,
                name: $name,
                size: $size,
                svg: include_str!(concat!("../../../assets/symbols/", $id, ".svg")),
            }
        };
    }
    vec![
        sym!("car", "Car", 4.5),
        sym!("truck", "Truck", 8.0),
        sym!("motorcycle", "Motorcycle", 2.2),
        sym!("person", "Person", 0.6),
        sym!("body", "Body outline", 1.8),
        sym!("blood", "Blood", 0.3),
        sym!("cartridge_case", "Cartridge case", 0.1),
        sym!("firearm", "Firearm", 0.3),
        sym!("knife", "Knife", 0.3),
        sym!("shoe_mark", "Footwear impression", 0.3),
        sym!("tire_mark", "Tire mark", 3.0),
        sym!("impact", "Point of impact", 1.0),
        sym!("camera", "Camera position", 0.5),
        sym!("tree", "Tree", 4.0),
        sym!("pole", "Pole or post", 0.3),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use typst::layout::{Frame, FrameItem, Point};

    fn opts(scale: f64) -> PrintOptions {
        PrintOptions {
            scale,
            paper: Paper::A4,
            landscape: true,
            title: "Scene plan".into(),
            details: vec![
                ("Project".into(), "Test".into()),
                ("Revision".into(), "3".into()),
            ],
        }
    }

    fn diagram(json: &str) -> Diagram {
        serde_json::from_str(json).unwrap()
    }

    const PLAN: &str = r##"{"version":1,"layers":[{"id":"base","name":"Base","visible":true,"locked":false,"color":"#fff"}],
      "entities":[
        {"id":"1","layer":"base","kind":"line","a":[-3,1],"b":[7,1]},
        {"id":"2","layer":"base","kind":"scalebar","at":[-3,-4],"length":5},
        {"id":"3","layer":"base","kind":"arc","center":[2,1],"radius":2,"start":0,"end":3.141592653589793},
        {"id":"4","layer":"base","kind":"symbol","symbol":"car","at":[2,-2],"rotation":0.5,"scale":1},
        {"id":"5","layer":"base","kind":"marker","number":1,"at":[0,3],"note":""},
        {"id":"6","layer":"base","kind":"point","at":[-2,2],"label":"A","measurement":null,"sigma":null},
        {"id":"7","layer":"base","kind":"dimension","a":[-3,1],"b":[7,1],"offset":-1},
        {"id":"8","layer":"base","kind":"north","at":[6,4],"rotation":0},
        {"id":"9","layer":"base","kind":"legend","at":[-3,5]},
        {"id":"10","layer":"base","kind":"text","at":[0,-3],"text":"Kerb","height":3,"rotation":0}
      ]}"##;

    #[test]
    fn a_ten_metre_line_is_100_mm_at_1_to_100() {
        let s = sheet(&diagram(PLAN), &symbols(), &opts(100.0)).unwrap();
        let l = s.lines[0];
        let len = ((l[2] - l[0]).powi(2) + (l[3] - l[1]).powi(2)).sqrt();
        assert!((len - 100.0).abs() < 1e-9, "{len} mm");
        assert!((s.scalebars[0].2 - 50.0).abs() < 1e-9);
        assert_eq!(s.scale_label, "1:100");
        // At 1:50 it is 200 mm (on A3: it doesn't fit A4 at that scale).
        let a3 = PrintOptions {
            paper: Paper::A3,
            ..opts(50.0)
        };
        let s = sheet(&diagram(PLAN), &symbols(), &a3).unwrap();
        let l = s.lines[0];
        assert!(((l[2] - l[0]) - 200.0).abs() < 1e-9);
    }

    #[test]
    fn built_items_print_their_stored_geometry() {
        // A 4 m wall face with a door swing (quarter circle, radius 0.9 m), as the editor
        // stores it; the parameters are the editor's business and are ignored here.
        let d = diagram(
            r##"{"version":1,"layers":[{"id":"base","name":"Base","visible":true,"locked":false,"color":"#fff"}],
          "entities":[
            {"id":"r","layer":"base","kind":"room","outline":[[0,0],[4,0],[4,3]],"thickness":0.2,"openings":[],
             "geometry":{"segments":[{"a":[0,0],"b":[4,0],"dashed":false}],
                         "arcs":[{"center":[1,0],"radius":0.9,"start":0,"end":1.5707963267948966}]}},
            {"id":"d","layer":"base","kind":"road","centreline":[[0,-5],[10,-5]],"road":{},
             "geometry":{"segments":[{"a":[0,-5],"b":[3,-5],"dashed":true}],"arcs":[]}}
          ]}"##,
        );
        let s = sheet(&d, &symbols(), &opts(50.0)).unwrap();
        let len = |l: [f64; 5]| ((l[2] - l[0]).powi(2) + (l[3] - l[1]).powi(2)).sqrt();
        assert!((len(s.lines[0]) - 80.0).abs() < 1e-9); // 4 m at 1:50
        assert!((len(s.lines[1]) - 60.0).abs() < 1e-9); // one 3 m dash
                                                        // The swing ends 0.9 m from the hinge: 18 mm.
        let (first, last) = (s.paths[0][0], *s.paths[0].last().unwrap());
        assert!(((first[0] - last[0]).powi(2) + (first[1] - last[1]).powi(2)).sqrt() > 25.0);
        let hinge = [s.lines[0][0] + 20.0, s.lines[0][1]];
        for q in &s.paths[0] {
            let r = ((q[0] - hinge[0]).powi(2) + (q[1] - hinge[1]).powi(2)).sqrt();
            assert!((r - 18.0).abs() < 1e-9);
        }
    }

    #[test]
    fn underlays_are_placed_by_their_calibration_and_set_the_extent_only_alone() {
        // A 1000 × 500 px image at 0.05 m/px (50 × 25 m, bigger than an A4 frame at 1:100),
        // turned 90° anticlockwise, under a 10 m line.
        let d = diagram(
            r##"{"version":1,"layers":[{"id":"base","name":"Base","visible":true,"locked":false,"color":"#fff"}],
          "entities":[
            {"id":"u","layer":"base","kind":"underlay","file":"evidence/1/aerial.svg","sha256":"x",
             "width":1000,"height":500,"opacity":0.6,"calibration":null,
             "placement":{"origin":[5,-3],"pixel":0.05,"rotation":1.5707963267948966}},
            {"id":"l","layer":"base","kind":"line","a":[0,0],"b":[10,0]}
          ]}"##,
        );
        assert_eq!(d.underlays(), vec![("evidence/1/aerial.svg", "x")]);
        let s = sheet(&d, &symbols(), &opts(100.0)).unwrap();
        let (u, l) = (&s.images[0], s.lines[0]);
        assert!((u.width - 500.0).abs() < 1e-9 && (u.height - 250.0).abs() < 1e-9);
        assert!((u.rotation + 90.0).abs() < 1e-9);
        // Its corner is at (5, −3) m: 50 mm right of the line's start and 30 mm below it.
        assert!((u.x - l[0] - 50.0).abs() < 1e-9 && (u.y - l[1] - 30.0).abs() < 1e-9);
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="1000" height="500"><rect width="1000" height="500" fill="#8a8"/></svg>"##;
        let out = pdf(
            &d,
            &symbols(),
            &opts(100.0),
            vec![("evidence/1/aerial.svg".into(), svg.to_vec())],
        )
        .unwrap();
        assert!(out.pdf.starts_with(b"%PDF"));
        // On its own, the underlay is the drawing: 25 m across by 50 m up fits A3 at 1:250.
        let alone = Diagram {
            entities: d.entities[..1].to_vec(),
            ..d
        };
        let a3 = PrintOptions {
            paper: Paper::A3,
            ..opts(250.0)
        };
        let s = sheet(&alone, &symbols(), &a3).unwrap();
        assert!((s.images[0].width - 200.0).abs() < 1e-9);
    }

    /// Every line's end points in the laid-out page (pt), from the frame tree.
    fn lines(frame: &Frame, at: Point, out: &mut Vec<(f64, f64)>) {
        for (pos, item) in frame.items() {
            let p = at + *pos;
            match item {
                FrameItem::Group(g) => lines(&g.frame, p, out),
                FrameItem::Shape(shape, _) => {
                    if let typst::visualize::Geometry::Line(d) = &shape.geometry {
                        out.push((p.x.to_pt(), (p + *d).x.to_pt() - p.x.to_pt()));
                    }
                }
                _ => {}
            }
        }
    }

    /// Sizes (mm) of every rectangle on the page.
    fn rect_sizes(frame: &Frame, out: &mut Vec<(f64, f64)>) {
        for (_, item) in frame.items() {
            match item {
                FrameItem::Group(g) => rect_sizes(&g.frame, out),
                FrameItem::Shape(shape, _) => {
                    if let typst::visualize::Geometry::Rect(size) = &shape.geometry {
                        out.push((size.x.to_mm(), size.y.to_mm()));
                    }
                }
                _ => {}
            }
        }
    }

    #[test]
    fn the_printed_page_measures_correctly() {
        let s = sheet(&diagram(PLAN), &symbols(), &opts(100.0)).unwrap();
        let data = serde_json::to_vec(&s).unwrap();
        let files: Vec<(String, Vec<u8>)> = symbols()
            .iter()
            .map(|d| (format!("symbols/{}.svg", d.id), d.svg.as_bytes().to_vec()))
            .collect();
        let doc = crate::compile(TEMPLATE, data, files).unwrap();
        assert_eq!(doc.pages().len(), 1, "one sheet");
        let page = &doc.pages()[0];
        // A4 landscape.
        assert!((page.frame.width().to_mm() - 297.0).abs() < 1e-6);
        assert!((page.frame.height().to_mm() - 210.0).abs() < 1e-6);
        let mut found = vec![];
        lines(&page.frame, Point::zero(), &mut found);
        // The 10 m line: a horizontal 100 mm line starting where the sheet put it.
        let want_x = typst::layout::Abs::mm(s.lines[0][0]).to_pt();
        let hit = found.iter().find(|(x, dx)| {
            (x - want_x).abs() < 1e-6 && (dx - typst::layout::Abs::mm(100.0).to_pt()).abs() < 1e-6
        });
        assert!(
            hit.is_some(),
            "no 100 mm line at x = {want_x} pt among {found:?}"
        );
        // The 100 mm calibration bar in the title block.
        let mut rects = vec![];
        rect_sizes(&page.frame, &mut rects);
        let bar = rects
            .iter()
            .any(|(w, h)| (w - 100.0).abs() < 1e-6 && (h - 2.0).abs() < 1e-6);
        assert!(bar, "no 100 mm calibration bar among {rects:?}");
        // And the whole thing renders to PDF.
        let out = pdf(&diagram(PLAN), &symbols(), &opts(100.0), vec![]).unwrap();
        assert!(out.pdf.starts_with(b"%PDF"));
        assert!(out.text.contains("Scale 1:100"));
        assert!(out.text.contains("10.000 m"));
        assert!(out.text.contains("Evidence markers 1"));
        let tmp = std::env::temp_dir();
        std::fs::write(tmp.join("locus-diagram.pdf"), &out.pdf).unwrap();
        let files: Vec<(String, Vec<u8>)> = symbols()
            .iter()
            .map(|d| (format!("symbols/{}.svg", d.id), d.svg.as_bytes().to_vec()))
            .collect();
        for (i, png) in crate::pages_png(TEMPLATE, serde_json::to_vec(&s).unwrap(), files)
            .iter()
            .enumerate()
        {
            std::fs::write(tmp.join(format!("locus-diagram-{}.png", i + 1)), png).unwrap();
        }
    }

    #[test]
    fn a_drawing_too_big_for_the_sheet_is_refused_not_shrunk() {
        let err = sheet(&diagram(PLAN), &symbols(), &opts(20.0)).unwrap_err();
        assert!(matches!(err, PrintError::DoesNotFit { .. }), "{err:?}");
        let empty = diagram(r#"{"version":1,"layers":[],"entities":[]}"#);
        assert_eq!(
            sheet(&empty, &symbols(), &opts(100.0)).unwrap_err(),
            PrintError::Empty
        );
    }

    #[test]
    fn every_symbol_prints() {
        let lib = symbols();
        let ents: Vec<String> = lib
            .iter()
            .enumerate()
            .map(|(i, d)| {
                format!(
                    r#"{{"id":"{i}","layer":"base","kind":"symbol","symbol":"{}","at":[{},{}],"rotation":0,"scale":1}}"#,
                    d.id,
                    (i % 5) as f64 * 8.0,
                    (i / 5) as f64 * 8.0
                )
            })
            .collect();
        let d = diagram(&format!(
            r#"{{"version":1,"layers":[{{"id":"base","visible":true}}],"entities":[{}]}}"#,
            ents.join(",")
        ));
        let o = PrintOptions {
            paper: Paper::A3,
            ..opts(200.0)
        };
        let out = pdf(&d, &lib, &o, vec![]).unwrap();
        assert!(out.pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn the_embedded_library_matches_the_editors() {
        let index: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../../../assets/symbols/index.json")).unwrap();
        let ids: Vec<&str> = index.iter().map(|v| v["id"].as_str().unwrap()).collect();
        assert_eq!(ids, symbols().iter().map(|s| s.id).collect::<Vec<_>>());
    }
}
