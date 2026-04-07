/// Strip non-alphanumeric/underscore characters from a string intended for use as an identifier.
pub fn sanitize_identifier(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric() || *c == '_')
        .collect()
}
