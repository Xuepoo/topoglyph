//! Deterministic line-art extraction. Region boundaries are constructed as
//! connected orthogonal curves rather than independently matched glyphs.
use image::{imageops::FilterType, GrayImage, ImageReader, Luma};
use std::{collections::HashSet, io::Cursor};
use topoglyph_core::canvas::{TextCanvas, TextCell};
use vectomancy_geometry::{BoundingBox, Point2D, Polyline, PolylineScene, StyledPath};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Foreground {
    /// Use alpha when transparency exists, otherwise dark ink on white.
    #[default]
    Auto,
    Dark,
    Light,
    Alpha,
}

#[derive(Clone, Copy, Debug)]
pub struct LineArtOptions {
    pub foreground: Foreground,
    pub threshold: u8,
    pub crop: bool,
    /// Rounded-mode grid component cleanup; zero preserves all regions.
    pub min_region_area: usize,
}
impl Default for LineArtOptions {
    fn default() -> Self {
        Self {
            foreground: Foreground::Auto,
            threshold: 128,
            crop: false,
            min_region_area: 0,
        }
    }
}

/// Decode coverage, respecting alpha rather than hidden RGB values. Explicit
/// dark/light modes composite onto white/black respectively before thresholding.
fn coverage(bytes: &[u8], opts: &LineArtOptions) -> Result<GrayImage, String> {
    if opts.threshold == 0 {
        return Err("threshold must be 1..255".into());
    }
    let reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let limits = image::Limits::default();
    let mut reader = reader;
    reader.limits(limits);
    let rgba = reader
        .decode()
        .map_err(|e| format!("Failed to decode line art: {e}"))?
        .to_rgba8();
    if u64::from(rgba.width()) * u64::from(rgba.height()) > 16_777_216 {
        return Err("line-art input exceeds 16 million pixels".into());
    }
    let alpha = opts.foreground == Foreground::Alpha
        || (opts.foreground == Foreground::Auto && rgba.pixels().any(|p| p[3] < 255));
    let mut out = GrayImage::new(rgba.width(), rgba.height());
    for (x, y, p) in rgba.enumerate_pixels() {
        let lum = (u32::from(p[0]) * 299 + u32::from(p[1]) * 587 + u32::from(p[2]) * 114) / 1000;
        let ink = if alpha {
            u32::from(p[3])
        } else if opts.foreground == Foreground::Light {
            lum * u32::from(p[3]) / 255
        } else {
            (255 - lum) * u32::from(p[3]) / 255
        };
        out.put_pixel(x, y, Luma([ink as u8]));
    }
    if opts.crop {
        let points: Vec<_> = out
            .enumerate_pixels()
            .filter(|(_, _, p)| p[0] >= opts.threshold && p[0] > 0)
            .map(|(x, y, _)| (x, y))
            .collect();
        if let Some(&(x, y)) = points.first() {
            let (mut minx, mut miny, mut maxx, mut maxy) = (x, y, x, y);
            for (x, y) in points {
                minx = minx.min(x);
                miny = miny.min(y);
                maxx = maxx.max(x);
                maxy = maxy.max(y);
            }
            out = image::imageops::crop_imm(&out, minx, miny, maxx - minx + 1, maxy - miny + 1)
                .to_image();
        }
    }
    Ok(out)
}

pub fn grid_size(
    width: Option<usize>,
    height: Option<usize>,
    source: (u32, u32),
) -> Result<(usize, usize), String> {
    let ratio = f64::from(source.0) / f64::from(source.1.max(1));
    let (w, h) = match (width, height) {
        (Some(w), Some(h)) => (w, h),
        (Some(w), None) => (w, (w as f64 / ratio / 2.).round().max(3.) as usize),
        (None, Some(h)) => ((h as f64 * ratio * 2.).round().max(3.) as usize, h),
        (None, None) => {
            let w = (source.0 as usize).min(120);
            (w, (w as f64 / ratio / 2.).round().clamp(3., 60.) as usize)
        }
    };
    if w < 3 || h < 3 || w > 4096 || h > 4096 || w.checked_mul(h).is_none_or(|n| n > 4_194_304) {
        return Err("line-art grid must be 3..4096 per axis and at most 4 million cells".into());
    }
    Ok((w, h))
}

