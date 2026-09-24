//! A diagram as DXF (ASCII, AutoCAD R12), in world metres in the project frame, one DXF layer
//! per diagram layer. Lines, polylines, arcs, built rooms and roads (as their stored geometry),
//! dimensions (line and length), text, markers, points, north arrows and scale bars are
//! written as drawn; a symbol is written as a circle of its size and its name, and underlay
//! images are left out. Hidden layers are left out.

use crate::diagram::{Diagram, Entity, SymbolDef};
use std::fmt::Write;

/// A layer name DXF accepts: letters, digits, `-` and `_`.
fn layer_name(d: &Diagram, id: &str) -> String {
    let name = d
        .layers
        .iter()
        .find(|l| l.id == id)
        .and_then(|l| l.name.clone())
        .unwrap_or_else(|| id.to_string());
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if s.is_empty() {
        "0".into()
    } else {
        s
    }
}

struct Out(String);

impl Out {
    fn pair(&mut self, code: i32, v: impl std::fmt::Display) {
        let _ = write!(self.0, "{code}\r\n{v}\r\n");
    }
    fn num(&mut self, code: i32, v: f64) {
        // Enough digits for 0.1 mm anywhere on Earth in metres.
        self.pair(code, format!("{v:.6}"));
    }
    fn line(&mut self, layer: &str, a: [f64; 2], b: [f64; 2]) {
        self.pair(0, "LINE");
        self.pair(8, layer);
        self.num(10, a[0]);
        self.num(20, a[1]);
        self.num(30, 0.0);
        self.num(11, b[0]);
        self.num(21, b[1]);
        self.num(31, 0.0);
    }
    fn arc(&mut self, layer: &str, c: [f64; 2], r: f64, start: f64, end: f64) {
        self.pair(0, "ARC");
        self.pair(8, layer);
        self.num(10, c[0]);
        self.num(20, c[1]);
        self.num(30, 0.0);
        self.num(40, r);
        self.num(50, start.to_degrees().rem_euclid(360.0));
        self.num(51, end.to_degrees().rem_euclid(360.0));
    }
    fn circle(&mut self, layer: &str, c: [f64; 2], r: f64) {
        self.pair(0, "CIRCLE");
        self.pair(8, layer);
        self.num(10, c[0]);
        self.num(20, c[1]);
        self.num(30, 0.0);
        self.num(40, r);
    }
    fn text(&mut self, layer: &str, at: [f64; 2], h: f64, rot: f64, s: &str) {
        self.pair(0, "TEXT");
        self.pair(8, layer);
        self.num(10, at[0]);
        self.num(20, at[1]);
        self.num(30, 0.0);
        self.num(40, h);
        // DXF R12 text is 8-bit: other characters become '?'.
        let t: String = s
            .chars()
            .map(|c| {
                if c.is_ascii() && !c.is_ascii_control() {
                    c
                } else {
                    '?'
                }
            })
            .collect();
        self.pair(1, t);
        self.num(50, rot.to_degrees());
    }
    fn polyline(&mut self, layer: &str, pts: &[[f64; 2]], closed: bool) {
        self.pair(0, "POLYLINE");
        self.pair(8, layer);
        self.pair(66, 1);
        self.num(10, 0.0);
        self.num(20, 0.0);
        self.num(30, 0.0);
        self.pair(70, if closed { 1 } else { 0 });
        for p in pts {
            self.pair(0, "VERTEX");
            self.pair(8, layer);
            self.num(10, p[0]);
            self.num(20, p[1]);
            self.num(30, 0.0);
        }
        self.pair(0, "SEQEND");
        self.pair(8, layer);
    }
}

