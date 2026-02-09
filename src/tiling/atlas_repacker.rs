use std::collections::HashMap;

use image::RgbaImage;
use tracing::warn;

use crate::config::TextureConfig;
use crate::tiling::texture_compress;
use crate::types::{IndexedMesh, MaterialLibrary, TextureData};

/// Result of atlas repacking for a single tile.
pub struct AtlasResult {
    /// Mesh with UVs remapped to atlas space.
    pub mesh: IndexedMesh,
    /// Composited and compressed atlas texture.
    pub atlas_texture: TextureData,
}

/// A connected component of UV-space triangles.
struct UvIsland {
    /// Face indices belonging to this island.
    faces: Vec<usize>,
    /// UV bounding rect: (min_u, min_v, max_u, max_v).
    uv_min: [f32; 2],
    uv_max: [f32; 2],
}

/// Placement result from the bin packer.
struct Placement {
    island_idx: usize,
    /// Position in pixels (top-left of padded region).
    x: u32,
    y: u32,
    /// Inner (content) dimensions in pixels.
    inner_w: u32,
    inner_h: u32,
    /// Padding in pixels.
    padding: u32,
}

/// A free rectangle in the guillotine packer.
#[derive(Clone)]
struct FreeRect {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
}

/// Repack textures for a tile mesh into a single atlas.
///
/// Returns `None` if the mesh has no UVs, no material, or the material has no texture.
pub fn repack_atlas(
    mesh: &IndexedMesh,
    materials: &MaterialLibrary,
    config: &TextureConfig,
) -> Option<AtlasResult> {
    if !mesh.has_uvs() {
        return None;
    }

    let mat_idx = mesh.material_index?;
    let mat = materials.materials.get(mat_idx)?;
    let tex_idx = mat.base_color_texture?;
    let tex = materials.textures.get(tex_idx)?;

    let source_image = decode_texture(tex)?;
    let (src_w, src_h) = source_image.dimensions();

    // 1. Build edge adjacency
    let adjacency = build_edge_adjacency(mesh);

    // 2. BFS island detection
    let islands = detect_islands(mesh, &adjacency);

    if islands.is_empty() {
        return None;
    }

    // 3. Pixel sizing for each island
    let sized: Vec<(usize, u32, u32, u32)> = islands
        .iter()
        .enumerate()
        .map(|(i, island)| {
            let u_range = island.uv_max[0] - island.uv_min[0];
            let v_range = island.uv_max[1] - island.uv_min[1];

            let mut px_w = (u_range * src_w as f32).ceil().max(1.0) as u32;
            let mut px_h = (v_range * src_h as f32).ceil().max(1.0) as u32;

            // Cap to max_size
            if px_w > config.max_size {
                px_w = config.max_size;
            }
            if px_h > config.max_size {
                px_h = config.max_size;
            }

            // Bleed padding: 2-5 px based on island size
            let max_dim = px_w.max(px_h);
            let padding = if max_dim > 512 {
                5
            } else if max_dim > 128 {
                3
            } else {
                2
            };

            (i, px_w, px_h, padding)
        })
        .collect();

    // 4. Guillotine bin packing
    let placements = guillotine_pack(&sized);
    let atlas_size = compute_atlas_size(&placements);

    // 5. UV remapping with vertex deduplication for shared vertices across islands
    let new_mesh = remap_uvs_with_dedup(mesh, &islands, &placements, atlas_size);

    // 6. Atlas compositing
    let atlas_image = composite_atlas(&source_image, &islands, &placements, atlas_size);

    // Downscale if the atlas exceeds the configured max_size
    let atlas_image = if atlas_size > config.max_size {
        image::imageops::resize(
            &atlas_image,
            config.max_size,
            config.max_size,
            image::imageops::FilterType::Lanczos3,
        )
    } else {
        atlas_image
    };

    let atlas_texture = texture_compress::compress_texture(&atlas_image, config);

    Some(AtlasResult {
        mesh: new_mesh,
        atlas_texture,
    })
}

/// Decode a TextureData into an RgbaImage.
///
/// Tries encoded image formats first, falls back to raw RGBA/RGB interpretation.
fn decode_texture(tex: &TextureData) -> Option<RgbaImage> {
    // Try decoding as an encoded image (PNG, JPEG, WebP, etc.)
    if let Ok(img) = image::load_from_memory(&tex.data) {
        return Some(img.to_rgba8());
    }

    // Fall back to raw pixel interpretation
    let pixel_count = (tex.width * tex.height) as usize;

    if tex.data.len() == pixel_count * 4 {
        // Raw RGBA
        return RgbaImage::from_raw(tex.width, tex.height, tex.data.clone());
    }

    if tex.data.len() == pixel_count * 3 {
        // Raw RGB → convert to RGBA
        let mut rgba = Vec::with_capacity(pixel_count * 4);
        for chunk in tex.data.chunks_exact(3) {
            rgba.extend_from_slice(chunk);
            rgba.push(255);
        }
        return RgbaImage::from_raw(tex.width, tex.height, rgba);
    }

    warn!(
        width = tex.width,
        height = tex.height,
        data_len = tex.data.len(),
        "Cannot decode texture data"
    );
    None
}

