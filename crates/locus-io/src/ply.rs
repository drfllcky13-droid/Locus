//! PLY (ASCII, binary little- and big-endian), streamed.
//!
//! A file with a non-empty `face` element is reported as a mesh; otherwise its
//! vertices are reported as a scan.

use crate::stats::{rescale, stem, Counting, Point, ScanStats, Visitor};
use crate::{parse_err, Error, ProgressFn, Result};
use locus_core::{Contents, MeshInfo, IDENTITY};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Encoding {
    Ascii,
    Le,
    Be,
}

#[derive(Clone, Copy)]
enum Scalar {
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    F32,
    F64,
}

impl Scalar {
    fn parse(s: &str) -> Option<Scalar> {
        Some(match s {
            "char" | "int8" => Scalar::I8,
            "uchar" | "uint8" => Scalar::U8,
            "short" | "int16" => Scalar::I16,
            "ushort" | "uint16" => Scalar::U16,
            "int" | "int32" => Scalar::I32,
            "uint" | "uint32" => Scalar::U32,
            "float" | "float32" => Scalar::F32,
            "double" | "float64" => Scalar::F64,
            _ => return None,
        })
    }

    fn size(self) -> usize {
        match self {
            Scalar::I8 | Scalar::U8 => 1,
            Scalar::I16 | Scalar::U16 => 2,
            Scalar::I32 | Scalar::U32 | Scalar::F32 => 4,
            Scalar::F64 => 8,
        }
    }

    fn decode(self, b: &[u8], enc: Encoding) -> f64 {
        macro_rules! num {
            ($t:ty, $n:expr) => {{
                let a: [u8; $n] = b[..$n].try_into().unwrap();
                (if enc == Encoding::Be {
                    <$t>::from_be_bytes(a)
                } else {
                    <$t>::from_le_bytes(a)
                }) as f64
            }};
        }
        match self {
            Scalar::I8 => b[0] as i8 as f64,
            Scalar::U8 => b[0] as f64,
            Scalar::I16 => num!(i16, 2),
            Scalar::U16 => num!(u16, 2),
            Scalar::I32 => num!(i32, 4),
            Scalar::U32 => num!(u32, 4),
            Scalar::F32 => num!(f32, 4),
            Scalar::F64 => num!(f64, 8),
        }
    }
}

enum Prop {
    Scalar(String, Scalar),
    List(Scalar, Scalar),
}

struct Element {
    name: String,
    count: u64,
    props: Vec<Prop>,
}

const FMT: &str = "PLY";

fn header(r: &mut impl BufRead) -> Result<(Encoding, Vec<Element>)> {
    let mut line = String::new();
    let mut encoding = None;
    let mut elements: Vec<Element> = vec![];
    loop {
        line.clear();
        if r.read_line(&mut line)? == 0 {
            return Err(parse_err(FMT, "header has no end_header line"));
        }
        let words: Vec<&str> = line.split_whitespace().collect();
        match words.as_slice() {
            ["ply"] | ["comment", ..] | ["obj_info", ..] | [] => {}
            ["format", f, _] => {
                encoding = Some(match *f {
                    "ascii" => Encoding::Ascii,
                    "binary_little_endian" => Encoding::Le,
                    "binary_big_endian" => Encoding::Be,
                    _ => return Err(parse_err(FMT, format!("unknown format {f}"))),
                })
            }
            ["element", name, count] => elements.push(Element {
                name: name.to_string(),
                count: count
                    .parse()
                    .map_err(|_| parse_err(FMT, format!("bad element count {count}")))?,
                props: vec![],
            }),
            ["property", "list", c, i, _] => {
                let prop = match (Scalar::parse(c), Scalar::parse(i)) {
                    (Some(c), Some(i)) => Prop::List(c, i),
                    _ => {
                        return Err(parse_err(
                            FMT,
                            format!("bad list property: {}", line.trim()),
                        ))
                    }
                };
                push_prop(&mut elements, prop)?;
            }
            ["property", t, name] => {
                let t = Scalar::parse(t)
                    .ok_or_else(|| parse_err(FMT, format!("unknown property type {t}")))?;
                push_prop(&mut elements, Prop::Scalar(name.to_string(), t))?;
            }
            ["end_header"] => break,
            _ => {
                return Err(parse_err(
                    FMT,
                    format!("unexpected header line: {}", line.trim()),
                ))
            }
        }
    }
    let encoding = encoding.ok_or_else(|| parse_err(FMT, "header has no format line"))?;
    Ok((encoding, elements))
}

fn push_prop(elements: &mut [Element], prop: Prop) -> Result<()> {
    elements
        .last_mut()
        .ok_or_else(|| parse_err(FMT, "property before any element"))?
        .props
        .push(prop);
    Ok(())
}

/// Reads one element's records, handing each record's scalar values (lists skipped) to `f`.
struct Records<'r, R> {
    r: &'r mut R,
    enc: Encoding,
    line: String,
    buf: [u8; 8],
}

