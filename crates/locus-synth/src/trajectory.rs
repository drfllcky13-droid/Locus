//! Bullet paths with known ground truth: a straight line through several panels (a vehicle
//! door, a wall, furniture), with the entry and exit defect centres an examiner would pick,
//! picking noise, an optional probe rod, and the panels as a point cloud with the holes in
//! them.
//!
//! Directions: bearing clockwise from project +y, elevation up from horizontal, degrees.

use crate::{add, cross, dot, scale, sub, Point, Rng};
use serde::{Deserialize, Serialize};

type V3 = [f64; 3];

/// One panel the bullet passes through.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PanelSpec {
    pub name: String,
    /// Distance along the path from the muzzle to the entry defect (m).
    pub distance: f64,
    /// Angle between the path and the panel's normal (0 = square on), degrees.
    pub incidence_deg: f64,
    /// Which way the panel leans, as a rotation about the path, degrees.
    pub lean_deg: f64,
    pub thickness: f64,
    /// Panel width and height (m), for the point cloud.
    pub size: [f64; 2],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Options {
    pub seed: u64,
    pub muzzle: V3,
    pub bearing_deg: f64,
    pub elevation_deg: f64,
    pub panels: Vec<PanelSpec>,
    /// 1σ of a picked defect centre within the surface (m).
    pub pick_sigma: f64,
    /// 1σ of scanned points along the surface normal (m).
    pub scan_sigma: f64,
    pub calibre: f64,
    /// Probe rod diameter (m); the rod goes through the last panel.
    pub rod_diameter: f64,
    /// Point spacing on the panels (m).
    pub spacing: f64,
}

impl Default for Options {
    /// A shot through a vehicle door, an interior wall and a wardrobe side, slightly
    /// downward, at oblique angles to each.
    fn default() -> Self {
        let panel = |name: &str, distance, incidence_deg, lean_deg, thickness, size| PanelSpec {
            name: name.into(),
            distance,
            incidence_deg,
            lean_deg,
            thickness,
            size,
        };
        Options {
            seed: 1,
            muzzle: [2.0, 1.0, 1.35],
            bearing_deg: 62.0,
            elevation_deg: -4.0,
            panels: vec![
                panel("vehicle door", 1.8, 35.0, 20.0, 0.001, [0.8, 0.6]),
                panel("interior wall", 4.6, 22.0, 100.0, 0.0125, [1.2, 1.2]),
                panel("wardrobe side", 6.1, 12.0, -60.0, 0.018, [0.6, 1.0]),
            ],
            pick_sigma: 0.002,
            scan_sigma: 0.001,
            calibre: 0.009,
            rod_diameter: 0.006,
            spacing: 0.005,
        }
    }
}

/// The true path and what an examiner would measure on it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Truth {
    pub options: Options,
    /// Unit direction of travel.
    pub direction: V3,
    pub panels: Vec<PanelTruth>,
    pub rod: Rod,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PanelTruth {
    pub name: String,
    /// Front face: a point on it and its unit normal, facing the shooter.
    pub point: V3,
    pub normal: V3,
    pub thickness: f64,
    /// True defect centres on the front (entry) and back (exit) faces.
    pub entry: V3,
    pub exit: V3,
    /// The same, as picked: noise within the surface and along its normal.
    pub entry_picked: V3,
    pub exit_picked: V3,
    /// Angle between the path and the panel's surface (90° = square on), degrees.
    pub impact_deg: f64,
    /// Long and short axes of the elliptical defect (m).
    pub defect_axes: [f64; 2],
}

/// A probe rod pushed through the last panel's holes, as picked at two points 0.5 m apart.
/// A rod can tilt in its hole: at most `max_play_deg`, from the hole's size and depth.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rod {
    pub a: V3,
    pub b: V3,
    pub play_deg: f64,
    pub max_play_deg: f64,
}

fn unit(a: V3) -> V3 {
    scale(a, 1.0 / dot(a, a).sqrt())
}

