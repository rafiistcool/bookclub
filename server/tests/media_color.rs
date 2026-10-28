//! The spine colour port against the legacy implementation (#43, docs/phase1/spec/shelf.md §6
//! risk 17): `tests/fixtures/images/pillow/` holds random covers and the colour the Python
//! `spine_color.dominant_color` (Pillow's median cut) picked for each (`make_covers.py`).
//! The hand-written median cut picks the same colour for those marked `exact`; the two marked
//! `differs` split one near-tie box differently, which the spec allows (only the qualitative
//! properties are contractual).

use bookclub_server::media::color::dominant_color;

#[test]
fn the_port_picks_the_legacy_colour() {
    let dir = format!(
        "{}/tests/fixtures/images/pillow",
        env!("CARGO_MANIFEST_DIR")
    );
    let expected = std::fs::read_to_string(format!("{dir}/expected.txt")).unwrap();
    let mut exact = 0;
    for line in expected.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        let [file, pillow, verdict] = fields[..] else {
            panic!("malformed line {line:?}");
        };
        let data = std::fs::read(format!("{dir}/{file}")).unwrap();
        let ours = dominant_color(&data).unwrap_or_else(|| panic!("{file}: no colour"));
        match verdict {
            "exact" => {
                assert_eq!(ours, pillow, "{file}");
                exact += 1;
            }
            "differs" => assert_ne!(ours, pillow, "{file} matches now: mark it exact"),
            other => panic!("unknown verdict {other}"),
        }
    }
    assert_eq!(exact, 10);
}
