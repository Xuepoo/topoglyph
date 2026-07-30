use vectomancy_geometry::{PolylineScene, BoundingBox};

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

pub fn json_to_scene(json: &str) -> Result<PolylineScene, String> {
    serde_json::from_str(json).map_err(|e| format!("JSON parse error: {:?}", e))
}