/// Remove tiny foreground components and fill tiny enclosed background holes.
/// Four-connected components touching the exterior are never filled.
fn clean_regions(mask: &mut [bool], w: usize, h: usize, minimum: usize, remove_specks: bool) {
    if minimum == 0 {
        return;
    }
    for foreground in [true, false] {
        if foreground && !remove_specks {
            continue;
        }
        let mut seen = vec![false; mask.len()];
        for start in 0..mask.len() {
            if seen[start] || mask[start] != foreground {
                continue;
            }
            let mut component = vec![start];
            seen[start] = true;
            let mut cursor = 0;
            let mut exterior = false;
            while cursor < component.len() {
                let i = component[cursor];
                cursor += 1;
                let (x, y) = (i % w, i / w);
                exterior |= x == 0 || y == 0 || x + 1 == w || y + 1 == h;
                for j in [
                    if x > 0 { Some(i - 1) } else { None },
                    if x + 1 < w { Some(i + 1) } else { None },
                    if y > 0 { Some(i - w) } else { None },
                    if y + 1 < h { Some(i + w) } else { None },
                ]
                .into_iter()
                .flatten()
                {
                    if !seen[j] && mask[j] == foreground {
                        seen[j] = true;
                        component.push(j);
                    }
                }
            }
            if component.len() < minimum && (foreground || !exterior) {
                for i in component {
                    mask[i] = !foreground;
                }
            }
        }
    }
}

/// Rounded boundary of a thresholded coverage field. Ambiguous diagonal
/// contacts are merged locally before mapping, so every output port has a
/// matching neighbor and no slash or branch glyph is necessary.
pub fn rounded_canvas(
    bytes: &[u8],
    options: &LineArtOptions,
    width: Option<usize>,
    height: Option<usize>,
) -> Result<TextCanvas, String> {
    let image = coverage(bytes, options)?;
    let (w, h) = grid_size(width, height, image.dimensions())?;
    let resized =
        image::imageops::resize(&image, (w - 1) as u32, (h - 1) as u32, FilterType::Triangle);
    let stride = w + 1;
    let mut mask = vec![false; (w + 1) * (h + 1)];
    for (x, y, p) in resized.enumerate_pixels() {
        mask[(y as usize + 1) * stride + x as usize + 1] = p[0] >= options.threshold && p[0] > 0;
    }
    if options.min_region_area > 4_194_304 {
        return Err("min-region-area exceeds 4 million cells".into());
    }
    clean_regions(&mut mask, w + 1, h + 1, options.min_region_area, true);
    // Monotonic additions terminate. Each modification fills a saddle's
    // missing quadrant; repeated passes handle interactions between saddles.
    loop {
        let mut changed = false;
        for y in 0..h {
            for x in 0..w {
                let i = y * stride + x;
                let (a, b, c, d) = (mask[i], mask[i + 1], mask[i + stride + 1], mask[i + stride]);
                if a == c && b == d && a != b {
                    // Never fill the padding: choose an interior empty quadrant.
                    for j in [i, i + 1, i + stride + 1, i + stride] {
                        let (xx, yy) = (j % stride, j / stride);
                        if !mask[j] && xx > 0 && xx < w && yy > 0 && yy < h {
                            mask[j] = true;
                            changed = true;
                            break;
                        }
                    }
                }
            }
        }
        if !changed && options.min_region_area > 0 {
            let before = mask.clone();
            // Saddle merging can enclose a new tiny hole. Fill those too,
            // then resolve any diagonal contacts created by the filling.
            clean_regions(&mut mask, w + 1, h + 1, options.min_region_area, false);
            changed = mask != before;
        }
        if !changed {
            break;
        }
    }
    let chars = [
        ' ', '╯', '╰', '─', '╭', ' ', '│', '╮', '╮', '│', ' ', '╭', '─', '╰', '╯', ' ',
    ];
    let mut cells = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            let i = y * stride + x;
            let bits = usize::from(mask[i])
                + usize::from(mask[i + 1]) * 2
                + usize::from(mask[i + stride + 1]) * 4
                + usize::from(mask[i + stride]) * 8;
            cells.push(TextCell {
                token: chars[bits].to_string(),
                score: 0.,
                source_path: None,
                color: Some("#CBDCEB".into()),
            });
        }
    }
    Ok(TextCanvas {
        width: w,
        height: h,
        cells,
    })
}

