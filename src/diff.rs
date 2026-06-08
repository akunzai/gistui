use sha2::{Digest, Sha256};
use similar::{ChangeTag, TextDiff};

pub fn sha256_hex(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

pub fn unified_diff(old_label: &str, old: &str, new_label: &str, new: &str) -> String {
    let diff = TextDiff::from_lines(old, new);
    let mut out = format!("--- {old_label}\n+++ {new_label}\n");

    for change in diff.iter_all_changes() {
        let sign = match change.tag() {
            ChangeTag::Delete => "-",
            ChangeTag::Insert => "+",
            ChangeTag::Equal => " ",
        };
        out.push_str(sign);
        out.push_str(change.value());
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_stable() {
        assert_eq!(sha256_hex("abc"), sha256_hex("abc"));
        assert_ne!(sha256_hex("abc"), sha256_hex("abcd"));
    }

    #[test]
    fn diff_shows_insert_and_delete() {
        let diff = unified_diff("gist", "a\nb\n", "local", "a\nc\n");
        assert!(diff.contains("-b\n"));
        assert!(diff.contains("+c\n"));
    }
}
