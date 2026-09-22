//! OBJ and glTF/GLB meshes.
//!
//! Only the named file becomes evidence. Material libraries, textures and external
//! buffers it references are listed as warnings so the examiner can import them too.

use crate::stats::stem;
use crate::{parse_err, Result};
use locus_core::{Bounds, Contents, LinearUnit, MeshInfo};
use std::path::Path;

pub(crate) fn inspect_obj(path: &Path) -> Result<Contents> {
    // ponytail: tobj reads the whole file into memory; fine for meshes, stream it if
    // multi-gigabyte OBJ exports show up.
    let (models, materials) =
        tobj::load_obj(path, &tobj::LoadOptions::default()).map_err(|e| parse_err("OBJ", e))?;
    let mut c = Contents::new("OBJ");
    for m in &models {
        let mut bounds = None;
        for p in m.mesh.positions.as_chunks::<3>().0 {
            Bounds::grow(&mut bounds, *p);
        }
        let faces = if m.mesh.face_arities.is_empty() {
            m.mesh.indices.len() / 3
        } else {
            m.mesh.face_arities.len()
        };
        c.meshes.push(MeshInfo {
            name: if m.name.is_empty() {
                stem(path)
            } else {
                m.name.clone()
            },
            vertex_count: (m.mesh.positions.len() / 3) as u64,
            face_count: faces as u64,
            bounds,
        });
    }
    match materials {
        Ok(mats) if !mats.is_empty() => {
            let mut files: Vec<String> = mats
                .iter()
                .flat_map(|m| {
                    [
                        &m.ambient_texture,
                        &m.diffuse_texture,
                        &m.specular_texture,
                        &m.normal_texture,
                        &m.shininess_texture,
                        &m.dissolve_texture,
                    ]
                })
                .flatten()
                .cloned()
                .collect();
            files.sort();
            files.dedup();
            let mut w =
                "materials come from a separate .mtl file, which is not part of this evidence item"
                    .to_string();
            if !files.is_empty() {
                w += &format!("; it references {}", files.join(", "));
            }
            c.warnings.push(w);
        }
        Ok(_) => {}
        Err(e) => c
            .warnings
            .push(format!("material library could not be read: {e}")),
    }
    if c.meshes.is_empty() {
        return Err(parse_err("OBJ", "no geometry found"));
    }
    Ok(c)
}

pub(crate) fn inspect_gltf(path: &Path) -> Result<Contents> {
    let g = gltf::Gltf::open(path).map_err(|e| parse_err("glTF", e))?;
    let mut c = Contents::new("glTF");
    c.declared_unit = Some(LinearUnit::Meter); // glTF 2.0: meters, +Y up
    c.y_up = true;
    for mesh in g.meshes() {
        let (mut vertices, mut faces, mut bounds) = (0u64, 0u64, None);
        for prim in mesh.primitives() {
            let Some(pos) = prim.get(&gltf::Semantic::Positions) else {
                continue;
            };
            let n = pos.count() as u64;
            vertices += n;
            for corner in [pos.min(), pos.max()].into_iter().flatten() {
                if let Some(v) = corner.as_array().filter(|a| a.len() == 3) {
                    let f = |i: usize| v[i].as_f64().unwrap_or(f64::NAN);
                    Bounds::grow(&mut bounds, [f(0), f(1), f(2)]);
                }
            }
            let idx = prim.indices().map_or(n, |a| a.count() as u64);
            faces += match prim.mode() {
                gltf::mesh::Mode::Triangles => idx / 3,
                gltf::mesh::Mode::TriangleStrip | gltf::mesh::Mode::TriangleFan => {
                    idx.saturating_sub(2)
                }
                _ => 0,
            };
        }
        c.meshes.push(MeshInfo {
            name: mesh
                .name()
                .map_or_else(|| format!("Mesh {}", mesh.index() + 1), str::to_string),
            vertex_count: vertices,
            face_count: faces,
            bounds,
        });
    }
    let mut external: Vec<String> = g
        .buffers()
        .filter_map(|b| match b.source() {
            gltf::buffer::Source::Uri(u) if !u.starts_with("data:") => Some(u.to_string()),
            _ => None,
        })
        .chain(g.images().filter_map(|i| match i.source() {
            gltf::image::Source::Uri { uri, .. } if !uri.starts_with("data:") => {
                Some(uri.to_string())
            }
            _ => None,
        }))
        .collect();
    external.sort();
    external.dedup();
    if !external.is_empty() {
        c.warnings.push(format!(
            "references external files that are not part of this evidence item: {}. Import a .glb, which contains them, if you can.",
            external.join(", ")
        ));
    }
    if c.meshes.is_empty() {
        return Err(parse_err("glTF", "no meshes found"));
    }
    Ok(c)
}
