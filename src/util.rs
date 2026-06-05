//! Small pure helpers: identifier casing, keyword handling, and comment scanning.

/// Convert a Rust identifier (CamelCase, SCREAMING_SNAKE, or mixed) into a
/// snake_case module file stem.
///
/// Examples: `Foo` → `foo`, `HTTPServer` → `http_server`, `MAX_SIZE` → `max_size`,
/// `IOError` → `io_error`, `parse_input` → `parse_input`.
pub fn to_snake(ident: &str) -> String {
    let chars: Vec<char> = ident.chars().collect();
    let mut out = String::with_capacity(ident.len() + 4);
    for (i, &c) in chars.iter().enumerate() {
        if c == '_' {
            // Collapse to a single underscore; never start with one.
            if !out.ends_with('_') && !out.is_empty() {
                out.push('_');
            }
            continue;
        }
        if c.is_ascii_uppercase() {
            let prev = if i > 0 { Some(chars[i - 1]) } else { None };
            let next = chars.get(i + 1).copied();
            let boundary = match prev {
                None => false,
                Some('_') => false,
                // lower/digit -> Upper : boundary  (parseInput | http2Server)
                Some(p) if p.is_ascii_lowercase() || p.is_ascii_digit() => true,
                // Upper -> Upper followed by lower : boundary (HTTPServer -> HTTP|Server)
                Some(p) if p.is_ascii_uppercase() => {
                    matches!(next, Some(n) if n.is_ascii_lowercase())
                }
                _ => false,
            };
            if boundary && !out.is_empty() && !out.ends_with('_') {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    let trimmed = out.trim_matches('_').to_string();
    let stem = if trimmed.is_empty() { "item".to_string() } else { trimmed };
    sanitize_stem(&stem)
}

/// Strict + reserved Rust keywords that cannot be used as a bare module name.
const KEYWORDS: &[&str] = &[
    "as", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern", "false", "fn",
    "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref",
    "return", "self", "Self", "static", "struct", "super", "trait", "true", "type", "unsafe",
    "use", "where", "while", "async", "await", "abstract", "become", "box", "do", "final",
    "macro", "override", "priv", "typeof", "unsized", "virtual", "yield", "try", "gen",
];

/// Ensure a file stem is a usable module name (not a keyword, not empty).
fn sanitize_stem(stem: &str) -> String {
    if KEYWORDS.contains(&stem) {
        format!("{stem}_")
    } else {
        stem.to_string()
    }
}

/// Is `name` a Rust keyword (used to decide whether a `mod`/`use` needs adjustment)?
pub fn is_keyword(name: &str) -> bool {
    KEYWORDS.contains(&name)
}

/// Byte offset of the start of the line containing `byte` in `src`.
pub fn line_start(src: &str, byte: usize) -> usize {
    src[..byte].rfind('\n').map(|i| i + 1).unwrap_or(0)
}

/// Given the gap between the previous item's end and this item's start, return the
/// byte offset at which a contiguous block of plain `//` comments directly above the
/// item begins. Doc-comments are already part of the item span, so they never appear
/// here. Returns `item_start` when there is no attached comment block.
///
/// A comment block is "attached" only if it is immediately above the item with no
/// intervening blank line.
pub fn leading_comment_start(src: &str, gap_start: usize, item_start: usize) -> usize {
    let ls = line_start(src, item_start);
    if ls <= gap_start {
        return item_start;
    }
    // Walk upward over whole lines strictly above the item's line.
    let mut block_start = ls;
    let mut cursor = ls;
    loop {
        if cursor <= gap_start {
            break;
        }
        // `cursor` is the start of a line; find the start of the previous line.
        let prev_line_end = cursor - 1; // the '\n' ending the previous line
        let prev_line_start = src[gap_start..prev_line_end]
            .rfind('\n')
            .map(|i| gap_start + i + 1)
            .unwrap_or(gap_start);
        let line = src[prev_line_start..prev_line_end].trim();
        let is_comment = line.starts_with("//") && !line.starts_with("///") && !line.starts_with("//!");
        // `///`/`//!` shouldn't appear in the gap, but guard anyway.
        if is_comment || (line.starts_with("//") && prev_line_start >= gap_start) {
            block_start = prev_line_start;
            cursor = prev_line_start;
        } else {
            break;
        }
    }
    if block_start < ls {
        block_start
    } else {
        item_start
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snake_cases() {
        assert_eq!(to_snake("Foo"), "foo");
        assert_eq!(to_snake("FooBar"), "foo_bar");
        assert_eq!(to_snake("HTTPServer"), "http_server");
        assert_eq!(to_snake("IOError"), "io_error");
        assert_eq!(to_snake("MAX_SIZE"), "max_size");
        assert_eq!(to_snake("parse_input"), "parse_input");
        assert_eq!(to_snake("Http2Server"), "http2_server");
        assert_eq!(to_snake("A"), "a");
        assert_eq!(to_snake("VersionReq"), "version_req");
    }

    #[test]
    fn keyword_stems_are_sanitized() {
        // `Match` → `match` is a keyword, must be escaped.
        assert_eq!(to_snake("Match"), "match_");
        assert_eq!(to_snake("Type"), "type_");
        assert!(!is_keyword("match_"));
    }
}

/// If the remainder of the line after `end` is only a trailing `//` line comment,
/// extend `end` to include it (but not the newline). Otherwise return `end`.
pub fn extend_trailing_comment(src: &str, end: usize) -> usize {
    let bytes = src.as_bytes();
    let line_end = src[end..].find('\n').map(|i| end + i).unwrap_or(src.len());
    let rest = &src[end..line_end];
    let trimmed = rest.trim_start();
    if trimmed.starts_with("//") {
        // Make sure we didn't just clip into a `///` that belongs elsewhere; trailing
        // doc comments after an item are unusual, treat them as trailing text anyway.
        let _ = bytes;
        line_end
    } else {
        end
    }
}
