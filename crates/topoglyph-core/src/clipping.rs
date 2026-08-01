use crate::geometry::{CellDescriptor, CellMask, GridOptions, PortMask};
use vectomancy_geometry::PolylineScene;

/// Map a polyline scene into a grid of CellDescriptors.
pub fn process_scene(
    scene: &PolylineScene,
    options: &GridOptions,
) -> (usize, usize, Vec<CellDescriptor>) {
    // Use the frame's actual pixel dimensions (`scene.dimensions`) as the
    // coordinate reference, not the extracted skeleton's content bounding
    // box (`scene.bounds`). Two problems if `bounds` is used instead:
    //
    // 1. Aspect-ratio blowup: a skeleton that degenerates to a near-zero-
    //    width/height bbox (e.g. a blank/near-blank video frame) divides by
    //    a value clamped to `1e-5`, so `aspect` can explode to the tens of
    //    millions and `columns * rows` tries to allocate terabytes (this is
    //    exactly what crashed `topoglyph video` on some bad-apple.mp4
    //    frames: "memory allocation of 37013760000000 bytes failed").
    // 2. Per-frame zoom/pan jitter: `bounds` is however much of the frame
    //    the skeleton happens to occupy, which varies frame to frame even
    //    when the source video's dimensions never change. Scaling content
    //    to fill the grid based on `bounds` means every frame gets its own
    //    independent crop-and-zoom, so the rendered subject appears to
    //    randomly grow/shrink/shift between frames ("有时扁有时宽有时矮").
    //    Anchoring on `dimensions` keeps one fixed frame of reference for
    //    the whole conversion, exactly like the source video/image.
    let (dim_w, dim_h) = scene.dimensions;
    let width = if dim_w > 0 { dim_w as f64 } else { 1.0 };
    let height = if dim_h > 0 { dim_h as f64 } else { 1.0 };

    let columns = options.columns;
    let rows = options.rows.unwrap_or_else(|| {
        let aspect = (height / width) as f32;
        ((columns as f32 * aspect * options.cell_aspect_ratio).round() as usize).max(1)
    });

    let mut cells = vec![
        CellDescriptor {
            mask: CellMask::new(),
            ports: PortMask::empty(),
            orientation: [0.0; 8],
            density: 0.0,
            centroid: [0.0; 2],
            curvature: 0.0,
            stroke_count: 0,
            color: None,
        };
        columns * rows
    ];

    // Subcell dimensions across the whole canvas, kept in continuous
    // (floating point) space so segment coordinates are never prematurely
    // rounded before clipping. Rounding to integers up front (the previous
    // implementation) silently dropped any portion of a segment that fell
    // outside the canvas instead of clipping it to the boundary.
    let total_sub_cols = columns * options.subcell_width as usize;
    let total_sub_rows = rows * options.subcell_height as usize;

    let scale_x = total_sub_cols as f64 / width;
    let scale_y = total_sub_rows as f64 / height;

    for path in &scene.paths {
        let pts = &path.geometry.points;
        if pts.is_empty() {
            continue;
        }

        for i in 0..pts.len().saturating_sub(1) {
            let p0 = pts[i];
            let p1 = pts[i + 1];

            let x0 = p0.x * scale_x;
            let y0 = p0.y * scale_y;
            let x1 = p1.x * scale_x;
            let y1 = p1.y * scale_y;

            clip_segment_into_cells(
                x0,
                y0,
                x1,
                y1,
                columns,
                rows,
                options,
                &mut cells,
                path.color_style.as_deref(),
            );
        }

        // Handle closed paths
        if path.geometry.closed && pts.len() > 2 {
            let p0 = pts[pts.len() - 1];
            let p1 = pts[0];
            let x0 = p0.x * scale_x;
            let y0 = p0.y * scale_y;
            let x1 = p1.x * scale_x;
            let y1 = p1.y * scale_y;
            clip_segment_into_cells(
                x0,
                y0,
                x1,
                y1,
                columns,
                rows,
                options,
                &mut cells,
                path.color_style.as_deref(),
            );
        }
    }

    // Extract per-cell orientation/density/centroid/curvature/stroke_count
    // features from the masks that were just rasterized. This is what lets
    // `match_scene`'s multi-factor scoring (see `crate::matching`) tell two
    // glyphs with the same XOR mask distance apart by how their stroke
    // actually looks — e.g. a straight diagonal vs. a jagged one with the
    // same bit count.
    let sw = options.subcell_width as usize;
    let sh = options.subcell_height as usize;
    for cell in &mut cells {
        let features = crate::features::extract_features(&cell.mask, sw, sh);
        cell.orientation = features.orientation;
        cell.density = features.density;
        cell.centroid = features.centroid;
        cell.curvature = features.curvature;
        cell.stroke_count = features.stroke_count;
    }

    (columns, rows, cells)
}

