//! Pantone minimalist light UI (spec §22, design.md). Lightweight ANSI styling — no heavy
//! TUI framework. Tokens mirror `docs/design.md`: Still Blue accent, Pavement ink, Polar
//! White background, soft Cloud Grey for muted text.

pub const RESET: &str = "\x1b[0m";
pub const BOLD: &str = "\x1b[1m";

// Pantone 14-4318 "Still Blue" accent.
pub const ACCENT: &str = "\x1b[38;2;124;166;180m";
// Pantone 19-3911 "Pavement" ink.
pub const INK: &str = "\x1b[38;2;58;58;60m";
// Pantone 14-4301 "Cloud Grey" muted.
pub const MUTED: &str = "\x1b[38;2;140;146;150m";
// Pantone 16-1344 "Marigold" for warnings/ask.
pub const WARN: &str = "\x1b[38;2;232;169;76m";
// Pantone 19-1664 "Red Maple" for errors.
pub const ERR: &str = "\x1b[38;2;188;72;74m";

pub fn banner(_title: &str) {
    let lead = format!("{}  æ  {}{}AETHER{}", ACCENT, BOLD, INK, RESET);
    let tail = format!("{}   rust-native coding agent   {}", MUTED, ACCENT);
    let visible = "  æ  AETHER   rust-native coding agent   ".chars().count();
    let bar = "─".repeat(visible);
    println!("{}╭{}╮{}", ACCENT, bar, RESET);
    println!("{}│{}{}{}│{}", ACCENT, lead, tail, ACCENT, RESET);
    println!("{}╰{}╯{}", ACCENT, bar, RESET);
}

pub fn section(title: &str, body: &str) {
    println!("{}{}{}{}", ACCENT, title, RESET, body);
}

pub fn note(msg: &str) {
    println!("{}•{} {}", MUTED, RESET, msg);
}

pub fn warn(msg: &str) {
    eprintln!("{}[ask]{} {}", WARN, RESET, msg);
}

pub fn error(msg: &str) {
    eprintln!("{}[error]{} {}", ERR, RESET, msg);
}
