pub fn token_count(text: &str) -> usize {
    let mut count = 0;
    let mut ascii_word_bytes = 0;
    for character in text.chars() {
        if character.is_ascii_alphanumeric() || character == '_' {
            ascii_word_bytes += 1;
            continue;
        }
        count += ascii_word_tokens(ascii_word_bytes);
        ascii_word_bytes = 0;
        if character.is_whitespace() {
            continue;
        }
        count += if character.is_ascii() { 1 } else { 2 };
    }
    count + ascii_word_tokens(ascii_word_bytes)
}

const fn ascii_word_tokens(bytes: usize) -> usize {
    bytes.div_ceil(4)
}