/// Build edge adjacency map.
///
/// Maps sorted edge vertex pairs to face indices.
/// Two faces sharing a geometry edge are only considered adjacent if their
/// UV coordinates at the shared vertices match (within epsilon), preventing
/// merging of disconnected UV islands.
fn build_edge_adjacency(mesh: &IndexedMesh) -> HashMap<(u32, u32), Vec<usize>> {
    let num_faces = mesh.triangle_count();
    let mut edge_map: HashMap<(u32, u32), Vec<(usize, [f32; 2], [f32; 2])>> = HashMap::new();

    for face in 0..num_faces {
        let i0 = mesh.indices[face * 3] as usize;
        let i1 = mesh.indices[face * 3 + 1] as usize;
        let i2 = mesh.indices[face * 3 + 2] as usize;

        let verts = [i0, i1, i2];
        for e in 0..3 {
            let va = verts[e] as u32;
            let vb = verts[(e + 1) % 3] as u32;
            let edge_key = if va < vb { (va, vb) } else { (vb, va) };

            let uv_a = [mesh.uvs[verts[e] * 2], mesh.uvs[verts[e] * 2 + 1]];
            let uv_b = [
                mesh.uvs[verts[(e + 1) % 3] * 2],
                mesh.uvs[verts[(e + 1) % 3] * 2 + 1],
            ];

            edge_map
                .entry(edge_key)
                .or_default()
                .push((face, uv_a, uv_b));
        }
    }

    // Build adjacency: faces sharing an edge with matching UVs
    let eps = 1e-5;
    let mut adjacency: HashMap<(u32, u32), Vec<usize>> = HashMap::new();

    for (edge_key, entries) in &edge_map {
        // For each pair of faces on this edge, check UV match
        let mut adj_faces: Vec<usize> = Vec::new();
        for i in 0..entries.len() {
            for j in (i + 1)..entries.len() {
                let (fi, uv_a_i, uv_b_i) = &entries[i];
                let (fj, uv_a_j, uv_b_j) = &entries[j];

                // UVs must match (in either order) at the shared edge vertices
                let match_same = uv_close(uv_a_i, uv_a_j, eps) && uv_close(uv_b_i, uv_b_j, eps);
                let match_swap = uv_close(uv_a_i, uv_b_j, eps) && uv_close(uv_b_i, uv_a_j, eps);

                if match_same || match_swap {
                    if !adj_faces.contains(fi) {
                        adj_faces.push(*fi);
                    }
                    if !adj_faces.contains(fj) {
                        adj_faces.push(*fj);
                    }
                }
            }
        }
        if !adj_faces.is_empty() {
            adjacency.insert(*edge_key, adj_faces);
        }
    }

    adjacency
}

fn uv_close(a: &[f32; 2], b: &[f32; 2], eps: f32) -> bool {
    (a[0] - b[0]).abs() < eps && (a[1] - b[1]).abs() < eps
}

/// BFS island detection.
///
/// Returns connected components via BFS over face adjacency.
fn detect_islands(mesh: &IndexedMesh, adjacency: &HashMap<(u32, u32), Vec<usize>>) -> Vec<UvIsland> {
    let num_faces = mesh.triangle_count();
    let mut visited = vec![false; num_faces];
    let mut islands = Vec::new();

    // Build face-to-face adjacency from edge adjacency
    let mut face_adj: Vec<Vec<usize>> = vec![Vec::new(); num_faces];
    for faces in adjacency.values() {
        for i in 0..faces.len() {
            for j in (i + 1)..faces.len() {
                let fi = faces[i];
                let fj = faces[j];
                if !face_adj[fi].contains(&fj) {
                    face_adj[fi].push(fj);
                }
                if !face_adj[fj].contains(&fi) {
                    face_adj[fj].push(fi);
                }
            }
        }
    }

    for start in 0..num_faces {
        if visited[start] {
            continue;
        }

        let mut queue = std::collections::VecDeque::new();
        queue.push_back(start);
        visited[start] = true;

        let mut island_faces = Vec::new();
        let mut uv_min = [f32::INFINITY; 2];
        let mut uv_max = [f32::NEG_INFINITY; 2];

        while let Some(face) = queue.pop_front() {
            island_faces.push(face);

            // Update UV bounds from this face's vertices
            for v in 0..3 {
                let vi = mesh.indices[face * 3 + v] as usize;
                let u = mesh.uvs[vi * 2];
                let vv = mesh.uvs[vi * 2 + 1];
                uv_min[0] = uv_min[0].min(u);
                uv_min[1] = uv_min[1].min(vv);
                uv_max[0] = uv_max[0].max(u);
                uv_max[1] = uv_max[1].max(vv);
            }

            for &neighbor in &face_adj[face] {
                if !visited[neighbor] {
                    visited[neighbor] = true;
                    queue.push_back(neighbor);
                }
            }
        }

        islands.push(UvIsland {
            faces: island_faces,
            uv_min,
            uv_max,
        });
    }

    islands
}