pub fn dxf(d: &Diagram, symbols: &[SymbolDef]) -> String {
    let mut o = Out(String::new());
    o.pair(
        999,
        "Lotus diagram. Units: metres, project frame (x east, y north).",
    );
    o.pair(0, "SECTION");
    o.pair(2, "HEADER");
    o.pair(9, "$ACADVER");
    o.pair(1, "AC1009");
    o.pair(9, "$INSUNITS");
    o.pair(70, 6);
    o.pair(0, "ENDSEC");
    // Layers.
    let visible: Vec<&Entity> = d.entities.iter().filter(|e| d.visible(e)).collect();
    let mut layers: Vec<String> = vec![];
    for e in &visible {
        let n = layer_name(d, e.layer());
        if !layers.contains(&n) {
            layers.push(n);
        }
    }
    o.pair(0, "SECTION");
    o.pair(2, "TABLES");
    o.pair(0, "TABLE");
    o.pair(2, "LAYER");
    o.pair(70, layers.len());
    for l in &layers {
        o.pair(0, "LAYER");
        o.pair(2, l);
        o.pair(70, 0);
        o.pair(62, 7);
        o.pair(6, "CONTINUOUS");
    }
    o.pair(0, "ENDTAB");
    o.pair(0, "ENDSEC");
    o.pair(0, "SECTION");
    o.pair(2, "ENTITIES");
    for e in visible {
        let l = layer_name(d, e.layer());
        match e {
            Entity::Line { a, b, .. } => o.line(&l, *a, *b),
            Entity::Polyline { points, closed, .. } => o.polyline(&l, points, *closed),
            Entity::Arc {
                center,
                radius,
                start,
                end,
                ..
            } => o.arc(&l, *center, *radius, *start, *end),
            Entity::Room { geometry, .. } | Entity::Road { geometry, .. } => {
                for s in &geometry.segments {
                    o.line(&l, s.a, s.b);
                }
                for a in &geometry.arcs {
                    o.arc(&l, a.center, a.radius, a.start, a.end);
                }
            }
            Entity::Dimension { a, b, offset, .. } => {
                let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
                let len = (dx * dx + dy * dy).sqrt();
                if len > 0.0 {
                    let n = [-dy / len * offset, dx / len * offset];
                    let (pa, pb) = ([a[0] + n[0], a[1] + n[1]], [b[0] + n[0], b[1] + n[1]]);
                    o.line(&l, *a, pa);
                    o.line(&l, *b, pb);
                    o.line(&l, pa, pb);
                    let mid = [(pa[0] + pb[0]) / 2.0, (pa[1] + pb[1]) / 2.0];
                    o.text(&l, mid, 0.15, dy.atan2(dx), &format!("{len:.3} m"));
                }
            }
            Entity::Text {
                at,
                text,
                height,
                rotation,
                ..
            } => o.text(&l, *at, *height, *rotation, text),
            Entity::Symbol {
                symbol, at, scale, ..
            } => {
                let size = symbols
                    .iter()
                    .find(|s| s.id == symbol)
                    .map_or(0.5, |s| s.size)
                    * scale;
                o.circle(&l, *at, size / 2.0);
                o.text(&l, [at[0] + size / 2.0, at[1]], size / 4.0, 0.0, symbol);
            }
            Entity::Marker { number, at, .. } => {
                o.circle(&l, *at, 0.15);
                o.text(&l, [at[0] + 0.2, at[1]], 0.2, 0.0, &number.to_string());
            }
            Entity::Point { at, label, .. } => {
                o.pair(0, "POINT");
                o.pair(8, &l);
                o.num(10, at[0]);
                o.num(20, at[1]);
                o.num(30, 0.0);
                if !label.is_empty() {
                    o.text(&l, [at[0] + 0.1, at[1] + 0.1], 0.15, 0.0, label);
                }
            }
            Entity::North { at, rotation, .. } => {
                let dir = [-rotation.sin(), rotation.cos()];
                let tip = [at[0] + dir[0], at[1] + dir[1]];
                o.line(&l, *at, tip);
                o.text(&l, tip, 0.25, *rotation, "N");
            }
            Entity::Scalebar { at, length, .. } => {
                o.line(&l, *at, [at[0] + length, at[1]]);
                o.text(&l, [at[0], at[1] + 0.1], 0.15, 0.0, &format!("{length} m"));
            }
            Entity::Legend { .. } | Entity::Underlay { .. } => {}
        }
    }
    o.pair(0, "ENDSEC");
    o.pair(0, "EOF");
    o.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_diagram_writes_in_world_metres_by_layer() {
        let d: Diagram = serde_json::from_value(serde_json::json!({
            "layers": [
                { "id": "l1", "name": "Walls & doors", "visible": true },
                { "id": "l2", "name": "Hidden", "visible": false }
            ],
            "entities": [
                { "kind": "line", "layer": "l1", "a": [0.0, 0.0], "b": [10.0, 0.0] },
                { "kind": "arc", "layer": "l1", "center": [5.0, 5.0], "radius": 2.0,
                  "start": 0.0, "end": std::f64::consts::FRAC_PI_2 },
                { "kind": "dimension", "layer": "l1", "a": [0.0, 0.0], "b": [3.0, 4.0], "offset": 0.5 },
                { "kind": "marker", "layer": "l1", "number": 7, "at": [1.0, 2.0] },
                { "kind": "line", "layer": "l2", "a": [0.0, 0.0], "b": [99.0, 0.0] }
            ]
        }))
        .unwrap();
        let s = dxf(&d, &[]);
        assert!(s.starts_with("999\r\n"));
        assert!(s.contains("2\r\nWalls___doors\r\n"));
        assert!(!s.contains("Hidden") && !s.contains("99.000000"));
        // The 10 m line, exactly.
        assert!(s.contains("LINE\r\n8\r\nWalls___doors\r\n10\r\n0.000000\r\n20\r\n0.000000\r\n30\r\n0.000000\r\n11\r\n10.000000\r\n21\r\n0.000000"));
        // The quarter arc, counter-clockwise from 0° to 90°.
        assert!(s.contains("40\r\n2.000000\r\n50\r\n0.000000\r\n51\r\n90.000000"));
        assert!(s.contains("1\r\n5.000 m\r\n"));
        assert!(s.contains("1\r\n7\r\n"));
        assert!(s.ends_with("0\r\nEOF\r\n"));
    }
}
