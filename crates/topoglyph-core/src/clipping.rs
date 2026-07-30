use crate::geometry::{CellDescriptor, CellMask, GridOptions, PortMask};
use vectomancy_geometry::{PolylineScene, Point2D};
use smallvec::SmallVec;

/// Map a polyline scene into a grid of CellDescriptors.
pub fn process_scene(scene: &PolylineScene, options: &GridOptions) -> (usize, usize, Vec<CellDescriptor>) {
    let bounds = &scene.bounds;
    let width = (bounds.max_x - bounds.min_x).max(1e-5);
    let height = (bounds.max_y - bounds.min_y).max(1e-5);
    
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

    // Subcell dimensions across the whole canvas
    let total_sub_cols = columns * options.subcell_width as usize;
    let total_sub_rows = rows * options.subcell_height as usize;

    let scale_x = total_sub_cols as f64 / width;
    let scale_y = total_sub_rows as f64 / height;

    for path in &scene.paths {
        let pts = &path.geometry.points;
        if pts.is_empty() { continue; }

        for i in 0..pts.len() - 1 {
            let p0 = pts[i];
            let p1 = pts[i + 1];

            let x0 = ((p0.x - bounds.min_x) * scale_x).round() as isize;
            let y0 = ((p0.y - bounds.min_y) * scale_y).round() as isize;
            let x1 = ((p1.x - bounds.min_x) * scale_x).round() as isize;
            let y1 = ((p1.y - bounds.min_y) * scale_y).round() as isize;

            draw_line(x0, y0, x1, y1, columns, rows, options, &mut cells, path.color_style.as_deref());
        }
        
        // Handle closed paths
        if path.geometry.closed && pts.len() > 2 {
            let p0 = pts[pts.len() - 1];
            let p1 = pts[0];
            let x0 = ((p0.x - bounds.min_x) * scale_x).round() as isize;
            let y0 = ((p0.y - bounds.min_y) * scale_y).round() as isize;
            let x1 = ((p1.x - bounds.min_x) * scale_x).round() as isize;
            let y1 = ((p1.y - bounds.min_y) * scale_y).round() as isize;
            draw_line(x0, y0, x1, y1, columns, rows, options, &mut cells, path.color_style.as_deref());
        }
    }

    (columns, rows, cells)
}

fn draw_line(mut x0: isize, mut y0: isize, x1: isize, y1: isize, cols: usize, rows: usize, opts: &GridOptions, cells: &mut Vec<CellDescriptor>, color: Option<&str>) {
    let dx = (x1 - x0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;

    let sw = opts.subcell_width as isize;
    let sh = opts.subcell_height as isize;

    loop {
        if x0 >= 0 && y0 >= 0 {
            let cx = (x0 / sw) as usize;
            let cy = (y0 / sh) as usize;
            if cx < cols && cy < rows {
                let cell_idx = cy * cols + cx;
                let sub_x = (x0 % sw) as usize;
                let sub_y = (y0 % sh) as usize;
                
                // Set the bit in the cell mask
                let bit_idx = sub_y * opts.subcell_width as usize + sub_x;
                let word_idx = bit_idx / 64;
                let bit_offset = bit_idx % 64;
                
                cells[cell_idx].mask.words[word_idx] |= 1 << bit_offset;

                // Detect port hits (edges of the cell)
                if sub_y == 0 { cells[cell_idx].ports.insert(PortMask::N); }
                if sub_y == (sh - 1) as usize { cells[cell_idx].ports.insert(PortMask::S); }
                if sub_x == 0 { cells[cell_idx].ports.insert(PortMask::W); }
                if sub_x == (sw - 1) as usize { cells[cell_idx].ports.insert(PortMask::E); }
                
                if let Some(c) = color {
                    cells[cell_idx].color = Some(c.to_string());
                }
            }
        }

        if x0 == x1 && y0 == y1 { break; }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x0 += sx;
        }
        if e2 <= dx {
            err += dx;
            y0 += sy;
        }
    }
}