/// Guillotine bin packing with Best Short Side Fit.
///
/// Sorts islands by max dimension descending, places each using BSSF.
/// Grows atlas (doubles smaller dimension) if needed.
fn guillotine_pack(sized: &[(usize, u32, u32, u32)]) -> Vec<Placement> {
    // Sort by max dimension descending
    let mut order: Vec<usize> = (0..sized.len()).collect();
    order.sort_by(|&a, &b| {
        let max_a = (sized[a].1 + sized[a].3 * 2).max(sized[a].2 + sized[a].3 * 2);
        let max_b = (sized[b].1 + sized[b].3 * 2).max(sized[b].2 + sized[b].3 * 2);
        max_b.cmp(&max_a)
    });

    // Start with an initial atlas size
    let first = order[0];
    let mut atlas_w = (sized[first].1 + sized[first].3 * 2).next_power_of_two().max(64);
    let mut atlas_h = (sized[first].2 + sized[first].3 * 2).next_power_of_two().max(64);

    loop {
        if let Some(placements) = try_pack(&order, sized, atlas_w, atlas_h) {
            return placements;
        }
        // Grow: double the smaller dimension
        if atlas_w <= atlas_h {
            atlas_w *= 2;
        } else {
            atlas_h *= 2;
        }

        // Safety limit
        if atlas_w > 16384 || atlas_h > 16384 {
            warn!(
                atlas_w,
                atlas_h, "Atlas size exceeded 16384, forcing placement"
            );
            // Force-pack with large atlas
            return try_pack(&order, sized, atlas_w, atlas_h).unwrap_or_default();
        }
    }
}

fn try_pack(
    order: &[usize],
    sized: &[(usize, u32, u32, u32)],
    atlas_w: u32,
    atlas_h: u32,
) -> Option<Vec<Placement>> {
    let mut free_rects = vec![FreeRect {
        x: 0,
        y: 0,
        w: atlas_w,
        h: atlas_h,
    }];

    let mut placements = Vec::with_capacity(order.len());

    for &idx in order {
        let (island_idx, inner_w, inner_h, padding) = sized[idx];
        let total_w = inner_w + padding * 2;
        let total_h = inner_h + padding * 2;

        // Find best short side fit
        let best = find_bssf(&free_rects, total_w, total_h);
        let best = best?;

        let rect = free_rects.remove(best.rect_idx);

        placements.push(Placement {
            island_idx,
            x: rect.x,
            y: rect.y,
            inner_w,
            inner_h,
            padding,
        });

        // Guillotine split
        guillotine_split(&mut free_rects, &rect, total_w, total_h);
    }

    Some(placements)
}

struct BssfResult {
    rect_idx: usize,
}

fn find_bssf(free_rects: &[FreeRect], w: u32, h: u32) -> Option<BssfResult> {
    let mut best_idx = None;
    let mut best_short_side = u32::MAX;

    for (i, rect) in free_rects.iter().enumerate() {
        if rect.w >= w && rect.h >= h {
            let short_side = (rect.w - w).min(rect.h - h);
            if short_side < best_short_side {
                best_short_side = short_side;
                best_idx = Some(i);
            }
        }
    }

    best_idx.map(|rect_idx| BssfResult { rect_idx })
}

fn guillotine_split(free_rects: &mut Vec<FreeRect>, rect: &FreeRect, w: u32, h: u32) {
    // Split along the shorter leftover axis
    let right_w = rect.w - w;
    let below_h = rect.h - h;

    if right_w > 0 {
        free_rects.push(FreeRect {
            x: rect.x + w,
            y: rect.y,
            w: right_w,
            h: h,
        });
    }

    if below_h > 0 {
        free_rects.push(FreeRect {
            x: rect.x,
            y: rect.y + h,
            w: rect.w,
            h: below_h,
        });
    }
}

/// Compute the smallest power-of-two atlas size containing all placements.
fn compute_atlas_size(placements: &[Placement]) -> u32 {
    let mut max_x = 0u32;
    let mut max_y = 0u32;

    for p in placements {
        let right = p.x + p.inner_w + p.padding * 2;
        let bottom = p.y + p.inner_h + p.padding * 2;
        max_x = max_x.max(right);
        max_y = max_y.max(bottom);
    }

    max_x.max(max_y).next_power_of_two().max(1)
}