/// Foreground skeletonization avoids Sobel's two boundaries around thick ink.
/// Paths remain geometry and can use the existing atlas/matching pipeline.
pub fn centerline_scene(bytes: &[u8], options: &LineArtOptions) -> Result<PolylineScene, String> {
    let image = coverage(bytes, options)?;
    let (w, h) = (image.width() as usize + 2, image.height() as usize + 2);
    let mut ink = vec![false; w * h];
    for (x, y, p) in image.enumerate_pixels() {
        ink[(y as usize + 1) * w + x as usize + 1] = p[0] >= options.threshold && p[0] > 0;
    }
    loop {
        let mut changed = false;
        for pass in 0..2 {
            let mut remove = Vec::new();
            for y in 1..h - 1 {
                for x in 1..w - 1 {
                    let i = y * w + x;
                    if !ink[i] {
                        continue;
                    }
                    let p = [
                        ink[i - w],
                        ink[i - w + 1],
                        ink[i + 1],
                        ink[i + w + 1],
                        ink[i + w],
                        ink[i + w - 1],
                        ink[i - 1],
                        ink[i - w - 1],
                    ];
                    let n = p.iter().filter(|&&v| v).count();
                    let transitions = (0..8).filter(|&j| !p[j] && p[(j + 1) % 8]).count();
                    let clear = if pass == 0 {
                        !(p[0] && p[2] && p[4] || p[2] && p[4] && p[6])
                    } else {
                        !(p[0] && p[2] && p[6] || p[0] && p[4] && p[6])
                    };
                    if (2..=6).contains(&n) && transitions == 1 && clear {
                        remove.push(i);
                    }
                }
            }
            changed |= !remove.is_empty();
            for i in remove {
                ink[i] = false;
            }
        }
        if !changed {
            break;
        }
    }
    let neighbors = |i: usize| -> Vec<usize> {
        let (x, y) = (i % w, i / w);
        let mut out = Vec::new();
        for dy in -1isize..=1 {
            for dx in -1isize..=1 {
                if dx == 0 && dy == 0 {
                    continue;
                }
                let (xx, yy) = (x as isize + dx, y as isize + dy);
                if xx < 0 || yy < 0 || xx >= w as isize || yy >= h as isize {
                    continue;
                }
                let j = yy as usize * w + xx as usize;
                // Prefer cardinal links to redundant diagonal shortcuts at corners.
                if ink[j]
                    && !(dx != 0
                        && dy != 0
                        && (ink[y * w + xx as usize] || ink[yy as usize * w + x]))
                {
                    out.push(j);
                }
            }
        }
        out
    };
    let mut used = HashSet::new();
    let edge = |a: usize, b: usize| if a < b { (a, b) } else { (b, a) };
    let point = |i: usize| Point2D::new((i % w) as f64 - 1., (i / w) as f64 - 1.);
    let mut paths = Vec::new();
    for terminals in [true, false] {
        for (start, &is_ink) in ink.iter().enumerate() {
            if !is_ink {
                continue;
            }
            let ns = neighbors(start);
            if terminals && ns.len() == 2 {
                continue;
            }
            for next in ns {
                if used.contains(&edge(start, next)) {
                    continue;
                }
                let mut points = vec![point(start)];
                let (mut prev, mut cur) = (start, next);
                used.insert(edge(prev, cur));
                loop {
                    points.push(point(cur));
                    let ns = neighbors(cur);
                    if cur == start || ns.len() != 2 {
                        break;
                    }
                    let next = *ns.iter().find(|&&n| n != prev).unwrap();
                    if !used.insert(edge(cur, next)) {
                        break;
                    }
                    prev = cur;
                    cur = next;
                }
                paths.push(StyledPath {
                    geometry: Polyline {
                        points,
                        closed: cur == start,
                    },
                    color_style: Some("#CBDCEB".into()),
                });
            }
        }
    }
    let points: Vec<_> = paths
        .iter()
        .flat_map(|p| p.geometry.points.iter().copied())
        .collect();
    Ok(PolylineScene {
        paths,
        dimensions: image.dimensions(),
        bounds: BoundingBox::from_points(&points),
    })
}

