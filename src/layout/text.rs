pub fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let mut rows = Vec::new();
    let mut current = String::new();

    for word in text.split_whitespace() {
        let current_len = current.chars().count();
        let word_len = word.chars().count();
        let separator = usize::from(!current.is_empty());

        if current_len + separator + word_len > width && !current.is_empty() {
            rows.push(current);
            current = word.to_string();
        } else {
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(word);
        }
    }

    if !current.is_empty() {
        rows.push(current);
    }

    rows
}

pub fn truncate_with_ellipsis(text: &str, width: usize) -> String {
    truncate_text(text, width.saturating_sub(1).max(1))
}

pub fn truncate_text(text: &str, max_width: usize) -> String {
    if text.chars().count() <= max_width {
        return text.to_string();
    }
    if max_width <= 1 {
        return "…".to_string();
    }

    let mut truncated = String::new();
    for ch in text.chars().take(max_width - 1) {
        truncated.push(ch);
    }
    truncated.push('…');
    truncated
}

pub fn pad_to_width(text: &str, width: usize) -> String {
    let current = text.chars().count();
    if current >= width {
        return text.to_string();
    }

    format!("{text}{:width$}", "", width = width - current)
}

pub fn fit_text(text: &str, width: usize) -> String {
    pad_to_width(&truncate_text(text, width), width)
}

pub fn format_lineno(lineno: Option<usize>) -> String {
    lineno
        .map(|value| value.to_string())
        .unwrap_or_else(|| "·".to_string())
}