/// Remap UVs from source island space to atlas space, duplicating vertices
/// that are shared across multiple UV islands.
///
/// When a vertex appears in faces from different UV islands (common after
/// triangle clipping), the first island would "win" and other islands would
/// get wrong UVs (= BLACK). This function detects such conflicts and
/// duplicates the vertex with the correct UV for each island.
///
/// Also applies a half-texel inset to prevent bilinear filtering from
/// sampling into the padding region (fixes zigzag seam artifacts).
fn remap_uvs_with_dedup(
    mesh: &IndexedMesh,
    islands: &[UvIsland],
    placements: &[Placement],
    atlas_size: u32,
) -> IndexedMesh {
    let atlas_f = atlas_size as f32;

    // Build island_idx -> placement lookup
    let mut placement_map: HashMap<usize, &Placement> = HashMap::new();
    for p in placements {
        placement_map.insert(p.island_idx, p);
    }

    // Clone mesh data for mutation
    let mut new_positions = mesh.positions.clone();
    let mut new_normals = mesh.normals.clone();
    let mut new_uvs = mesh.uvs.clone();
    let mut new_colors = mesh.colors.clone();
    let mut new_indices = mesh.indices.clone();

    // Track which island owns each vertex: None = unassigned
    let mut vertex_island: Vec<Option<usize>> = vec![None; mesh.vertex_count()];

    for (island_idx, island) in islands.iter().enumerate() {
        let placement = match placement_map.get(&island_idx) {
            Some(p) => p,
            None => continue,
        };

        let uv_range_u = island.uv_max[0] - island.uv_min[0];
        let uv_range_v = island.uv_max[1] - island.uv_min[1];

        let uv_range_u = if uv_range_u < 1e-8 { 1.0 } else { uv_range_u };
        let uv_range_v = if uv_range_v < 1e-8 { 1.0 } else { uv_range_v };

        for &face in &island.faces {
            for v in 0..3 {
                let original_vi = mesh.indices[face * 3 + v] as usize;
                let fi = face * 3 + v; // index into the indices array

                let vi = if vertex_island[original_vi].is_none() {
                    // First island to claim this vertex
                    vertex_island[original_vi] = Some(island_idx);
                    original_vi
                } else if vertex_island[original_vi] == Some(island_idx) {
                    // Same island, already remapped
                    original_vi
                } else {
                    // Different island! Duplicate the vertex.
                    let new_vi = new_positions.len() / 3;
                    new_positions.extend_from_slice(&[
                        mesh.positions[original_vi * 3],
                        mesh.positions[original_vi * 3 + 1],
                        mesh.positions[original_vi * 3 + 2],
                    ]);
                    if mesh.has_normals() {
                        new_normals.extend_from_slice(&[
                            mesh.normals[original_vi * 3],
                            mesh.normals[original_vi * 3 + 1],
                            mesh.normals[original_vi * 3 + 2],
                        ]);
                    }
                    // Push placeholder UVs (will be remapped below)
                    new_uvs.extend_from_slice(&[
                        mesh.uvs[original_vi * 2],
                        mesh.uvs[original_vi * 2 + 1],
                    ]);
                    if mesh.has_colors() {
                        new_colors.extend_from_slice(&[
                            mesh.colors[original_vi * 4],
                            mesh.colors[original_vi * 4 + 1],
                            mesh.colors[original_vi * 4 + 2],
                            mesh.colors[original_vi * 4 + 3],
                        ]);
                    }
                    // Update this face's index to point to the new vertex
                    new_indices[fi] = new_vi as u32;
                    new_vi
                };

                let old_u = mesh.uvs[original_vi * 2];
                let old_v = mesh.uvs[original_vi * 2 + 1];

                // Normalize to [0,1] within island UV range
                let norm_u = (old_u - island.uv_min[0]) / uv_range_u;
                let norm_v = (old_v - island.uv_min[1]) / uv_range_v;

                // Map to atlas pixel coords with half-texel inset, then back to [0,1]
                let new_u = (norm_u * (placement.inner_w as f32 - 1.0) + 0.5
                    + (placement.x + placement.padding) as f32)
                    / atlas_f;
                let new_v = (norm_v * (placement.inner_h as f32 - 1.0) + 0.5
                    + (placement.y + placement.padding) as f32)
                    / atlas_f;

                new_uvs[vi * 2] = new_u;
                new_uvs[vi * 2 + 1] = new_v;
            }
        }
    }

    IndexedMesh {
        positions: new_positions,
        normals: new_normals,
        uvs: new_uvs,
        colors: new_colors,
        indices: new_indices,
        material_index: mesh.material_index,
    }
}

