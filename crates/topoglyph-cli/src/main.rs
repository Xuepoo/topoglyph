use topoglyph::input::adapter;
use topoglyph::core::geometry::GridOptions;
use topoglyph::core::clipping;
use topoglyph::core::matching;
use topoglyph::atlas::atlas::{GlyphAtlas, AtlasOptions};
use topoglyph::output::encoder::{TextEncoder, PlainTextEncoder, AnsiEncoder};
use clap::Parser;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Input file path (image)
    input: String,

    /// Width of the output text grid
    #[arg(short = 'W', long, default_value_t = 160)]
    width: usize,

    /// Height of the output text grid
    #[arg(short = 'H', long, default_value_t = 80)]
    height: usize,
    
    /// Enable plain text mode (no colors)
    #[arg(long, default_value_t = false)]
    no_color: bool,
}

fn main() {
    let args = Args::parse();
    
    // 1. Read Input
    let bytes = std::fs::read(&args.input).expect("Failed to read file");
    
    // 2. Decode to PolylineScene
    let scene = adapter::raster_to_scene(&bytes, true).expect("Failed to decode image");
    
    // 3. Setup Subcell grid clipping
    let grid_opts = GridOptions {
        columns: args.width,
        rows: Some(args.height),
        ..Default::default()
    };
    let (out_cols, out_rows, cell_descriptors) = clipping::process_scene(&scene, &grid_opts);
    
    // 4. Generate built-in GlyphAtlas
    let atlas = GlyphAtlas::from_text("", &AtlasOptions::default()).unwrap();
    
    // 5. Match glyphs
    let canvas = matching::match_scene(out_cols, out_rows, &cell_descriptors, &atlas.glyphs);
    
    // 6. Encode and output
    let out = if args.no_color {
        let encoder = PlainTextEncoder::new();
        encoder.encode(&canvas).unwrap()
    } else {
        let encoder = AnsiEncoder::new();
        encoder.encode(&canvas).unwrap()
    };
    
    let text = String::from_utf8(out).unwrap();
    println!("{}", text);
}
