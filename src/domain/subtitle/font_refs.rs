//! ASS font-reference extraction.

use crate::Error;

pub(super) fn extract_fonts(ass: &str) -> Result<Vec<String>, Error> {
    let lines = ass.replace("\r\n", "\n").replace('\r', "\n");
    let lines = lines.lines().collect::<Vec<_>>();
    let style_start =
        section_index_str(&lines, "V4+ Styles").or_else(|| section_index_str(&lines, "V4 Styles"));
    let mut fonts = Vec::new();
    if let Some(start) = style_start {
        let end = lines[start + 1..]
            .iter()
            .position(|line| is_section(line))
            .map_or(lines.len(), |offset| start + 1 + offset);
        let format = lines[start + 1..end]
            .iter()
            .find_map(|line| line.trim_start().strip_prefix("Format:"))
            .map(|format| {
                format
                    .split(',')
                    .map(|field| field.trim())
                    .collect::<Vec<_>>()
            })
            .ok_or_else(|| Error::Subtitle("ASS styles section has no Format line".to_string()))?;
        let font_index = format
            .iter()
            .position(|field| field.eq_ignore_ascii_case("Fontname"))
            .ok_or_else(|| Error::Subtitle("ASS style format has no Fontname".to_string()))?;
        for line in &lines[start + 1..end] {
            if let Some(style) = line.trim_start().strip_prefix("Style:")
                && let Some(font) = style.split(',').nth(font_index)
            {
                push_font(&mut fonts, font);
            }
        }
    }
    let mut rest = ass;
    while let Some(index) = rest.find("\\fn") {
        rest = &rest[index + 3..];
        let end = rest.find(['\\', '}']).unwrap_or(rest.len());
        push_font(&mut fonts, &rest[..end]);
        rest = &rest[end..];
    }
    Ok(fonts)
}

fn push_font(fonts: &mut Vec<String>, value: &str) {
    let canonical = value.trim().trim_matches(['\'', '"']);
    if canonical.is_empty() {
        return;
    }
    if !fonts
        .iter()
        .any(|font| font.eq_ignore_ascii_case(canonical))
    {
        fonts.push(canonical.to_string());
    }
}

fn section_index_str(lines: &[&str], name: &str) -> Option<usize> {
    lines.iter().position(|line| {
        section_name(line).is_some_and(|section| section.eq_ignore_ascii_case(name))
    })
}

fn is_section(line: &str) -> bool {
    section_name(line).is_some()
}

fn section_name(line: &str) -> Option<&str> {
    line.trim().strip_prefix('[')?.strip_suffix(']')
}