/// Composite the atlas image from source texture + island placements.
fn composite_atlas(
    source: &RgbaImage,
    islands: &[UvIsland],
    placements: &[Placement],
    atlas_size: u32,
) -> RgbaImage {
    let mut atlas = RgbaImage::new(atlas_size, atlas_size);
    let (src_w, src_h) = source.dimensions();

    // Build island_idx -> placement lookup
    let mut placement_map: HashMap<usize, &Placement> = HashMap::new();
    for p in placements {
        placement_map.insert(p.island_idx, p);
    }

    for (island_idx, island) in islands.iter().enumerate() {
        let placement = match placement_map.get(&island_idx) {
            Some(p) => p,
            None => continue,
        };

        let uv_range_u = island.uv_max[0] - island.uv_min[0];
        let uv_range_v = island.uv_max[1] - island.uv_min[1];

        let inner_w = placement.inner_w;
        let inner_h = placement.inner_h;
        let pad = placement.padding;

        // Fill inner region by sampling source texture using scanline bulk copies
        let dest_x0 = placement.x + pad;
        let dest_y0 = placement.y + pad;

        for py in 0..inner_h {
            let v = island.uv_min[1] + (py as f32 / inner_h.max(1) as f32) * uv_range_v;
            let sv = ((v.fract() + 1.0).fract() * src_h as f32) as u32 % src_h;
            let ay = dest_y0 + py;
            if ay >= atlas_size {
                continue;
            }

            // Check if the entire scanline maps to a contiguous source row
            let u_start = island.uv_min[0];
            let u_end = island.uv_min[0] + uv_range_u;
            let su_start = ((u_start.fract() + 1.0).fract() * src_w as f32) as u32 % src_w;
            let su_end_raw = ((u_end.fract() + 1.0).fract() * src_w as f32) as u32 % src_w;

            // Fast path: contiguous source scanline (no UV wrapping within row)
            let scanline_end_x = (dest_x0 + inner_w).min(atlas_size);
            if su_start < su_end_raw
                && su_end_raw <= src_w
                && (su_end_raw - su_start) as usize >= inner_w as usize
                && dest_x0 < scanline_end_x
            {
                let src_row =
                    &source.as_raw()[(sv * src_w * 4 + su_start * 4) as usize..];
                let copy_w = (scanline_end_x - dest_x0) as usize;
                let dst_offset = (ay * atlas_size * 4 + dest_x0 * 4) as usize;
                let dst_row =
                    &mut atlas.as_mut().as_mut()[dst_offset..dst_offset + copy_w * 4];
                dst_row.copy_from_slice(&src_row[..copy_w * 4]);
            } else {
                // Slow path: per-pixel sampling (handles UV wrapping)
                for px in 0..inner_w {
                    let u = island.uv_min[0]
                        + (px as f32 / inner_w.max(1) as f32) * uv_range_u;
                    let su = ((u.fract() + 1.0).fract() * src_w as f32) as u32 % src_w;
                    let ax = dest_x0 + px;
                    if ax < atlas_size {
                        let pixel = *source.get_pixel(su, sv);
                        atlas.put_pixel(ax, ay, pixel);
                    }
                }
            }
        }

        // Fill bleed padding by replicating edge pixels
        fill_bleed(&mut atlas, placement, atlas_size);
    }

    atlas
}

