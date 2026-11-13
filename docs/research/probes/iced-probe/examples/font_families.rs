//! List family names / weights of a few macOS system fonts as fontdb (iced's font db) sees them.
use cosmic_text::FontSystem;

fn main() {
    let fs = FontSystem::new();
    let wanted = ["SFNS", "SFNSMono", "SFNSRounded", "HelveticaNeue", "AvenirNext-", "Menlo", "Inter", "Fira"];
    let mut rows: Vec<String> = fs
        .db()
        .faces()
        .filter(|f| wanted.iter().any(|w| f.post_script_name.starts_with(w)))
        .map(|f| {
            format!(
                "{:28} family={:?} weight={} style={:?} mono={}",
                f.post_script_name,
                f.families.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(),
                f.weight.0,
                f.style,
                f.monospaced
            )
        })
        .collect();
    rows.sort();
    rows.dedup();
    for row in rows {
        println!("{row}");
    }
}
