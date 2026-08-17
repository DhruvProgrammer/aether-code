//! Tiny glob matcher — `*` matches any chars except `/`, `**` matches any
//! chars including `/`. `?` matches one char.

pub fn glob_match(pattern: &str, value: &str) -> bool {
    let pat: Vec<char> = pattern.chars().collect();
    let val: Vec<char> = value.chars().collect();
    glob_match_rec(&pat, 0, &val, 0)
}

fn glob_match_rec(pat: &[char], pi: usize, val: &[char], mut vi: usize) -> bool {
    if pi == pat.len() { return vi == val.len(); }
    match pat[pi] {
        '*' => {
            // Collapse `**`
            if pi + 1 < pat.len() && pat[pi + 1] == '*' {
                if glob_match_rec(pat, pi + 2, val, vi) { return true; }
                if vi < val.len() { return glob_match_rec(pat, pi, val, vi + 1); }
                return false;
            }
            while vi <= val.len() {
                if glob_match_rec(pat, pi + 1, val, vi) { return true; }
                if vi == val.len() || val[vi] == '/' { return false; }
                vi += 1;
            }
            false
        }
        '?' => {
            if vi >= val.len() || val[vi] == '/' { return false; }
            glob_match_rec(pat, pi + 1, val, vi + 1)
        }
        c => {
            if vi >= val.len() || val[vi] != c { return false; }
            glob_match_rec(pat, pi + 1, val, vi + 1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn simple_star() { assert!(glob_match("*.txt", "foo.txt")); assert!(!glob_match("*.txt", "foo.rs")); }
    #[test] fn double_star() { assert!(glob_match("**/*.env", "a/b/c/.env")); }
    #[test] fn literal() { assert!(glob_match("README.md", "README.md")); assert!(!glob_match("README.md", "readme.md")); }
    #[test] fn star_in_middle() { assert!(glob_match("/x/*/y", "/x/a/y")); assert!(!glob_match("/x/*/y", "/x/a/b/y")); }
    #[test] fn question_mark() { assert!(glob_match("a?c", "abc")); assert!(!glob_match("a?c", "ac")); }
}