/// Crop unused output cells, with optional uniform cell padding. Empty input
/// becomes a single blank cell (plus padding), never a zero-sized canvas.
pub fn trim_canvas(canvas: &TextCanvas, padding: usize) -> Result<TextCanvas, String> {
    if canvas.width == 0
        || canvas.height == 0
        || canvas.width.checked_mul(canvas.height) != Some(canvas.cells.len())
    {
        return Err("invalid canvas dimensions".into());
    }
    if padding > 1024 {
        return Err("padding must be at most 1024 cells".into());
    }
    let occupied: Vec<_> = canvas
        .cells
        .iter()
        .enumerate()
        .filter(|(_, c)| !c.token.trim().is_empty())
        .map(|(i, _)| (i % canvas.width, i / canvas.width))
        .collect();
    let (minx, miny, maxx, maxy) = if occupied.is_empty() {
        (0, 0, 0, 0)
    } else {
        (
            occupied.iter().map(|p| p.0).min().unwrap(),
            occupied.iter().map(|p| p.1).min().unwrap(),
            occupied.iter().map(|p| p.0).max().unwrap(),
            occupied.iter().map(|p| p.1).max().unwrap(),
        )
    };
    let (w, h) = (maxx - minx + 1 + 2 * padding, maxy - miny + 1 + 2 * padding);
    if w.checked_mul(h).is_none_or(|n| n > 4_194_304) {
        return Err("padded canvas exceeds 4 million cells".into());
    }
    let blank = TextCell {
        token: " ".into(),
        score: 0.,
        source_path: None,
        color: None,
    };
    let mut cells = vec![blank; w * h];
    if !occupied.is_empty() {
        for y in miny..=maxy {
            for x in minx..=maxx {
                cells[(y - miny + padding) * w + x - minx + padding] =
                    canvas.cells[y * canvas.width + x].clone();
            }
        }
    }
    Ok(TextCanvas {
        width: w,
        height: h,
        cells,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn png(image: image::DynamicImage) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        image.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }
    fn ports(token: &str) -> [bool; 4] {
        // north, east, south, west
        match token {
            "─" => [false, true, false, true],
            "│" => [true, false, true, false],
            "╭" => [false, true, true, false],
            "╮" => [false, false, true, true],
            "╰" => [true, true, false, false],
            "╯" => [true, false, false, true],
            " " => [false; 4],
            _ => panic!("unexpected glyph"),
        }
    }
    fn assert_connected(c: &TextCanvas) {
        for y in 0..c.height {
            for x in 0..c.width {
                let p = ports(&c.cells[y * c.width + x].token);
                for (direction, (dx, dy)) in
                    [(0, -1), (1, 0), (0, 1), (-1, 0)].into_iter().enumerate()
                {
                    let (xx, yy) = (x as isize + dx, y as isize + dy);
                    let other =
                        if xx < 0 || yy < 0 || xx >= c.width as isize || yy >= c.height as isize {
                            false
                        } else {
                            ports(&c.cells[yy as usize * c.width + xx as usize].token)
                                [(direction + 2) % 4]
                        };
                    assert_eq!(p[direction], other, "dangling port at {x},{y}");
                }
            }
        }
    }
    #[test]
    fn all_three_by_three_regions_have_closed_reciprocal_contours() {
        for bits in 0..512 {
            let mut im = GrayImage::from_pixel(3, 3, Luma([255]));
            for i in 0..9 {
                if bits & (1 << i) != 0 {
                    im.put_pixel(i % 3, i / 3, Luma([0]));
                }
            }
            let c = rounded_canvas(
                &png(im.into()),
                &LineArtOptions::default(),
                Some(4),
                Some(4),
            )
            .unwrap();
            assert_connected(&c);
        }
    }
    #[test]
    fn alpha_logo_uses_coverage_and_ignores_hidden_rgb() {
        let mut im = image::RgbaImage::from_pixel(8, 8, image::Rgba([0, 0, 0, 0]));
        for y in 2..6 {
            for x in 2..6 {
                im.put_pixel(x, y, image::Rgba([203, 220, 235, 255]));
            }
        }
        let c = rounded_canvas(
            &png(im.into()),
            &LineArtOptions::default(),
            Some(9),
            Some(9),
        )
        .unwrap();
        assert_connected(&c);
        assert!(c.cells.iter().any(|c| c.token != " "));
        assert_eq!(c.cells[0].token, " ");
    }
    #[test]
    fn thick_stroke_has_one_centerline_not_two_edges() {
        let mut im = GrayImage::from_pixel(40, 24, Luma([255]));
        for y in 8..16 {
            for x in 5..35 {
                im.put_pixel(x, y, Luma([0]));
            }
        }
        let scene = centerline_scene(&png(im.into()), &LineArtOptions::default()).unwrap();
        assert_eq!(scene.paths.len(), 1);
        let points = &scene.paths[0].geometry.points;
        assert!(points.len() > 15);
        let miny = points.iter().map(|p| p.y).fold(f64::INFINITY, f64::min);
        let maxy = points.iter().map(|p| p.y).fold(f64::NEG_INFINITY, f64::max);
        assert!(
            maxy - miny <= 1.,
            "thick stroke did not collapse to a centerline"
        );
    }
    #[test]
    fn blank_input_and_trim_are_well_defined() {
        let bytes = png(GrayImage::from_pixel(8, 8, Luma([255])).into());
        let c = rounded_canvas(&bytes, &LineArtOptions::default(), Some(10), Some(5)).unwrap();
        assert!(c.cells.iter().all(|c| c.token == " "));
        let trimmed = trim_canvas(&c, 2).unwrap();
        assert_eq!((trimmed.width, trimmed.height), (5, 5));
        assert!(centerline_scene(&bytes, &LineArtOptions::default())
            .unwrap()
            .paths
            .is_empty());
    }
    #[test]
    fn cleanup_removes_specks_and_pinholes_but_preserves_eye_holes() {
        let (w, h) = (12, 12);
        let mut mask = vec![false; w * h];
        for y in 2..10 {
            for x in 2..10 {
                mask[y * w + x] = true;
            }
        }
        mask[0] = true; // isolated foreground speck
        mask[3 * w + 3] = false; // single-cell enclosed pinhole
        for y in 5..8 {
            for x in 5..8 {
                mask[y * w + x] = false;
            }
        } // real eye hole
        clean_regions(&mut mask, w, h, 3, true);
        assert!(!mask[0]);
        assert!(mask[3 * w + 3]);
        assert!(!mask[6 * w + 6]);
        assert!(!mask[1]); // exterior stays empty
    }

    #[test]
    fn crop_preserves_aspect_and_limits_reject_bad_allocations() {
        assert!(grid_size(Some(0), Some(5), (10, 10)).is_err());
        assert!(grid_size(Some(4096), Some(4096), (10, 10)).is_err());
        assert_eq!(grid_size(Some(120), None, (100, 100)).unwrap(), (120, 60));
        let malformed = TextCanvas {
            width: 0,
            height: 0,
            cells: vec![],
        };
        assert!(trim_canvas(&malformed, 0).is_err());
    }
}