/// Liang-Barsky line-clipping algorithm. Clips the segment `(x0,y0)-(x1,y1)`
/// against the axis-aligned rectangle `[xmin,xmax] x [ymin,ymax]` and returns
/// the clipped endpoints, or `None` if the segment doesn't intersect the
/// rectangle at all.
///
/// Unlike clamping/rounding coordinates before rasterizing, this computes
/// the exact fractional entry/exit points of the segment against the cell
/// boundary, which is what lets [`clip_segment_into_cells`] hand each cell a
/// [`LocalSegment`]-equivalent (the clipped sub-segment) instead of a
/// globally pixel-walked trail that can straddle or skip cell boundaries.
/// Axis-aligned clip rectangle, grouped into a struct so
/// [`liang_barsky_clip`] stays under clippy's argument-count lint while
/// still taking the segment endpoints as plain coordinates.
#[derive(Debug, Clone, Copy)]
struct ClipRect {
    xmin: f64,
    ymin: f64,
    xmax: f64,
    ymax: f64,
}

fn liang_barsky_clip(
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
    rect: ClipRect,
) -> Option<(f64, f64, f64, f64)> {
    let dx = x1 - x0;
    let dy = y1 - y0;

    let mut t0 = 0.0f64;
    let mut t1 = 1.0f64;

    // (p, q) pairs for the left, right, bottom and top boundaries.
    let checks = [
        (-dx, x0 - rect.xmin),
        (dx, rect.xmax - x0),
        (-dy, y0 - rect.ymin),
        (dy, rect.ymax - y0),
    ];

    for (p, q) in checks {
        if p == 0.0 {
            if q < 0.0 {
                // Parallel to this boundary and outside it: no intersection.
                return None;
            }
        } else {
            let r = q / p;
            if p < 0.0 {
                if r > t1 {
                    return None;
                }
                if r > t0 {
                    t0 = r;
                }
            } else {
                if r < t0 {
                    return None;
                }
                if r < t1 {
                    t1 = r;
                }
            }
        }
    }

    if t0 > t1 {
        return None;
    }

    Some((x0 + t0 * dx, y0 + t0 * dy, x0 + t1 * dx, y0 + t1 * dy))
}

/// Enumerates the cells a segment's bounding box overlaps and, for each one,
/// clips the segment to that cell's exact boundary via [`liang_barsky_clip`]
/// before rasterizing the clipped sub-segment into the cell's subcell mask.
///
/// A straight line only ever passes through cells within its own bounding
/// box, so enumerating that box and rejecting cells the clip test misses is
/// equivalent to (and simpler than) a full Amanatides-Woo grid traversal,
/// at the cost of a handful of Liang-Barsky calls that return `None`.
#[allow(clippy::too_many_arguments)]
fn clip_segment_into_cells(
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
    cols: usize,
    rows: usize,
    opts: &GridOptions,
    cells: &mut [CellDescriptor],
    color: Option<&str>,
) {
    let sw = opts.subcell_width as f64;
    let sh = opts.subcell_height as f64;

    let cell_x_min = (x0.min(x1) / sw).floor();
    let cell_x_max = (x0.max(x1) / sw).floor();
    let cell_y_min = (y0.min(y1) / sh).floor();
    let cell_y_max = (y0.max(y1) / sh).floor();

    if !cell_x_min.is_finite() || !cell_y_min.is_finite() {
        return;
    }

    let cx_start = cell_x_min.max(0.0) as usize;
    let cx_end = (cell_x_max.max(0.0) as usize).min(cols.saturating_sub(1));
    let cy_start = cell_y_min.max(0.0) as usize;
    let cy_end = (cell_y_max.max(0.0) as usize).min(rows.saturating_sub(1));

    if cell_x_max < 0.0 || cell_y_max < 0.0 || cx_start >= cols || cy_start >= rows {
        return;
    }

    for cy in cy_start..=cy_end {
        for cx in cx_start..=cx_end {
            let rect_xmin = cx as f64 * sw;
            let rect_xmax = rect_xmin + sw;
            let rect_ymin = cy as f64 * sh;
            let rect_ymax = rect_ymin + sh;
            let rect = ClipRect {
                xmin: rect_xmin,
                ymin: rect_ymin,
                xmax: rect_xmax,
                ymax: rect_ymax,
            };

            if let Some((lx0, ly0, lx1, ly1)) = liang_barsky_clip(x0, y0, x1, y1, rect) {
                rasterize_local_segment(
                    cx,
                    cy,
                    lx0 - rect_xmin,
                    ly0 - rect_ymin,
                    lx1 - rect_xmin,
                    ly1 - rect_ymin,
                    cols,
                    opts,
                    cells,
                    color,
                );
            }
        }
    }
}

