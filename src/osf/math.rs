//! Pose maths of the tracker: Rodrigues rotations, a Levenberg-Marquardt PnP solver and
//! OpenCV-compatible Euler angles.
//!
//! OpenSeeFace feeds landmarks to `cv2.solvePnP` as (row, column) and starts from a zero
//! pose, which makes OpenCV settle in the *mirrored* solution of the PnP problem (face
//! "behind" the camera, Euler angles around x≈180°, z≈90°, negative z translation).
//! Everything downstream (and every existing calibration) relies on that convention, so
//! `solve_pnp` is started in the same basin and returns the same local minimum.

pub type V3 = [f64; 3];
pub type M3 = [[f64; 3]; 3];

pub fn mat_mul(a: &M3, b: &M3) -> M3 {
    let mut r = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            r[i][j] = a[i][0] * b[0][j] + a[i][1] * b[1][j] + a[i][2] * b[2][j];
        }
    }
    r
}

pub fn mat_vec(m: &M3, v: V3) -> V3 {
    [m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2], m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2], m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2]]
}

pub fn transpose(m: &M3) -> M3 {
    [[m[0][0], m[1][0], m[2][0]], [m[0][1], m[1][1], m[2][1]], [m[0][2], m[1][2], m[2][2]]]
}

/// Rotation vector → matrix.
pub fn rodrigues(r: V3) -> M3 {
    let th = (r[0] * r[0] + r[1] * r[1] + r[2] * r[2]).sqrt();
    if th < 1e-12 {
        return [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    }
    let (x, y, z) = (r[0] / th, r[1] / th, r[2] / th);
    let (s, c) = th.sin_cos();
    let c1 = 1.0 - c;
    [
        [c + x * x * c1, x * y * c1 - z * s, x * z * c1 + y * s],
        [y * x * c1 + z * s, c + y * y * c1, y * z * c1 - x * s],
        [z * x * c1 - y * s, z * y * c1 + x * s, c + z * z * c1],
    ]
}

/// Rotation matrix → rotation vector.
pub fn rodrigues_inv(m: &M3) -> V3 {
    let tr = m[0][0] + m[1][1] + m[2][2];
    let c = ((tr - 1.0) * 0.5).clamp(-1.0, 1.0);
    let th = c.acos();
    let v = [m[2][1] - m[1][2], m[0][2] - m[2][0], m[1][0] - m[0][1]];
    let s = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt() * 0.5;
    if s < 1e-9 {
        if c > 0.0 {
            return [0.0; 3];
        }
        // Angle of π: axis from the diagonal.
        let ax = [((m[0][0] + 1.0) * 0.5).max(0.0).sqrt(), ((m[1][1] + 1.0) * 0.5).max(0.0).sqrt(), ((m[2][2] + 1.0) * 0.5).max(0.0).sqrt()];
        let mut ax = ax;
        if m[0][1] < 0.0 {
            ax[1] = -ax[1];
        }
        if m[0][2] < 0.0 {
            ax[2] = -ax[2];
        }
        return [ax[0] * th, ax[1] * th, ax[2] * th];
    }
    let k = th / (2.0 * s);
    [v[0] * k, v[1] * k, v[2] * k]
}

/// Pinhole camera of the tracker: focal length = image width, principal point = centre.
#[derive(Clone, Copy, Debug)]
pub struct Camera {
    pub f: f64,
    pub cx: f64,
    pub cy: f64,
}

impl Camera {
    pub fn new(width: f64, height: f64) -> Camera {
        Camera { f: width, cx: width / 2.0, cy: height / 2.0 }
    }

    /// camera · p (homogeneous image coordinates scaled by depth).
    pub fn apply(&self, p: V3) -> V3 {
        [self.f * p[0] + self.cx * p[2], self.f * p[1] + self.cy * p[2], p[2]]
    }

    /// camera⁻¹ · p.
    pub fn unapply(&self, p: V3) -> V3 {
        [(p[0] - self.cx * p[2]) / self.f, (p[1] - self.cy * p[2]) / self.f, p[2]]
    }
}

fn project(cam: &Camera, r: &M3, t: V3, p: V3) -> [f64; 2] {
    let q = mat_vec(r, p);
    let (x, y, z) = (q[0] + t[0], q[1] + t[1], q[2] + t[2]);
    // OpenCV: a zero depth is treated as 1.
    let iz = if z != 0.0 { 1.0 / z } else { 1.0 };
    [cam.f * x * iz + cam.cx, cam.f * y * iz + cam.cy]
}

fn residuals(cam: &Camera, pose: &[f64; 6], obj: &[V3], img: &[[f64; 2]], out: &mut [f64]) -> f64 {
    let r = rodrigues([pose[0], pose[1], pose[2]]);
    let t = [pose[3], pose[4], pose[5]];
    let mut sse = 0.0;
    for (i, (p, m)) in obj.iter().zip(img).enumerate() {
        let q = project(cam, &r, t, *p);
        out[i * 2] = q[0] - m[0];
        out[i * 2 + 1] = q[1] - m[1];
        sse += out[i * 2] * out[i * 2] + out[i * 2 + 1] * out[i * 2 + 1];
    }
    sse
}

/// Solves a 6x6 linear system (Gaussian elimination with partial pivoting).
fn solve6(mut a: [[f64; 6]; 6], mut b: [f64; 6]) -> Option<[f64; 6]> {
    for c in 0..6 {
        let p = (c..6).max_by(|&i, &j| a[i][c].abs().partial_cmp(&a[j][c].abs()).unwrap())?;
        if a[p][c].abs() < 1e-14 {
            return None;
        }
        a.swap(c, p);
        b.swap(c, p);
        for r in c + 1..6 {
            let k = a[r][c] / a[c][c];
            for j in c..6 {
                a[r][j] -= k * a[c][j];
            }
            b[r] -= k * b[c];
        }
    }
    let mut x = [0.0; 6];
    for c in (0..6).rev() {
        let mut s = b[c];
        for j in c + 1..6 {
            s -= a[c][j] * x[j];
        }
        x[c] = s / a[c][c];
    }
    Some(x)
}

/// Canonical starting pose (mirrored basin, see the module docs) for image points given
/// as (row, column): depth from the apparent size of the point set.
pub fn initial_pose(cam: &Camera, obj: &[V3], img: &[[f64; 2]]) -> (V3, V3) {
    let r0: M3 = [[0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, -1.0]];
    let ext = |v: &dyn Fn(usize) -> f64, n: usize| {
        let (mut lo, mut hi) = (f64::MAX, f64::MIN);
        for i in 0..n {
            lo = lo.min(v(i));
            hi = hi.max(v(i));
        }
        (lo, hi)
    };
    let n = obj.len().min(img.len());
    // Image rows follow model y, columns follow model x.
    let (oy0, oy1) = ext(&|i| obj[i][1], n);
    let (ox0, ox1) = ext(&|i| obj[i][0], n);
    let (u0, u1) = ext(&|i| img[i][0], n);
    let (v0, v1) = ext(&|i| img[i][1], n);
    let model = ((oy1 - oy0).powi(2) + (ox1 - ox0).powi(2)).sqrt().max(1e-6);
    let pixels = ((u1 - u0).powi(2) + (v1 - v0).powi(2)).sqrt().max(1.0);
    let z = -(cam.f * model / pixels);
    let (uc, vc) = ((u0 + u1) * 0.5, (v0 + v1) * 0.5);
    (rodrigues_inv(&r0), [(uc - cam.cx) * z / cam.f, (vc - cam.cy) * z / cam.f, z])
}

/// Iterative PnP (Levenberg-Marquardt on rotation vector + translation) from a starting
/// pose, like `cv2.solvePnP(..., useExtrinsicGuess=True, flags=SOLVEPNP_ITERATIVE)`.
/// Returns (rvec, tvec, sum of squared reprojection errors).
pub fn solve_pnp(cam: &Camera, obj: &[V3], img: &[[f64; 2]], rvec: V3, tvec: V3) -> Option<(V3, V3, f64)> {
    let n = obj.len().min(img.len());
    if n < 4 {
        return None;
    }
    let (obj, img) = (&obj[..n], &img[..n]);
    let mut pose = [rvec[0], rvec[1], rvec[2], tvec[0], tvec[1], tvec[2]];
    let mut res = vec![0.0; n * 2];
    let mut res2 = vec![0.0; n * 2];
    let mut jac = vec![[0.0f64; 6]; n * 2];
    let mut err = residuals(cam, &pose, obj, img, &mut res);
    let mut lambda = 1e-3;
    for _ in 0..200 {
        // Numerical Jacobian (central differences).
        for k in 0..6 {
            let h = 1e-6 * pose[k].abs().max(1.0);
            let (mut a, mut b) = (pose, pose);
            a[k] += h;
            b[k] -= h;
            let mut ra = vec![0.0; n * 2];
            residuals(cam, &a, obj, img, &mut ra);
            residuals(cam, &b, obj, img, &mut res2);
            for i in 0..n * 2 {
                jac[i][k] = (ra[i] - res2[i]) / (2.0 * h);
            }
        }
        let mut jtj = [[0.0; 6]; 6];
        let mut jtr = [0.0; 6];
        for i in 0..n * 2 {
            for a in 0..6 {
                jtr[a] += jac[i][a] * res[i];
                for b in 0..6 {
                    jtj[a][b] += jac[i][a] * jac[i][b];
                }
            }
        }
        let mut improved = false;
        for _ in 0..12 {
            let mut a = jtj;
            for (d, row) in a.iter_mut().enumerate() {
                row[d] += lambda * jtj[d][d].max(1e-12);
            }
            let Some(step) = solve6(a, [-jtr[0], -jtr[1], -jtr[2], -jtr[3], -jtr[4], -jtr[5]]) else {
                lambda *= 10.0;
                continue;
            };
            let mut cand = pose;
            for k in 0..6 {
                cand[k] += step[k];
            }
            let e = residuals(cam, &cand, obj, img, &mut res2);
            if e.is_finite() && e < err {
                let done = (err - e) < 1e-12 * err.max(1e-12) || step.iter().all(|s| s.abs() < 1e-10);
                pose = cand;
                err = e;
                res.copy_from_slice(&res2);
                lambda = (lambda * 0.1).max(1e-12);
                improved = true;
                if done {
                    return finish(pose, err);
                }
                break;
            }
            lambda *= 10.0;
        }
        if !improved {
            break;
        }
    }
    finish(pose, err)
}

fn finish(pose: [f64; 6], err: f64) -> Option<(V3, V3, f64)> {
    if !pose.iter().all(|v| v.is_finite()) || !err.is_finite() {
        return None;
    }
    // Keep the rotation vector's angle within [0, π] like OpenCV does.
    let r = rodrigues_inv(&rodrigues([pose[0], pose[1], pose[2]]));
    Some((r, [pose[3], pose[4], pose[5]], err))
}

/// Euler angles in degrees exactly as `cv2.RQDecomp3x3(m)[0]` returns them.
pub fn rq_euler(m: &M3) -> V3 {
    let norm = |c: f64, s: f64| {
        let z = 1.0 / (c * c + s * s + f64::EPSILON).sqrt();
        (c * z, s * z)
    };
    // Givens rotation for the x axis.
    let (c, s) = norm(m[2][2], m[2][1]);
    let mut qx: M3 = [[1.0, 0.0, 0.0], [0.0, c, s], [0.0, -s, c]];
    let r = mat_mul(m, &qx);
    // y axis.
    let (c, s) = norm(r[2][2], -r[2][0]);
    let mut qy: M3 = [[c, 0.0, -s], [0.0, 1.0, 0.0], [s, 0.0, c]];
    let m2 = mat_mul(&r, &qy);
    // z axis.
    let (c, s) = norm(m2[1][1], m2[1][0]);
    let mut qz: M3 = [[c, s, 0.0], [-s, c, 0.0], [0.0, 0.0, 1.0]];
    let r = mat_mul(&m2, &qz);
    // Resolve the ambiguity: the first two diagonal entries of R must be positive.
    if r[0][0] < 0.0 {
        if r[1][1] < 0.0 {
            qz[0][0] *= -1.0;
            qz[0][1] *= -1.0;
            qz[1][0] *= -1.0;
            qz[1][1] *= -1.0;
        } else {
            qz = transpose(&qz);
            qy[0][0] *= -1.0;
            qy[0][2] *= -1.0;
            qy[2][0] *= -1.0;
            qy[2][2] *= -1.0;
        }
    } else if r[1][1] < 0.0 {
        qz = transpose(&qz);
        qy = transpose(&qy);
        qx[1][1] *= -1.0;
        qx[1][2] *= -1.0;
        qx[2][1] *= -1.0;
        qx[2][2] *= -1.0;
    }
    let ang = |c: f64, s: f64| c.clamp(-1.0, 1.0).acos() * if s >= 0.0 { 1.0 } else { -1.0 } * 180.0 / std::f64::consts::PI;
    [ang(qx[1][1], qx[1][2]), ang(qy[0][0], qy[2][0]), ang(qz[0][0], qz[0][1])]
}

/// Rotation matrix → quaternion (x, y, z, w) the way the tracker reports it.
pub fn quaternion(m: &M3) -> [f64; 4] {
    let (t, q);
    if m[2][2] < 0.0 {
        if m[0][0] > m[1][1] {
            t = 1.0 + m[0][0] - m[1][1] - m[2][2];
            q = [t, m[0][1] + m[1][0], m[2][0] + m[0][2], m[1][2] - m[2][1]];
        } else {
            t = 1.0 - m[0][0] + m[1][1] - m[2][2];
            q = [m[0][1] + m[1][0], t, m[1][2] + m[2][1], m[2][0] - m[0][2]];
        }
    } else if m[0][0] < -m[1][1] {
        t = 1.0 - m[0][0] - m[1][1] + m[2][2];
        q = [m[2][0] + m[0][2], m[1][2] + m[2][1], t, m[0][1] - m[1][0]];
    } else {
        t = 1.0 + m[0][0] + m[1][1] + m[2][2];
        q = [m[1][2] - m[2][1], m[2][0] - m[0][2], m[0][1] - m[1][0], t];
    }
    let k = 0.5 / t.sqrt();
    [q[0] * k, q[1] * k, q[2] * k, q[3] * k]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::osf::face3d::FACE_3D;

    fn m3(v: &serde_json::Value) -> M3 {
        let mut m = [[0.0; 3]; 3];
        for i in 0..3 {
            for j in 0..3 {
                m[i][j] = v[i][j].as_f64().unwrap();
            }
        }
        m
    }

    /// Reference values generated with `cv2.RQDecomp3x3` on random rotations.
    #[test]
    fn euler_matches_opencv() {
        let cases: serde_json::Value = serde_json::from_str(include_str!("../../tests/data/osf_rq.json")).unwrap();
        for c in cases.as_array().unwrap() {
            let e = rq_euler(&m3(&c["R"]));
            for k in 0..3 {
                let want = c["euler"][k].as_f64().unwrap();
                let d = (e[k] - want).abs();
                assert!(d < 1e-6 || (d - 360.0).abs() < 1e-6, "euler {k}: {} vs {want}", e[k]);
            }
        }
    }

    /// Reference poses from `cv2.solvePnP` started in the tracker's (mirrored) basin.
    #[test]
    fn pnp_matches_opencv() {
        let cases: serde_json::Value = serde_json::from_str(include_str!("../../tests/data/osf_pnp.json")).unwrap();
        let cam = Camera::new(640.0, 480.0);
        let contour = [0usize, 1, 8, 15, 16, 27, 28, 29, 30, 31, 32, 33, 34, 35];
        let obj: Vec<V3> = contour.iter().map(|&i| FACE_3D[i]).collect();
        for c in cases.as_array().unwrap() {
            let img: Vec<[f64; 2]> = c["uv"].as_array().unwrap().iter().map(|p| [p[0].as_f64().unwrap(), p[1].as_f64().unwrap()]).collect();
            let (r0, t0) = initial_pose(&cam, &obj, &img);
            let (r, t, _) = solve_pnp(&cam, &obj, &img, r0, t0).unwrap();
            let e = rq_euler(&rodrigues(r));
            for k in 0..3 {
                let wt = c["tvec"][k].as_f64().unwrap();
                assert!((t[k] - wt).abs() < 5e-3, "t[{k}] {} vs {wt}", t[k]);
                let we = c["euler"][k].as_f64().unwrap();
                let d = (e[k] - we).abs();
                assert!(d < 0.05 || (d - 360.0).abs() < 0.05, "euler[{k}] {} vs {we}", e[k]);
            }
        }
    }

    #[test]
    fn rodrigues_roundtrip() {
        for r in [[0.1, -0.2, 0.3], [2.0, 1.0, -1.5], [0.0, 0.0, 0.0], [std::f64::consts::PI, 0.0, 0.0]] {
            let m = rodrigues(r);
            let m2 = rodrigues(rodrigues_inv(&m));
            for i in 0..3 {
                for j in 0..3 {
                    assert!((m[i][j] - m2[i][j]).abs() < 1e-9);
                }
            }
        }
    }
}
