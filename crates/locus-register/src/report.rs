//! The registration report's contents, recomputed from the stored links and poses rather than
//! copied from the solver, so the report can be checked against the adjustment: per-link
//! error, overlap and test, target residuals, and overall figures. Rendering is elsewhere.

use crate::posegraph::{residual, Link, LinkKind, LinkReport, LinkStatus};
use nalgebra::{Isometry3, Vector3};
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReportLink {
    pub index: usize,
    pub kind: LinkKind,
    pub a: usize,
    pub b: Option<usize>,
    pub status: LinkStatus,
    pub pairs: usize,
    /// RMS and largest pair residual at the final poses (m).
    pub rms: f64,
    pub max: f64,
    pub chi2_per_dof: f64,
    pub limit_per_dof: f64,
    /// ICP overlap (cloud links).
    pub overlap: Option<f64>,
    pub forced: bool,
    pub shape_only: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReportScan {
    pub index: usize,
    /// Registered position (m) and heading (degrees from +x, anticlockwise seen from above).
    pub position: [f64; 3],
    pub heading_deg: f64,
    pub verified: bool,
}

/// Residuals of every target and control pair in links used by the solution (m).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TargetResiduals {
    pub count: usize,
    pub mean: Option<f64>,
    pub max: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RegistrationReport {
    pub scans: Vec<ReportScan>,
    pub links: Vec<ReportLink>,
    pub targets: TargetResiduals,
    pub ok: usize,
    pub flagged: usize,
    pub untested: usize,
    pub shape_only: usize,
    pub unverified: usize,
}

/// Build the report from the links, the final poses and the solver's per-link tests. Pair
/// residuals are recomputed here from the poses.
pub fn build(
    links: &[Link],
    poses: &[Isometry3<f64>],
    tests: &[LinkReport],
    overlap: &[Option<f64>],
    verified: &[bool],
) -> RegistrationReport {
    let mut target_res = vec![];
    let report_links: Vec<ReportLink> = links
        .iter()
        .enumerate()
        .map(|(i, l)| {
            let res: Vec<f64> = l
                .pairs
                .iter()
                .map(|p| residual(l, p, poses).0.norm())
                .collect();
            let t = &tests[i];
            if l.kind != LinkKind::Cloud && t.status != LinkStatus::Flagged {
                target_res.extend(&res);
            }
            ReportLink {
                index: i,
                kind: l.kind,
                a: l.a,
                b: l.b,
                status: t.status.clone(),
                pairs: res.len(),
                rms: (res.iter().map(|r| r * r).sum::<f64>() / res.len().max(1) as f64).sqrt(),
                max: res.iter().copied().fold(0.0, f64::max),
                chi2_per_dof: t.chi2_per_dof,
                limit_per_dof: t.limit_per_dof,
                overlap: overlap.get(i).copied().flatten(),
                forced: l.forced,
                shape_only: l.shape_only,
            }
        })
        .collect();
    let scans = poses
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let x = p.rotation * Vector3::x();
            ReportScan {
                index: i,
                position: p.translation.vector.into(),
                heading_deg: x.y.atan2(x.x).to_degrees(),
                verified: verified.get(i).copied().unwrap_or(false),
            }
        })
        .collect::<Vec<_>>();
    let count = |s: LinkStatus| report_links.iter().filter(|l| l.status == s).count();
    RegistrationReport {
        targets: TargetResiduals {
            count: target_res.len(),
            mean: (!target_res.is_empty())
                .then(|| target_res.iter().sum::<f64>() / target_res.len() as f64),
            max: target_res.iter().copied().reduce(f64::max),
        },
        ok: count(LinkStatus::Ok),
        flagged: count(LinkStatus::Flagged),
        untested: count(LinkStatus::Untested),
        shape_only: links.iter().filter(|l| l.shape_only).count(),
        unverified: scans.iter().filter(|s| !s.verified).count(),
        scans,
        links: report_links,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::posegraph::{solve, Pair};
    use nalgebra::{Matrix3, Point3, Translation3, UnitQuaternion};

    #[test]
    fn report_numbers_match_the_adjustment() {
        // Three scans, target links with small disagreements, one bad cloud link.
        let truth = [
            Isometry3::identity(),
            Isometry3::from_parts(
                Translation3::new(8.0, 1.0, 0.0),
                UnitQuaternion::from_euler_angles(0.0, 0.0, 0.6),
            ),
            Isometry3::from_parts(
                Translation3::new(3.0, 9.0, 0.1),
                UnitQuaternion::from_euler_angles(0.0, 0.0, -1.1),
            ),
        ];
        let world = [
            [1.0, 2.0, 1.2],
            [6.0, -1.0, 1.5],
            [4.0, 6.0, 1.1],
            [9.0, 5.0, 1.8],
            [-2.0, 4.0, 1.3],
        ];
        let cov = Matrix3::identity() * (0.0005f64.powi(2) / 3.0);
        let mut k = 0u32;
        let mut wobble = || {
            k += 1;
            (k as f64 * 1.37).sin() * 0.0004
        };
        let mut links = vec![];
        for (a, b) in [(0, 1), (1, 2), (0, 2)] {
            let pairs = world
                .iter()
                .map(|w| {
                    let pa = (truth[a].inverse() * Point3::from(*w)).coords
                        + Vector3::new(wobble(), 0.0, 0.0);
                    let pb = (truth[b].inverse() * Point3::from(*w)).coords
                        + Vector3::new(0.0, wobble(), 0.0);
                    Pair {
                        pa: pa.into(),
                        cov_a: cov,
                        pb: pb.into(),
                        cov_b: cov,
                    }
                })
                .collect();
            links.push(Link::targets(a, b, pairs));
        }
        let wrong = Isometry3::translation(0.03, 0.0, 0.0) * truth[1].inverse() * truth[2];
        links.push(Link::cloud(
            1,
            2,
            &wrong,
            &[[0.0, 0.0, 0.0], [5.0, 0.0, 0.0], [0.0, 5.0, 2.0]],
            0.002,
        ));

        let sol = solve(&truth, &links).unwrap();
        let verified = crate::pipeline::verified(3, &links, &sol);
        let overlap = [None, None, None, Some(0.7)];
        let r = build(&links, &sol.poses, &sol.links, &overlap, &verified);

        // Per link: the report's residuals are the adjustment's, recomputed independently.
        for (l, t) in r.links.iter().zip(&sol.links) {
            assert!((l.rms - t.rms).abs() < 1e-12, "rms {} vs {}", l.rms, t.rms);
            assert!((l.max - t.max).abs() < 1e-12);
            assert_eq!(l.status, t.status);
            assert_eq!(l.chi2_per_dof, t.chi2_per_dof);
        }
        assert_eq!(r.links[3].status, LinkStatus::Flagged);
        assert_eq!(r.links[3].overlap, Some(0.7));
        assert_eq!((r.ok, r.flagged, r.untested), (3, 1, 0));
        // Target residuals: the 15 pairs of the three target links.
        assert_eq!(r.targets.count, 15);
        let all: Vec<f64> = links[..3]
            .iter()
            .flat_map(|l| {
                l.pairs
                    .iter()
                    .map(|p| residual(l, p, &sol.poses).0.norm())
                    .collect::<Vec<_>>()
            })
            .collect();
        assert!((r.targets.mean.unwrap() - all.iter().sum::<f64>() / 15.0).abs() < 1e-15);
        assert_eq!(r.targets.max, all.iter().copied().reduce(f64::max));
        // Scans: scan 1's heading is its 0.6 rad yaw.
        assert!((r.scans[1].heading_deg - 0.6f64.to_degrees()).abs() < 0.01);
        assert_eq!(r.unverified, 0);
    }
}