/// Two unit vectors square to `d` and to each other.
fn basis(d: V3) -> (V3, V3) {
    let t = if d[2].abs() < 0.9 {
        [0.0, 0.0, 1.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    let u = unit(cross(d, t));
    (u, cross(d, u))
}

/// Direction of travel from a bearing (clockwise from +y) and an elevation, degrees.
pub fn direction(bearing_deg: f64, elevation_deg: f64) -> V3 {
    let (b, e) = (bearing_deg.to_radians(), elevation_deg.to_radians());
    [b.sin() * e.cos(), b.cos() * e.cos(), e.sin()]
}

pub fn truth(o: &Options) -> Truth {
    let mut rng = Rng::new(o.seed);
    let d = direction(o.bearing_deg, o.elevation_deg);
    let (u, v) = basis(d);
    let mut panels = vec![];
    for p in &o.panels {
        let (i, l) = (p.incidence_deg.to_radians(), p.lean_deg.to_radians());
        // The normal faces back along the path, tipped by the incidence angle.
        let side = add(scale(u, l.cos()), scale(v, l.sin()));
        let normal = unit(add(scale(d, -i.cos()), scale(side, i.sin())));
        let entry = add(o.muzzle, scale(d, p.distance));
        let exit = add(entry, scale(d, p.thickness / i.cos()));
        let (pu, pv) = basis(normal);
        let pick = |rng: &mut Rng, at: V3| {
            let within = add(
                scale(pu, rng.normal() * o.pick_sigma),
                scale(pv, rng.normal() * o.pick_sigma),
            );
            add(at, add(within, scale(normal, rng.normal() * o.scan_sigma)))
        };
        let entry_picked = pick(&mut rng, entry);
        let exit_picked = pick(&mut rng, exit);
        panels.push(PanelTruth {
            name: p.name.clone(),
            point: entry,
            normal,
            thickness: p.thickness,
            entry,
            exit,
            entry_picked,
            exit_picked,
            impact_deg: 90.0 - p.incidence_deg,
            defect_axes: [o.calibre / i.cos(), o.calibre],
        });
    }
    // The rod: through the last panel's entry, tilted by play within what its hole allows.
    let last = panels.last().expect("at least one panel");
    let depth = last.thickness / (90.0 - last.impact_deg).to_radians().cos();
    let max_play = ((o.calibre - o.rod_diameter).max(0.0) / depth.max(1e-4))
        .atan()
        .to_degrees()
        .min(20.0);
    let play = rng.range(0.0, max_play);
    let turn = rng.range(0.0, std::f64::consts::TAU);
    let axis = add(scale(u, turn.cos()), scale(v, turn.sin()));
    let rod_dir = unit(add(
        scale(d, play.to_radians().cos()),
        scale(axis, play.to_radians().sin()),
    ));
    let noise =
        |rng: &mut Rng| [rng.normal(), rng.normal(), rng.normal()].map(|x| x * o.pick_sigma);
    let a = add(sub(last.entry, scale(rod_dir, 0.25)), noise(&mut rng));
    let b = add(add(last.entry, scale(rod_dir, 0.25)), noise(&mut rng));
    Truth {
        options: o.clone(),
        direction: d,
        panels,
        rod: Rod {
            a,
            b,
            play_deg: play,
            max_play_deg: max_play,
        },
    }
}

/// The panels as scanned points: both faces on a grid, noise along the normal, with the
/// elliptical holes left out.
pub fn points(t: &Truth) -> Vec<Point> {
    let o = &t.options;
    let mut rng = Rng::new(o.seed ^ 0x5eed);
    let mut out = vec![];
    for (p, spec) in t.panels.iter().zip(&o.panels) {
        let (pu, pv) = basis(p.normal);
        // The hole's long axis: the path's direction projected onto the panel.
        let along = sub(t.direction, scale(p.normal, dot(t.direction, p.normal)));
        let long = if dot(along, along) > 1e-12 {
            unit(along)
        } else {
            pu
        };
        let short = cross(p.normal, long);
        let (a, b) = (p.defect_axes[0] / 2.0, p.defect_axes[1] / 2.0);
        let n = [
            (spec.size[0] / o.spacing) as i64,
            (spec.size[1] / o.spacing) as i64,
        ];
        for (face, centre) in [(0.0, p.entry), (-p.thickness, p.exit)] {
            for i in -n[0] / 2..=n[0] / 2 {
                for j in -n[1] / 2..=n[1] / 2 {
                    let off = add(
                        scale(pu, i as f64 * o.spacing),
                        scale(pv, j as f64 * o.spacing),
                    );
                    let q = add(add(p.entry, off), scale(p.normal, face));
                    let r = sub(q, centre);
                    let (x, y) = (dot(r, long) / a, dot(r, short) / b);
                    if x * x + y * y <= 1.0 {
                        continue;
                    }
                    out.push(Point {
                        xyz: add(q, scale(p.normal, rng.normal() * o.scan_sigma)),
                        intensity: 1200,
                        rgb: [170, 172, 176],
                    });
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defects_lie_on_the_path_and_the_panels() {
        let t = truth(&Options::default());
        for p in &t.panels {
            // Entry on the front face, exit on the back face.
            assert!(dot(sub(p.entry, p.point), p.normal).abs() < 1e-12);
            assert!((dot(sub(p.exit, p.point), p.normal) + p.thickness).abs() < 1e-12);
            // Both on the line from the muzzle.
            for q in [p.entry, p.exit] {
                let r = sub(q, t.options.muzzle);
                let off = sub(r, scale(t.direction, dot(r, t.direction)));
                assert!(dot(off, off).sqrt() < 1e-12);
            }
            // The impact angle is the angle between the path and the surface.
            let s = (-dot(t.direction, p.normal)).asin().to_degrees();
            assert!((s - p.impact_deg).abs() < 1e-9);
        }
    }

    #[test]
    fn picks_are_noisy_by_about_the_stated_sigma() {
        let o = Options {
            panels: (0..200)
                .map(|k| PanelSpec {
                    name: format!("p{k}"),
                    distance: 1.0 + k as f64 * 0.1,
                    incidence_deg: 30.0,
                    lean_deg: k as f64,
                    thickness: 0.01,
                    size: [0.1, 0.1],
                })
                .collect(),
            ..Options::default()
        };
        let t = truth(&o);
        let rms = (t
            .panels
            .iter()
            .map(|p| {
                let e = sub(p.entry_picked, p.entry);
                dot(e, e)
            })
            .sum::<f64>()
            / 200.0)
            .sqrt();
        // In-plane 2 mm on two axes plus 1 mm along the normal: sqrt(4 + 4 + 1) = 3 mm.
        assert!((rms - 0.003).abs() < 0.0005, "{rms}");
    }

    #[test]
    fn the_panels_have_holes_where_the_bullet_went() {
        let t = truth(&Options::default());
        let pts = points(&t);
        for p in &t.panels {
            let near = pts
                .iter()
                .filter(|q| {
                    let r = sub(q.xyz, p.entry);
                    dot(r, r).sqrt() < t.options.calibre * 0.45
                })
                .count();
            assert_eq!(near, 0, "{}", p.name);
        }
        assert!(pts.len() > 50_000);
    }
}
