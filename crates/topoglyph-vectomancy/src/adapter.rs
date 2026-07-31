use vectomancy_geometry::{
    chaikin_smooth, simplify_rdp, BoundingBox, Point2D, Polyline, PolylineScene, StyledPath,
};

/// Post-processing options applied to raw skeleton paths before they are
/// handed to the grid-clipping stage. Mirrors the `tolerance` /
/// `chaikin_iters` knobs already exposed by the Vectomancy CLI so both
/// engines can be tuned the same way (see
/// `vectomancy-docs/parameter_tuning_guide.md`).
#[derive(Debug, Clone, Copy)]
pub struct SmoothingOptions {
    /// RDP simplification tolerance. `0.0` disables simplification.
    pub tolerance: f64,
    /// Number of Chaikin corner-cutting iterations. `0` disables smoothing.
    pub chaikin_iters: usize,
}

impl Default for SmoothingOptions {
    fn default() -> Self {
        Self {
            tolerance: 0.5,
            chaikin_iters: 1,
        }
    }
}

/// Applies RDP simplification followed by Chaikin smoothing to every path in
/// a scene.
///
/// Raw skeleton extraction (Zhang-Suen thinning, see `vectomancy-raster`)
/// produces single-pixel-wide polylines that still carry pixel-grid jitter:
/// nominally straight or gently curved strokes zig-zag by a pixel at every
/// step. Feeding that directly into subcell rasterization (see
/// `topoglyph_core::clipping`) means every jitter step becomes its own
/// `CellMask` bit pattern, which fragments what should be one continuous
/// glyph match into many noisy, disconnected ones.
///
/// RDP removes the collinear jitter points first (cheaply, since Chaikin's
/// output size grows geometrically with iteration count), then Chaikin
/// rounds the remaining corners into smooth curves — matching the
/// `Raster -> contour extraction -> RDP+Chaikin -> grid` pipeline described
/// in `topoglyph-docs/technical.md` section 1.3.
pub fn smooth_scene(scene: &PolylineScene, options: &SmoothingOptions) -> PolylineScene {
    let paths: Vec<StyledPath<Polyline>> = scene
        .paths
        .iter()
        .map(|path| smooth_path(path, options))
        .collect();

    let points: Vec<Point2D> = paths
        .iter()
        .flat_map(|path| path.geometry.points.iter().copied())
        .collect();

    PolylineScene {
        paths,
        dimensions: scene.dimensions,
        bounds: BoundingBox::from_points(&points),
    }
}

fn smooth_path(path: &StyledPath<Polyline>, options: &SmoothingOptions) -> StyledPath<Polyline> {
    let mut points = path.geometry.points.clone();

    if options.tolerance > 0.0 {
        points = simplify_rdp(&points, options.tolerance);
    }

    let mut polyline = Polyline {
        points,
        closed: path.geometry.closed,
    };

    if options.chaikin_iters > 0 {
        polyline = chaikin_smooth(&polyline, options.chaikin_iters);
    }

    StyledPath {
        geometry: polyline,
        color_style: path.color_style.clone(),
    }
}

/// Inverts every path's sampled color (`#rrggbb` -> `#(255-r)(255-g)(255-b)`),
/// implementing the `--invert` CLI flag (see `topoglyph-docs/TODO.md`
/// 0.2.0). Paths with no sampled color (e.g. `--charset` runs without color
/// sampling) are left untouched — there's nothing to invert.
///
/// This operates on the scene after skeleton extraction/smoothing rather
/// than on the source image, so it composes with every input path
/// (raster file, mock JSON scene) instead of needing its own image
/// pre-processing step.
pub fn invert_scene_colors(scene: &PolylineScene) -> PolylineScene {
    let paths: Vec<StyledPath<Polyline>> = scene
        .paths
        .iter()
        .map(|path| StyledPath {
            geometry: path.geometry.clone(),
            color_style: path
                .color_style
                .as_deref()
                .and_then(invert_hex_color)
                .or_else(|| path.color_style.clone()),
        })
        .collect();

    PolylineScene {
        paths,
        dimensions: scene.dimensions,
        bounds: scene.bounds,
    }
}

