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
    
    /// Charset to use: 'lines', 'ascii', 'blocks', 'braille', 'custom'
    #[arg(short = 'C', long, default_value = "lines")]
    charset: String,

    /// Custom characters to use when charset is 'custom'
    #[arg(long, default_value = "")]
    custom_chars: String,

    /// Path to TTF/OTF font file (required for rasterization)
    #[arg(long)]
    font: Option<String>,
    
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
    
    // 4. Generate GlyphAtlas
    let atlas = if args.charset == "lines" {
        GlyphAtlas::from_text("", &AtlasOptions::default()).unwrap()
    } else {
        let chars = if args.charset == "custom" {
            args.custom_chars.clone()
        } else {
            GlyphAtlas::get_charset_string(&args.charset)
                .expect("Invalid charset specified")
                .to_string()
        };
        
        let font_path = args.font.expect("A --font must be provided for text rasterization");
        let font_bytes = std::fs::read(&font_path).expect("Failed to read font file");
        GlyphAtlas::from_custom_font(&chars, &font_bytes, &AtlasOptions::default()).unwrap()
    };
    
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
