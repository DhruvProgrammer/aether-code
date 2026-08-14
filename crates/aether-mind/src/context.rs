//! Repository context discovery (spec §11, §12). Walks up from `root` collecting
//! `AGENTS.md` / `CLAUDE.md` / `AETHER.md` / `CONTEXT.md`. More specific (deeper) wins,
//! so we read deepest first.

use std::path::Path;

pub fn discover_context(root: &Path) -> String {
    let names = ["AGENTS.md", "CLAUDE.md", "AETHER.md", "CONTEXT.md"];
    let mut out = String::new();
    let mut cur = Some(root.to_path_buf());
    let mut levels = 0;
    while let Some(p) = cur {
        if levels > 6 {
            break;
        }
        for n in &names {
            let f = p.join(n);
            if f.exists() {
                if let Ok(text) = std::fs::read_to_string(&f) {
                    out.push_str(&format!("\n## {}\n{}\n", n, text));
                }
            }
        }
        cur = p.parent().map(|x| x.to_path_buf());
        levels += 1;
    }
    out
}