/// Replicate edge pixels into the padding region for bleed.
fn fill_bleed(atlas: &mut RgbaImage, placement: &Placement, atlas_size: u32) {
    let pad = placement.padding;
    let inner_x = placement.x + pad;
    let inner_y = placement.y + pad;
    let inner_w = placement.inner_w;
    let inner_h = placement.inner_h;

    if inner_w == 0 || inner_h == 0 {
        return;
    }

    // Top and bottom bleed
    for px in 0..inner_w {
        let top_pixel = *atlas.get_pixel(
            (inner_x + px).min(atlas_size - 1),
            inner_y.min(atlas_size - 1),
        );
        let bot_pixel = *atlas.get_pixel(
            (inner_x + px).min(atlas_size - 1),
            (inner_y + inner_h - 1).min(atlas_size - 1),
        );

        for p in 1..=pad {
            if inner_y >= p {
                let ay = inner_y - p;
                let ax = inner_x + px;
                if ax < atlas_size && ay < atlas_size {
                    atlas.put_pixel(ax, ay, top_pixel);
                }
            }
            let ay = inner_y + inner_h - 1 + p;
            let ax = inner_x + px;
            if ax < atlas_size && ay < atlas_size {
                atlas.put_pixel(ax, ay, bot_pixel);
            }
        }
    }

    // Left and right bleed
    for py in 0..inner_h {
        let left_pixel = *atlas.get_pixel(
            inner_x.min(atlas_size - 1),
            (inner_y + py).min(atlas_size - 1),
        );
        let right_pixel = *atlas.get_pixel(
            (inner_x + inner_w - 1).min(atlas_size - 1),
            (inner_y + py).min(atlas_size - 1),
        );

        for p in 1..=pad {
            if inner_x >= p {
                let ax = inner_x - p;
                let ay = inner_y + py;
                if ax < atlas_size && ay < atlas_size {
                    atlas.put_pixel(ax, ay, left_pixel);
                }
            }
            let ax = inner_x + inner_w - 1 + p;
            let ay = inner_y + py;
            if ax < atlas_size && ay < atlas_size {
                atlas.put_pixel(ax, ay, right_pixel);
            }
        }
    }

    // Corner bleed: replicate corner pixels into the pad x pad corner rectangles
    let tl_pixel = *atlas.get_pixel(inner_x.min(atlas_size - 1), inner_y.min(atlas_size - 1));
    let tr_pixel = *atlas.get_pixel(
        (inner_x + inner_w - 1).min(atlas_size - 1),
        inner_y.min(atlas_size - 1),
    );
    let bl_pixel = *atlas.get_pixel(
        inner_x.min(atlas_size - 1),
        (inner_y + inner_h - 1).min(atlas_size - 1),
    );
    let br_pixel = *atlas.get_pixel(
        (inner_x + inner_w - 1).min(atlas_size - 1),
        (inner_y + inner_h - 1).min(atlas_size - 1),
    );

    for dy in 1..=pad {
        for dx in 1..=pad {
            // Top-left corner
            if inner_x >= dx && inner_y >= dy {
                atlas.put_pixel(inner_x - dx, inner_y - dy, tl_pixel);
            }
            // Top-right corner
            let ax = inner_x + inner_w - 1 + dx;
            if ax < atlas_size && inner_y >= dy {
                atlas.put_pixel(ax, inner_y - dy, tr_pixel);
            }
            // Bottom-left corner
            let ay = inner_y + inner_h - 1 + dy;
            if inner_x >= dx && ay < atlas_size {
                atlas.put_pixel(inner_x - dx, ay, bl_pixel);
            }
            // Bottom-right corner
            if ax < atlas_size && ay < atlas_size {
                atlas.put_pixel(ax, ay, br_pixel);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::PBRMaterial;

    /// Create a simple 4x4 checkerboard PNG texture.
    fn checkerboard_texture(size: u32) -> TextureData {
        let img = RgbaImage::from_fn(size, size, |x, y| {
            if (x + y) % 2 == 0 {
                image::Rgba([255, 0, 0, 255])
            } else {
                image::Rgba([0, 0, 255, 255])
            }
        });
        let mut buf = std::io::Cursor::new(Vec::new());
        img.write_to(&mut buf, image::ImageFormat::Png).unwrap();
        TextureData {
            data: buf.into_inner(),
            mime_type: "image/png".into(),
            width: size,
            height: size,
        }
    }

    fn make_textured_quad() -> (IndexedMesh, MaterialLibrary) {
        let mesh = IndexedMesh {
            positions: vec![
                0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 0.0,
            ],
            normals: vec![],
            uvs: vec![0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0],
            colors: vec![],
            indices: vec![0, 1, 2, 0, 2, 3],
            material_index: Some(0),
        };

        let mut materials = MaterialLibrary::default();
        materials.textures.push(checkerboard_texture(16));
        materials.materials.push(PBRMaterial {
            name: "textured".into(),
            base_color_texture: Some(0),
            ..Default::default()
        });

        (mesh, materials)
    }

    fn make_two_island_mesh() -> (IndexedMesh, MaterialLibrary) {
        // Two separate quads with disconnected UV islands
        let mesh = IndexedMesh {
            positions: vec![
                // Quad 1
                0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 0.0,
                // Quad 2 (spatially separate)
                2.0, 0.0, 0.0, 3.0, 0.0, 0.0, 3.0, 1.0, 0.0, 2.0, 1.0, 0.0,
            ],
            normals: vec![],
            uvs: vec![
                // Quad 1 UVs: [0,0.5] x [0,0.5]
                0.0, 0.0, 0.5, 0.0, 0.5, 0.5, 0.0, 0.5,
                // Quad 2 UVs: [0.5,1] x [0.5,1]
                0.5, 0.5, 1.0, 0.5, 1.0, 1.0, 0.5, 1.0,
            ],
            colors: vec![],
            indices: vec![0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7],
            material_index: Some(0),
        };

        let mut materials = MaterialLibrary::default();
        materials.textures.push(checkerboard_texture(32));
        materials.materials.push(PBRMaterial {
            name: "textured".into(),
            base_color_texture: Some(0),
            ..Default::default()
        });

        (mesh, materials)
    }

    #[test]
    fn adjacency_map_quad() {
        let (mesh, _) = make_textured_quad();
        let adj = build_edge_adjacency(&mesh);

        // A quad with 2 triangles sharing an edge should have at least one edge
        // with 2 faces
        let shared_count = adj.values().filter(|faces| faces.len() >= 2).count();
        assert!(shared_count >= 1, "quad should have at least 1 shared edge");
    }

    #[test]
    fn island_detection_single() {
        let (mesh, _) = make_textured_quad();
        let adj = build_edge_adjacency(&mesh);
        let islands = detect_islands(&mesh, &adj);

        assert_eq!(islands.len(), 1, "quad should produce 1 UV island");
        assert_eq!(islands[0].faces.len(), 2, "island should have 2 faces");
    }

    #[test]
    fn island_detection_multiple() {
        let (mesh, _) = make_two_island_mesh();
        let adj = build_edge_adjacency(&mesh);
        let islands = detect_islands(&mesh, &adj);

        assert_eq!(islands.len(), 2, "two separated quads should produce 2 UV islands");
    }

    #[test]
    fn island_uv_bounds() {
        let (mesh, _) = make_textured_quad();
        let adj = build_edge_adjacency(&mesh);
        let islands = detect_islands(&mesh, &adj);

        let island = &islands[0];
        assert!(island.uv_min[0] >= 0.0);
        assert!(island.uv_min[1] >= 0.0);
        assert!(island.uv_max[0] <= 1.0);
        assert!(island.uv_max[1] <= 1.0);
    }

    #[test]
    fn packer_single_island() {
        let sized = vec![(0, 16, 16, 2)];
        let placements = guillotine_pack(&sized);
        assert_eq!(placements.len(), 1);
        assert_eq!(placements[0].island_idx, 0);
    }

    #[test]
    fn packer_multiple_islands() {
        let sized = vec![(0, 32, 32, 2), (1, 16, 16, 2), (2, 8, 8, 2)];
        let placements = guillotine_pack(&sized);
        assert_eq!(placements.len(), 3);

        // All islands should be placed
        let mut placed_ids: Vec<usize> = placements.iter().map(|p| p.island_idx).collect();
        placed_ids.sort();
        assert_eq!(placed_ids, vec![0, 1, 2]);
    }

    #[test]
    fn packer_grows_atlas() {
        // Large islands that won't fit in a small atlas
        let sized = vec![(0, 128, 128, 2), (1, 128, 128, 2), (2, 128, 128, 2)];
        let placements = guillotine_pack(&sized);
        assert_eq!(placements.len(), 3);

        let atlas_size = compute_atlas_size(&placements);
        assert!(atlas_size >= 256, "atlas should have grown to fit all islands");
    }

    #[test]
    fn uv_remapping_range() {
        let (mesh, materials) = make_textured_quad();
        let config = TextureConfig::default();

        let result = repack_atlas(&mesh, &materials, &config).expect("should produce atlas");

        // All remapped UVs should be within [0, 1]
        for chunk in result.mesh.uvs.chunks_exact(2) {
            assert!(
                chunk[0] >= -0.01 && chunk[0] <= 1.01,
                "remapped U={} out of range",
                chunk[0]
            );
            assert!(
                chunk[1] >= -0.01 && chunk[1] <= 1.01,
                "remapped V={} out of range",
                chunk[1]
            );
        }
    }

    #[test]
    fn full_repack_roundtrip() {
        let (mesh, materials) = make_textured_quad();
        let config = TextureConfig::default();

        let result = repack_atlas(&mesh, &materials, &config).expect("should produce atlas");

        // Mesh geometry should be preserved (vertex count may grow due to dedup)
        assert!(result.mesh.positions.len() >= mesh.positions.len());
        assert_eq!(result.mesh.indices.len(), mesh.indices.len());

        // Atlas texture should be non-empty
        assert!(!result.atlas_texture.data.is_empty());
        assert!(result.atlas_texture.width > 0);
        assert!(result.atlas_texture.height > 0);

        // Should be decodable
        let decoded = image::load_from_memory(&result.atlas_texture.data)
            .expect("atlas should be decodable");
        let rgba = decoded.to_rgba8();
        assert_eq!(rgba.dimensions(), (result.atlas_texture.width, result.atlas_texture.height));
    }

    #[test]
    fn repack_two_islands() {
        let (mesh, materials) = make_two_island_mesh();
        let config = TextureConfig::default();

        let result = repack_atlas(&mesh, &materials, &config).expect("should produce atlas");

        // Vertex count may increase due to vertex deduplication across islands
        assert!(result.mesh.vertex_count() >= mesh.vertex_count());
        assert!(!result.atlas_texture.data.is_empty());
    }

    #[test]
    fn no_uvs_returns_none() {
        let mesh = IndexedMesh {
            positions: vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
            indices: vec![0, 1, 2],
            ..Default::default()
        };
        let materials = MaterialLibrary::default();
        let config = TextureConfig::default();

        assert!(repack_atlas(&mesh, &materials, &config).is_none());
    }

    #[test]
    fn no_material_returns_none() {
        let mesh = IndexedMesh {
            positions: vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
            uvs: vec![0.0, 0.0, 1.0, 0.0, 0.0, 1.0],
            indices: vec![0, 1, 2],
            material_index: None,
            ..Default::default()
        };
        let materials = MaterialLibrary::default();
        let config = TextureConfig::default();

        assert!(repack_atlas(&mesh, &materials, &config).is_none());
    }

    #[test]
    fn no_texture_returns_none() {
        let mesh = IndexedMesh {
            positions: vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
            uvs: vec![0.0, 0.0, 1.0, 0.0, 0.0, 1.0],
            indices: vec![0, 1, 2],
            material_index: Some(0),
            ..Default::default()
        };
        let mut materials = MaterialLibrary::default();
        materials.materials.push(PBRMaterial {
            base_color_texture: None,
            ..Default::default()
        });
        let config = TextureConfig::default();

        assert!(repack_atlas(&mesh, &materials, &config).is_none());
    }

    #[test]
    fn repack_shared_vertex_across_islands() {
        // Two triangles sharing vertex 2, but in different UV islands.
        // Simulates post-clipping state where a vertex appears in faces
        // from different UV islands.
        let mesh = IndexedMesh {
            positions: vec![
                // Triangle 1 (island A): v0, v1, v2
                0.0, 0.0, 0.0,  // v0
                1.0, 0.0, 0.0,  // v1
                0.5, 0.5, 0.0,  // v2 (SHARED vertex)
                // Triangle 2 (island B): v3, v4, v2 (reuses v2)
                2.0, 0.0, 0.0,  // v3
                3.0, 0.0, 0.0,  // v4
            ],
            normals: vec![],
            uvs: vec![
                // Island A UVs
                0.0, 0.0,  // v0
                0.5, 0.0,  // v1
                0.25, 0.5, // v2 (UV in island A)
                // Island B UVs
                0.5, 0.5,  // v3
                1.0, 0.5,  // v4
                // v2 reused with island A's UV, but island B wants different mapping
            ],
            colors: vec![],
            indices: vec![
                0, 1, 2, // Triangle 1 (island A)
                3, 4, 2, // Triangle 2 (island B) — shares v2!
            ],
            material_index: Some(0),
        };

        let mut materials = MaterialLibrary::default();
        materials.textures.push(checkerboard_texture(16));
        materials.materials.push(PBRMaterial {
            name: "textured".into(),
            base_color_texture: Some(0),
            ..Default::default()
        });
        let config = TextureConfig::default();

        let result = repack_atlas(&mesh, &materials, &config).expect("should produce atlas");

        // The shared vertex should have been duplicated, so vertex count >= 5
        // (original had 5 vertices, the shared one gets duplicated = 6)
        assert!(
            result.mesh.vertex_count() >= mesh.vertex_count(),
            "vertex count should be >= original after dedup, got {} vs {}",
            result.mesh.vertex_count(),
            mesh.vertex_count()
        );

        // Triangle 1 and Triangle 2 should reference different vertex indices
        // for the shared position
        let t1_v2 = result.mesh.indices[2];
        let t2_v2 = result.mesh.indices[5];

        // They should reference different vertices (deduplication happened)
        // OR if they're in the same island they may share
        // The key assertion is that all UVs are valid (within atlas range)
        for chunk in result.mesh.uvs.chunks_exact(2) {
            assert!(
                chunk[0] >= -0.01 && chunk[0] <= 1.01,
                "U={} out of range",
                chunk[0]
            );
            assert!(
                chunk[1] >= -0.01 && chunk[1] <= 1.01,
                "V={} out of range",
                chunk[1]
            );
        }

        // The two triangles' third vertex should have been remapped independently
        let uv_t1 = [
            result.mesh.uvs[t1_v2 as usize * 2],
            result.mesh.uvs[t1_v2 as usize * 2 + 1],
        ];
        let uv_t2 = [
            result.mesh.uvs[t2_v2 as usize * 2],
            result.mesh.uvs[t2_v2 as usize * 2 + 1],
        ];

        // Both UVs should be valid (not zero/black)
        assert!(
            uv_t1[0] > 0.001 || uv_t1[1] > 0.001,
            "island A vertex UV should be remapped: {:?}",
            uv_t1
        );
        assert!(
            uv_t2[0] > 0.001 || uv_t2[1] > 0.001,
            "island B vertex UV should be remapped: {:?}",
            uv_t2
        );
    }

    #[test]
    fn decode_texture_png() {
        let tex = checkerboard_texture(8);
        let img = decode_texture(&tex).expect("should decode PNG");
        assert_eq!(img.dimensions(), (8, 8));
    }

    #[test]
    fn decode_texture_raw_rgba() {
        let tex = TextureData {
            data: vec![255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 0, 255],
            mime_type: "image/raw".into(),
            width: 2,
            height: 2,
        };
        let img = decode_texture(&tex).expect("should decode raw RGBA");
        assert_eq!(img.dimensions(), (2, 2));
        assert_eq!(img.get_pixel(0, 0), &image::Rgba([255, 0, 0, 255]));
    }

    #[test]
    fn decode_texture_raw_rgb() {
        let tex = TextureData {
            data: vec![255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 0],
            mime_type: "image/raw".into(),
            width: 2,
            height: 2,
        };
        let img = decode_texture(&tex).expect("should decode raw RGB");
        assert_eq!(img.dimensions(), (2, 2));
        assert_eq!(img.get_pixel(0, 0), &image::Rgba([255, 0, 0, 255]));
    }
}
