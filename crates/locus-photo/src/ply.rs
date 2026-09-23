//! Reading a PLY point cloud (COLMAP's `fused.ply`: binary little-endian or ASCII vertices
//! with x, y, z and optionally red, green, blue; other properties skipped).

#[derive(Debug, Clone, PartialEq)]
pub struct PlyPoints {
    pub xyz: Vec<[f64; 3]>,
    pub rgb: Vec<[u8; 3]>,
}

fn size_of(t: &str) -> Option<usize> {
    Some(match t {
        "char" | "uchar" | "int8" | "uint8" => 1,
        "short" | "ushort" | "int16" | "uint16" => 2,
        "int" | "uint" | "float" | "int32" | "uint32" | "float32" => 4,
        "double" | "float64" => 8,
        _ => return None,
    })
}

fn value(t: &str, b: &[u8]) -> f64 {
    match t {
        "char" | "int8" => b[0] as i8 as f64,
        "uchar" | "uint8" => b[0] as f64,
        "short" | "int16" => i16::from_le_bytes([b[0], b[1]]) as f64,
        "ushort" | "uint16" => u16::from_le_bytes([b[0], b[1]]) as f64,
        "int" | "int32" => i32::from_le_bytes(b[..4].try_into().unwrap()) as f64,
        "uint" | "uint32" => u32::from_le_bytes(b[..4].try_into().unwrap()) as f64,
        "float" | "float32" => f32::from_le_bytes(b[..4].try_into().unwrap()) as f64,
        _ => f64::from_le_bytes(b[..8].try_into().unwrap()),
    }
}

pub fn read(bytes: &[u8]) -> Result<PlyPoints, String> {
    let end = bytes
        .windows(11)
        .position(|w| w == b"end_header\n")
        .ok_or("not a PLY file (no end_header)")?;
    let header = String::from_utf8_lossy(&bytes[..end]);
    let mut lines = header.lines();
    if lines.next().map(str::trim) != Some("ply") {
        return Err("not a PLY file".into());
    }
    let (mut format, mut count, mut props, mut in_vertex) = (String::new(), 0usize, vec![], false);
    for l in lines {
        let f: Vec<&str> = l.split_whitespace().collect();
        match f.as_slice() {
            ["format", fmt, ..] => format = fmt.to_string(),
            ["element", "vertex", n] => {
                count = n.parse().map_err(|_| "bad vertex count")?;
                in_vertex = true;
            }
            ["element", ..] => in_vertex = false,
            ["property", "list", ..] if in_vertex => {
                return Err("list properties on vertices aren't supported".into())
            }
            ["property", t, name] if in_vertex => props.push((t.to_string(), name.to_string())),
            _ => {}
        }
    }
    let find = |n: &str| props.iter().position(|p| p.1 == n);
    let (Some(x), Some(y), Some(z)) = (find("x"), find("y"), find("z")) else {
        return Err("the PLY has no x, y, z".into());
    };
    let colour = (find("red"), find("green"), find("blue"));
    let mut out = PlyPoints {
        xyz: Vec::with_capacity(count),
        rgb: Vec::with_capacity(count),
    };
    let data = &bytes[end + 11..];
    match format.as_str() {
        "binary_little_endian" => {
            let sizes: Vec<usize> = props
                .iter()
                .map(|p| size_of(&p.0).ok_or(format!("unknown type {}", p.0)))
                .collect::<Result<_, _>>()?;
            let offs: Vec<usize> = sizes
                .iter()
                .scan(0, |a, s| {
                    let o = *a;
                    *a += s;
                    Some(o)
                })
                .collect();
            let stride: usize = sizes.iter().sum();
            if data.len() < stride * count {
                return Err("the PLY is shorter than its header says".into());
            }
            for v in 0..count {
                let r = &data[v * stride..];
                let get = |k: usize| value(&props[k].0, &r[offs[k]..]);
                out.xyz.push([get(x), get(y), get(z)]);
                if let (Some(a), Some(b), Some(c)) = colour {
                    out.rgb.push([get(a) as u8, get(b) as u8, get(c) as u8]);
                }
            }
        }
        "ascii" => {
            for l in String::from_utf8_lossy(data).lines().take(count) {
                let f: Vec<f64> = l
                    .split_whitespace()
                    .filter_map(|s| s.parse().ok())
                    .collect();
                if f.len() < props.len() {
                    return Err("a PLY vertex line is short".into());
                }
                out.xyz.push([f[x], f[y], f[z]]);
                if let (Some(a), Some(b), Some(c)) = colour {
                    out.rgb.push([f[a] as u8, f[b] as u8, f[c] as u8]);
                }
            }
        }
        f => return Err(format!("PLY format {f} isn't supported")),
    }
    if out.xyz.len() != count {
        return Err("the PLY has fewer vertices than its header says".into());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colmap_fused_ply_is_read() {
        let mut b = b"ply\nformat binary_little_endian 1.0\nelement vertex 2\nproperty float x\nproperty float y\nproperty float z\nproperty float nx\nproperty float ny\nproperty float nz\nproperty uchar red\nproperty uchar green\nproperty uchar blue\nend_header\n".to_vec();
        for (p, c) in [
            ([1.0f32, 2.0, 3.0], [10u8, 20, 30]),
            ([-1.5, 0.25, 8.0], [200, 100, 0]),
        ] {
            for v in p.iter().chain([0.0f32, 0.0, 1.0].iter()) {
                b.extend(v.to_le_bytes());
            }
            b.extend(c);
        }
        let r = read(&b).unwrap();
        assert_eq!(r.xyz, vec![[1.0, 2.0, 3.0], [-1.5, 0.25, 8.0]]);
        assert_eq!(r.rgb, vec![[10, 20, 30], [200, 100, 0]]);
        let a = read(b"ply\nformat ascii 1.0\nelement vertex 1\nproperty double x\nproperty double y\nproperty double z\nend_header\n0.5 1 2\n").unwrap();
        assert_eq!(a.xyz, vec![[0.5, 1.0, 2.0]]);
        assert!(a.rgb.is_empty());
        assert!(read(&b[..b.len() - 3]).is_err());
    }
}
