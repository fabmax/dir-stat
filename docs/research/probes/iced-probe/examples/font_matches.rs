//! Diagnostic: which faces does cosmic-text 0.15 (iced 0.14's text engine) put first for a Font?
//! Re-implements cosmic_text::FontSystem::get_font_matches (font/system.rs) because the match
//! key fields are private: candidates = all faces with matching style+stretch, sorted by
//! (|weight diff|, weight, id); the db.query(family) hit (if any) is moved to the front.
//! iced_graphics::text::font_system() uses FontSystem::new_with_fonts(..), which sets
//! sans-serif = "Open Sans", monospace = "Noto Sans Mono", serif = "DejaVu Serif".
use cosmic_text::fontdb::{Family, Query, Stretch, Style, Weight};
use cosmic_text::FontSystem;

fn main() {
    let fs = FontSystem::new();
    let db = fs.db();
    println!("faces loaded: {}", db.len());
    println!(
        "has 'Open Sans': {}",
        db.faces().any(|f| f.families.iter().any(|(n, _)| n == "Open Sans"))
    );

    let cases: &[(&str, Family<'_>, Weight)] = &[
        ("SansSerif Normal (Font::DEFAULT)", Family::SansSerif, Weight::NORMAL),
        ("SansSerif Semibold", Family::SansSerif, Weight::SEMIBOLD),
        ("SansSerif Bold", Family::SansSerif, Weight::BOLD),
        ("Monospace Normal (Font::MONOSPACE)", Family::Monospace, Weight::NORMAL),
        (".SF NS Bold", Family::Name(".SF NS"), Weight::BOLD),
        ("Helvetica Neue Bold", Family::Name("Helvetica Neue"), Weight::BOLD),
        ("Menlo Normal", Family::Name("Menlo"), Weight::NORMAL),
    ];

    for (label, family, weight) in cases {
        let query = Query {
            families: &[*family],
            weight: *weight,
            stretch: Stretch::Normal,
            style: Style::Normal,
        };
        let queried = db.query(&query);

        let mut keys: Vec<(u16, u16, _)> = db
            .faces()
            .filter(|f| f.post_script_name.contains("Emoji") || (f.style == Style::Normal && f.stretch == Stretch::Normal))
            .map(|f| (weight.0.abs_diff(f.weight.0), f.weight.0, f.id))
            .collect();
        keys.sort();
        if let Some(id) = queried {
            keys.retain(|k| k.2 != id);
            let face = db.face(id).unwrap();
            keys.insert(0, (weight.0.abs_diff(face.weight.0), face.weight.0, id));
        }

        let first = keys.first().and_then(|k| db.face(k.2)).map(|f| {
            format!("{} [{}] w{}", f.families[0].0, f.post_script_name, f.weight.0)
        });
        println!(
            "{label:36} query(family)={:<5} first match -> {}",
            queried.is_some(),
            first.unwrap_or_default()
        );
    }
}