/// Parses a `#rrggbb` hex color and returns its inverted `#rrggbb` string.
/// Returns `None` for malformed input (missing/short hex digits), in which
/// case the caller keeps the original, unaltered string rather than
/// silently dropping the color.
fn invert_hex_color(hex: &str) -> Option<String> {
    let hex = hex.strip_prefix('#')?;
    if hex.len() < 6 {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(format!("#{:02x}{:02x}{:02x}", 255 - r, 255 - g, 255 - b))
}

pub fn raster_to_scene(bytes: &[u8], color: bool) -> Result<PolylineScene, String> {
    let (paths, dimensions) = vectomancy_raster::decode_raster_memory(bytes, color)
        .map_err(|e| format!("Raster error: {:?}", e))?;

    let points = paths
        .iter()
        .flat_map(|path| path.geometry.points.iter().copied())
        .collect::<Vec<_>>();

    Ok(PolylineScene {
        paths,
        dimensions,
        bounds: BoundingBox::from_points(&points),
    })
}

/// Decodes a raster image directly into a smoothed [`PolylineScene`], ready
/// for grid clipping. This is the path the CLI should use; [`raster_to_scene`]
/// remains available for callers that want the raw, unsmoothed skeleton
/// (e.g. tests comparing before/after smoothing).
pub fn raster_to_smoothed_scene(
    bytes: &[u8],
    color: bool,
    options: &SmoothingOptions,
) -> Result<PolylineScene, String> {
    let scene = raster_to_scene(bytes, color)?;
    Ok(smooth_scene(&scene, options))
}

pub fn json_to_scene(json: &str) -> Result<PolylineScene, String> {
    serde_json::from_str(json).map_err(|e| format!("JSON parse error: {:?}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn styled(points: Vec<Point2D>, closed: bool) -> StyledPath<Polyline> {
        StyledPath {
            geometry: Polyline { points, closed },
            color_style: None,
        }
    }

    #[test]
    fn smoothing_removes_collinear_jitter() {
        // Simulates a single-pixel-wide horizontal skeleton stroke with
        // +/-1px stairstep jitter, the kind Zhang-Suen thinning produces on
        // a near-horizontal raster line.
        let jittered = vec![
            Point2D::new(0.0, 0.0),
            Point2D::new(1.0, 1.0),
            Point2D::new(2.0, 0.0),
            Point2D::new(3.0, 1.0),
            Point2D::new(4.0, 0.0),
            Point2D::new(5.0, 1.0),
            Point2D::new(6.0, 0.0),
        ];
        let scene = PolylineScene {
            paths: vec![styled(jittered, false)],
            dimensions: (10, 10),
            bounds: BoundingBox::new(0.0, 0.0, 6.0, 1.0),
        };

        let smoothed = smooth_scene(
            &scene,
            &SmoothingOptions {
                tolerance: 1.5,
                chaikin_iters: 0,
            },
        );

        assert!(
            smoothed.paths[0].geometry.points.len() < scene.paths[0].geometry.points.len(),
            "RDP should collapse near-collinear jitter into fewer points"
        );
    }

    #[test]
    fn chaikin_rounds_sharp_corners() {
        let corner = vec![
            Point2D::new(0.0, 0.0),
            Point2D::new(4.0, 0.0),
            Point2D::new(4.0, 4.0),
        ];
        let scene = PolylineScene {
            paths: vec![styled(corner, false)],
            dimensions: (10, 10),
            bounds: BoundingBox::new(0.0, 0.0, 4.0, 4.0),
        };

        let smoothed = smooth_scene(
            &scene,
            &SmoothingOptions {
                tolerance: 0.0,
                chaikin_iters: 1,
            },
        );

        assert!(
            smoothed.paths[0].geometry.points.len() > scene.paths[0].geometry.points.len(),
            "Chaikin should subdivide the sharp corner into more points"
        );
    }

    #[test]
    fn zero_options_are_a_no_op() {
        let straight = vec![
            Point2D::new(0.0, 0.0),
            Point2D::new(1.0, 0.5),
            Point2D::new(2.0, 0.0),
        ];
        let scene = PolylineScene {
            paths: vec![styled(straight.clone(), false)],
            dimensions: (10, 10),
            bounds: BoundingBox::new(0.0, 0.0, 2.0, 0.5),
        };

        let smoothed = smooth_scene(
            &scene,
            &SmoothingOptions {
                tolerance: 0.0,
                chaikin_iters: 0,
            },
        );

        assert_eq!(smoothed.paths[0].geometry.points, straight);
    }

    #[test]
    fn empty_scene_does_not_panic() {
        let scene = PolylineScene {
            paths: vec![],
            dimensions: (10, 10),
            bounds: BoundingBox::new(0.0, 0.0, 0.0, 0.0),
        };
        let smoothed = smooth_scene(&scene, &SmoothingOptions::default());
        assert!(smoothed.paths.is_empty());
    }

    #[test]
    fn closed_path_stays_closed_after_smoothing() {
        let square = vec![
            Point2D::new(0.0, 0.0),
            Point2D::new(4.0, 0.0),
            Point2D::new(4.0, 4.0),
            Point2D::new(0.0, 4.0),
        ];
        let scene = PolylineScene {
            paths: vec![styled(square, true)],
            dimensions: (10, 10),
            bounds: BoundingBox::new(0.0, 0.0, 4.0, 4.0),
        };

        let smoothed = smooth_scene(
            &scene,
            &SmoothingOptions {
                tolerance: 0.0,
                chaikin_iters: 1,
            },
        );

        assert!(smoothed.paths[0].geometry.closed);
    }

    #[test]
    fn json_to_scene_round_trips_mock_input() {
        // Exercises the serde-based mock input path called out in
        // `topoglyph-docs/technical.md` section 3, which lets geometry logic
        // be unit-tested without a real image decode step.
        let scene = PolylineScene {
            paths: vec![styled(
                vec![Point2D::new(0.0, 0.0), Point2D::new(1.0, 1.0)],
                false,
            )],
            dimensions: (2, 2),
            bounds: BoundingBox::new(0.0, 0.0, 1.0, 1.0),
        };
        let json = serde_json::to_string(&scene).expect("scene should serialize");
        let parsed = json_to_scene(&json).expect("scene should round-trip through JSON");
        assert_eq!(parsed.dimensions, scene.dimensions);
        assert_eq!(parsed.paths.len(), scene.paths.len());
    }

    #[test]
    fn json_to_scene_rejects_malformed_input() {
        assert!(json_to_scene("not valid json").is_err());
    }

    fn styled_colored(color: &str) -> StyledPath<Polyline> {
        StyledPath {
            geometry: Polyline {
                points: vec![Point2D::new(0.0, 0.0), Point2D::new(1.0, 1.0)],
                closed: false,
            },
            color_style: Some(color.to_string()),
        }
    }

    #[test]
    fn invert_scene_colors_inverts_every_channel() {
        let scene = PolylineScene {
            paths: vec![styled_colored("#ff8000")],
            dimensions: (2, 2),
            bounds: BoundingBox::new(0.0, 0.0, 1.0, 1.0),
        };
        let inverted = invert_scene_colors(&scene);
        // 0xff -> 0x00, 0x80 -> 0x7f, 0x00 -> 0xff
        assert_eq!(inverted.paths[0].color_style.as_deref(), Some("#007fff"));
    }

    #[test]
    fn invert_scene_colors_round_trips_black_and_white() {
        let scene = PolylineScene {
            paths: vec![styled_colored("#000000")],
            dimensions: (2, 2),
            bounds: BoundingBox::new(0.0, 0.0, 1.0, 1.0),
        };
        let inverted = invert_scene_colors(&scene);
        assert_eq!(inverted.paths[0].color_style.as_deref(), Some("#ffffff"));

        let inverted_twice = invert_scene_colors(&inverted);
        assert_eq!(
            inverted_twice.paths[0].color_style.as_deref(),
            Some("#000000")
        );
    }

    #[test]
    fn invert_scene_colors_leaves_uncolored_paths_untouched() {
        let scene = PolylineScene {
            paths: vec![styled(
                vec![Point2D::new(0.0, 0.0), Point2D::new(1.0, 1.0)],
                false,
            )],
            dimensions: (2, 2),
            bounds: BoundingBox::new(0.0, 0.0, 1.0, 1.0),
        };
        let inverted = invert_scene_colors(&scene);
        assert_eq!(inverted.paths[0].color_style, None);
    }

    #[test]
    fn invert_scene_colors_keeps_malformed_color_unchanged() {
        let scene = PolylineScene {
            paths: vec![styled_colored("not-a-color")],
            dimensions: (2, 2),
            bounds: BoundingBox::new(0.0, 0.0, 1.0, 1.0),
        };
        let inverted = invert_scene_colors(&scene);
        assert_eq!(
            inverted.paths[0].color_style.as_deref(),
            Some("not-a-color")
        );
    }
}