/// Rasterizes a segment that has already been exactly clipped to a single
/// cell's boundary (`lx0/ly0/lx1/ly1` are in that cell's local subcell space,
/// `[0, subcell_width] x [0, subcell_height]`) into the cell's `CellMask`,
/// and records any subcell-grid ports the segment touches.
///
/// Because the segment is guaranteed to lie within the cell already, a
/// Bresenham-style integer walk here is safe (no canvas-boundary drop risk)
/// and cheap — it only ever visits the handful of subcells this one cell
/// contains.
#[allow(clippy::too_many_arguments)]
fn rasterize_local_segment(
    cx: usize,
    cy: usize,
    lx0: f64,
    ly0: f64,
    lx1: f64,
    ly1: f64,
    cols: usize,
    opts: &GridOptions,
    cells: &mut [CellDescriptor],
    color: Option<&str>,
) {
    let sw = opts.subcell_width as usize;
    let sh = opts.subcell_height as usize;
    let cell_idx = cy * cols + cx;

    let clamp_sub = |v: f64, max: usize| -> isize {
        v.round().clamp(0.0, (max.saturating_sub(1)) as f64) as isize
    };

    let mut sub_x0 = clamp_sub(lx0, sw);
    let mut sub_y0 = clamp_sub(ly0, sh);
    let sub_x1 = clamp_sub(lx1, sw);
    let sub_y1 = clamp_sub(ly1, sh);

    let dx = (sub_x1 - sub_x0).abs();
    let sx: isize = if sub_x0 < sub_x1 { 1 } else { -1 };
    let dy = -(sub_y1 - sub_y0).abs();
    let sy: isize = if sub_y0 < sub_y1 { 1 } else { -1 };
    let mut err = dx + dy;

    loop {
        set_subcell_bit(cell_idx, sub_x0 as usize, sub_y0 as usize, sw, cells);

        if sub_y0 == 0 {
            cells[cell_idx].ports.insert(PortMask::N);
        }
        if sub_y0 as usize == sh.saturating_sub(1) {
            cells[cell_idx].ports.insert(PortMask::S);
        }
        if sub_x0 == 0 {
            cells[cell_idx].ports.insert(PortMask::W);
        }
        if sub_x0 as usize == sw.saturating_sub(1) {
            cells[cell_idx].ports.insert(PortMask::E);
        }
        if let Some(c) = color {
            cells[cell_idx].color = Some(c.to_string());
        }

        if sub_x0 == sub_x1 && sub_y0 == sub_y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            sub_x0 += sx;
        }
        if e2 <= dx {
            err += dx;
            sub_y0 += sy;
        }
    }
}