impl<R: BufRead> Records<'_, R> {
    fn read(&mut self, el: &Element, values: &mut Vec<f64>) -> Result<()> {
        values.clear();
        if self.enc == Encoding::Ascii {
            self.line.clear();
            if self.r.read_line(&mut self.line)? == 0 {
                return Err(parse_err(
                    FMT,
                    format!("file ends inside element {}", el.name),
                ));
            }
            let mut tokens = self.line.split_whitespace();
            let mut next = || -> Result<f64> {
                tokens
                    .next()
                    .and_then(|t| t.parse().ok())
                    .ok_or_else(|| parse_err(FMT, format!("malformed {} record", el.name)))
            };
            for p in &el.props {
                match p {
                    Prop::Scalar(..) => values.push(next()?),
                    Prop::List(..) => {
                        for _ in 0..next()? as u64 {
                            next()?;
                        }
                    }
                }
            }
        } else {
            for p in &el.props {
                match p {
                    Prop::Scalar(_, t) => values.push(self.scalar(*t)?),
                    Prop::List(c, i) => {
                        for _ in 0..self.scalar(*c)? as u64 {
                            self.scalar(*i)?;
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn scalar(&mut self, t: Scalar) -> Result<f64> {
        let b = &mut self.buf[..t.size()];
        self.r.read_exact(b).map_err(|e| match e.kind() {
            std::io::ErrorKind::UnexpectedEof => parse_err(FMT, "file ends before all records"),
            _ => Error::Io(e),
        })?;
        Ok(t.decode(b, self.enc))
    }
}

/// A colour or intensity channel's position among the scalar values, and its type.
fn channel(vertex: &Element, names: &[&str]) -> Option<(usize, Scalar)> {
    let scalars = || {
        vertex.props.iter().filter_map(|p| match p {
            Prop::Scalar(n, t) => Some((n.as_str(), *t)),
            Prop::List(..) => None,
        })
    };
    names
        .iter()
        .find_map(|want| scalars().position(|(n, _)| n == *want))
        .map(|i| (i, scalars().nth(i).unwrap().1))
}

/// Full-scale value of a scalar type: floats are taken to run from 0 to 1.
fn full_scale(t: Scalar) -> f64 {
    match t {
        Scalar::U8 | Scalar::I8 => 255.0,
        Scalar::U16 | Scalar::I16 => 65535.0,
        Scalar::U32 | Scalar::I32 => u32::MAX as f64,
        Scalar::F32 | Scalar::F64 => 1.0,
    }
}

pub(crate) fn inspect(path: &Path, progress: ProgressFn, mut visit: Visitor) -> Result<Contents> {
    let total = std::fs::metadata(path)?.len();
    let mut r = Counting::new(
        BufReader::with_capacity(1 << 20, File::open(path)?),
        total,
        progress,
    );
    let (enc, elements) = header(&mut r)?;
    let vertex_idx = elements
        .iter()
        .position(|e| e.name == "vertex")
        .ok_or_else(|| parse_err(FMT, "no vertex element"))?;
    let vertex = &elements[vertex_idx];
    let scalar_index = |name: &str| {
        vertex
            .props
            .iter()
            .filter(|p| matches!(p, Prop::Scalar(..)))
            .position(|p| matches!(p, Prop::Scalar(n, _) if n == name))
    };
    let (Some(ix), Some(iy), Some(iz)) = (scalar_index("x"), scalar_index("y"), scalar_index("z"))
    else {
        return Err(parse_err(FMT, "vertex element has no x, y, z properties"));
    };

    let mut records = Records {
        r: &mut r,
        enc,
        line: String::new(),
        buf: [0; 8],
    };
    let mut values = Vec::new();
    for el in &elements[..vertex_idx] {
        for _ in 0..el.count {
            records.read(el, &mut values)?;
        }
    }
    let rgb = [
        channel(vertex, &["red", "r", "diffuse_red"]),
        channel(vertex, &["green", "g", "diffuse_green"]),
        channel(vertex, &["blue", "b", "diffuse_blue"]),
    ];
    let intensity = channel(
        vertex,
        &["intensity", "scalar_intensity", "scalar_Intensity"],
    );
    let mut stats = ScanStats::default();
    for _ in 0..vertex.count {
        records.read(vertex, &mut values)?;
        let p = [values[ix], values[iy], values[iz]];
        if let Some((_, f)) = visit.as_mut() {
            let rgb = match rgb {
                [Some(r), Some(g), Some(b)] => Some(
                    [r, g, b].map(|(i, t)| rescale(values[i], 0.0, full_scale(t), 255.0) as u8),
                ),
                _ => None,
            };
            let intensity =
                intensity.map(|(i, t)| rescale(values[i], 0.0, full_scale(t), 65535.0) as u16);
            f(&Point {
                p,
                rgb,
                intensity,
                index: stats.count,
            });
        }
        stats.point(p);
    }

    let faces = elements
        .iter()
        .find(|e| e.name == "face")
        .map_or(0, |e| e.count);
    let mut c = Contents::new(FMT);
    if faces > 0 {
        c.meshes.push(MeshInfo {
            name: stem(path),
            vertex_count: stats.count,
            face_count: faces,
            bounds: stats.bounds,
        });
    } else {
        let names = |n: &[&str]| {
            n.iter().all(|w| {
                vertex
                    .props
                    .iter()
                    .any(|p| matches!(p, Prop::Scalar(s, _) if s == w))
            })
        };
        let mut attributes = vec![];
        if names(&["red", "green", "blue"]) {
            attributes.push("color".into());
        }
        if names(&["intensity"]) || names(&["scalar_intensity"]) {
            attributes.push("intensity".into());
        }
        if names(&["nx", "ny", "nz"]) {
            attributes.push("normals".into());
        }
        c.scans
            .push(stats.into_scan(stem(path), IDENTITY, attributes));
    }
    Ok(c)
}
