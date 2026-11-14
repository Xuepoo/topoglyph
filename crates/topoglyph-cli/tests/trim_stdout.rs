use std::process::Command;

#[test]
fn actual_cli_trimmed_text_has_exactly_canvas_height_rows() {
    let fixture = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/stroke.png");
    for mode in ["rounded", "centerline", "match"] {
        for padding in ["0", "2"] {
            let args = [
                "render",
                fixture,
                "--render-mode",
                mode,
                "-W",
                "24",
                "-H",
                "12",
                "--trim",
                "--padding",
                padding,
                "--no-color",
            ];
            let plain = Command::new(env!("CARGO_BIN_EXE_topoglyph"))
                .args(args)
                .output()
                .unwrap();
            assert!(
                plain.status.success(),
                "{}",
                String::from_utf8_lossy(&plain.stderr)
            );
            let json = Command::new(env!("CARGO_BIN_EXE_topoglyph"))
                .args(args)
                .args(["--output-format", "json"])
                .output()
                .unwrap();
            assert!(json.status.success());
            let canvas: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
            let text = String::from_utf8(plain.stdout).unwrap();
            assert!(text.ends_with('\n'));
            assert_eq!(
                text.lines().count(),
                canvas["height"].as_u64().unwrap() as usize,
                "{mode} padding={padding}"
            );
        }
    }
}