fn set_subcell_bit(
    cell_idx: usize,
    sub_x: usize,
    sub_y: usize,
    subcell_width: usize,
    cells: &mut [CellDescriptor],
) {
    let bit_idx = sub_y * subcell_width + sub_x;
    let word_idx = bit_idx / 64;
    let bit_offset = bit_idx % 64;
    if word_idx < cells[cell_idx].mask.words.len() {
        cells[cell_idx].mask.words[word_idx] |= 1 << bit_offset;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectomancy_geometry::{BoundingBox, Point2D, Polyline, StyledPath};

    fn scene_with_segment(p0: Point2D, p1: Point2D, closed: bool) -> PolylineScene {
        PolylineScene {
            paths: vec![StyledPath {
                geometry: Polyline {
                    points: vec![p0, p1],
                    closed,
                },
                color_style: None,
            }],
            dimensions: (10, 10),
            bounds: BoundingBox::new(
                p0.x.min(p1.x),
                p0.y.min(p1.y),
                p0.x.max(p1.x).max(p0.x + 1e-3),
                p0.y.max(p1.y).max(p0.y + 1e-3),
            ),
        }
    }

    fn unit_rect() -> ClipRect {
        ClipRect {
            xmin: 0.0,
            ymin: 0.0,
            xmax: 10.0,
            ymax: 10.0,
        }
    }

    #[test]
    fn liang_barsky_clips_segment_crossing_rect() {
        // Horizontal segment crossing straight through a 10x10 rect.
        let clipped = liang_barsky_clip(-5.0, 5.0, 15.0, 5.0, unit_rect());
        assert_eq!(clipped, Some((0.0, 5.0, 10.0, 5.0)));
    }

    #[test]
    fn liang_barsky_rejects_segment_entirely_outside() {
        let clipped = liang_barsky_clip(-5.0, -5.0, -1.0, -1.0, unit_rect());
        assert_eq!(clipped, None);
    }

    #[test]
    fn liang_barsky_handles_degenerate_point_segment() {
        // A zero-length segment (dx = dy = 0) sitting inside the rect must
        // still clip to itself rather than being treated as "parallel and
        // outside".
        let clipped = liang_barsky_clip(3.0, 4.0, 3.0, 4.0, unit_rect());
        assert_eq!(clipped, Some((3.0, 4.0, 3.0, 4.0)));
    }

    #[test]
    fn process_scene_produces_nonempty_mask_for_diagonal_line() {
        let scene = scene_with_segment(Point2D::new(0.0, 0.0), Point2D::new(10.0, 10.0), false);
        let opts = GridOptions {
            columns: 4,
            rows: Some(4),
            ..Default::default()
        };
        let (cols, rows, cells) = process_scene(&scene, &opts);
        assert_eq!(cols, 4);
        assert_eq!(rows, 4);
        let hit_cells = cells
            .iter()
            .filter(|c| c.mask.words.iter().any(|&w| w != 0))
            .count();
        assert!(hit_cells > 0, "diagonal line should mark at least one cell");
    }

    #[test]
    fn process_scene_segment_spanning_canvas_edge_is_clipped_not_dropped() {
        // A segment that would previously be silently dropped once it
        // rounded to a negative subcell coordinate must now still leave a
        // mark inside the visible grid, since it gets clipped to the
        // canvas boundary instead of skipped pixel-by-pixel.
        let scene = scene_with_segment(Point2D::new(-5.0, 2.0), Point2D::new(2.0, 2.0), false);
        let opts = GridOptions {
            columns: 4,
            rows: Some(4),
            ..Default::default()
        };
        let (_, _, cells) = process_scene(&scene, &opts);
        let hit_cells = cells
            .iter()
            .filter(|c| c.mask.words.iter().any(|&w| w != 0))
            .count();
        assert!(
            hit_cells > 0,
            "segment crossing the canvas boundary must still be rasterized where it overlaps the grid"
        );
    }

    #[test]
    fn process_scene_empty_scene_yields_empty_grid() {
        let scene = PolylineScene {
            paths: vec![],
            dimensions: (10, 10),
            bounds: BoundingBox::new(0.0, 0.0, 1.0, 1.0),
        };
        let opts = GridOptions {
            columns: 3,
            rows: Some(3),
            ..Default::default()
        };
        let (cols, rows, cells) = process_scene(&scene, &opts);
        assert_eq!(cells.len(), cols * rows);
        assert!(cells.iter().all(|c| c.mask.words.iter().all(|&w| w == 0)));
    }

    #[test]
    fn process_scene_detects_boundary_ports() {
        // A vertical segment along the left edge of a single-cell grid
        // should register the West port.
        let scene = scene_with_segment(Point2D::new(0.0, 0.0), Point2D::new(0.0, 1.0), false);
        let opts = GridOptions {
            columns: 1,
            rows: Some(1),
            ..Default::default()
        };
        let (_, _, cells) = process_scene(&scene, &opts);
        assert!(cells[0].ports.contains(PortMask::W));
    }
}
