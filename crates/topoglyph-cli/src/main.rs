use topoglyph::input::adapter;
use topoglyph::core::geometry::GridOptions;
use topoglyph::core::clipping;
use topoglyph::core::matching;
use topoglyph::atlas::atlas::{GlyphAtlas, AtlasOptions};
use topoglyph::output::encoder::{TextEncoder, PlainTextEncoder, AnsiEncoder};
use std::env;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: topoglyph render <image.png>");
        std::process::exit(1);
    }

    let filepath = &args[1];
    
    // 1. Read input bytes
    let bytes = std::fs::read(filepath).expect("Failed to read file");
    
    // 2. Decode to PolylineScene
    let scene = adapter::raster_to_scene(&bytes, true).expect("Failed to decode image");
    
    // 3. Setup Subcell grid clipping
    let grid_opts = GridOptions {
        columns: 120, // fixed columns for now
        ..Default::default()
    };
    let (cols, rows, cell_descriptors) = clipping::process_scene(&scene, &grid_opts);
    
    // 4. Generate built-in GlyphAtlas
    let atlas = GlyphAtlas::from_text("", &AtlasOptions::default()).unwrap();
    
    // 5. Match
    let canvas = matching::match_scene(cols, rows, &cell_descriptors, &atlas.glyphs);
    
    // 6. Encode and output
    let encoder = AnsiEncoder::new();
    let out = encoder.encode(&canvas).unwrap();
    
    let text = String::from_utf8(out).unwrap();
    println!("{}", text);
}
