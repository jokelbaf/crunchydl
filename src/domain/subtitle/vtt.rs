//! WebVTT parsing and cue collection.

use std::collections::BTreeMap;

use crate::Error;

mod render;

use render::{ass_style, ass_timestamp, cue_position, cue_text_to_ass, style_name};

#[derive(Clone, Debug)]
struct Cue {
    start_ms: u64,
    end_ms: u64,
    settings: BTreeMap<String, String>,
    text: String,
}

#[derive(Clone, Debug, Default)]
struct CueStyle {
    name: String,
    font_family: Option<String>,
    font_size: Option<f32>,
    color: Option<String>,
    bold: bool,
    italic: bool,
    underline: bool,
}

pub(super) fn vtt_to_ass(source: &str, title: &str) -> Result<String, Error> {
    let normalized = source
        .trim_start_matches('\u{feff}')
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let blocks = text_blocks(&normalized);
    let Some(first) = blocks.first() else {
        return Err(Error::Subtitle("empty WebVTT document".to_string()));
    };
    if !first
        .first()
        .is_some_and(|line| line.trim_start().starts_with("WEBVTT"))
    {
        return Err(Error::Subtitle("missing WebVTT header".to_string()));
    }
    let mut styles = BTreeMap::new();
    let mut cues = Vec::new();
    for block in blocks.iter().skip(1) {
        let Some(first_line) = block.first() else {
            continue;
        };
        if first_line.trim_start().starts_with("NOTE") {
            continue;
        }
        if first_line.trim() == "REGION" {
            return Err(Error::Subtitle(
                "WebVTT regions are outside the supported conversion profile".to_string(),
            ));
        }
        if first_line.trim() == "STYLE" {
            parse_css(&block[1..].join("\n"), &mut styles);
            continue;
        }
        cues.push(parse_cue(block)?);
    }
    let mut output = vec![
        "[Script Info]".to_string(),
        format!("Title: {title}"),
        "ScriptType: v4.00+".to_string(),
        "PlayResX: 1280".to_string(),
        "PlayResY: 720".to_string(),
        "WrapStyle: 0".to_string(),
        "ScaledBorderAndShadow: yes".to_string(),
        String::new(),
        "[V4+ Styles]".to_string(),
        "Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding".to_string(),
        ass_style("Default", "Arial", 42.0, "&H00FFFFFF", false, false, false),
    ];
    for style in styles.values() {
        output.push(ass_style(
            &style.name,
            style.font_family.as_deref().unwrap_or("Arial"),
            style.font_size.unwrap_or(42.0),
            style.color.as_deref().unwrap_or("&H00FFFFFF"),
            style.bold,
            style.italic,
            style.underline,
        ));
    }
    output.extend([
        String::new(),
        "[Events]".to_string(),
        "Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text"
            .to_string(),
    ]);
    for cue in cues {
        let (style, text) = cue_text_to_ass(&cue.text, &styles);
        let position = cue_position(&cue.settings)?;
        output.push(format!(
            "Dialogue: 0,{},{},{style},,0,0,0,,{position}{text}",
            ass_timestamp(cue.start_ms),
            ass_timestamp(cue.end_ms)
        ));
    }
    Ok(output.join("\r\n") + "\r\n")
}

fn text_blocks(source: &str) -> Vec<Vec<&str>> {
    let mut blocks = Vec::new();
    let mut current = Vec::new();
    for line in source.lines() {
        if line.trim().is_empty() {
            if !current.is_empty() {
                blocks.push(std::mem::take(&mut current));
            }
        } else {
            current.push(line);
        }
    }
    if !current.is_empty() {
        blocks.push(current);
    }
    blocks
}

fn parse_cue(block: &[&str]) -> Result<Cue, Error> {
    let time_index = block
        .iter()
        .position(|line| line.contains("-->"))
        .ok_or_else(|| Error::Subtitle("WebVTT cue has no timing line".to_string()))?;
    let timing = block[time_index];
    let (start, remainder) = timing
        .split_once("-->")
        .ok_or_else(|| Error::Subtitle("invalid WebVTT timing line".to_string()))?;
    let mut right = remainder.split_whitespace();
    let end = right
        .next()
        .ok_or_else(|| Error::Subtitle("WebVTT cue has no end timestamp".to_string()))?;
    let settings = right
        .filter_map(|setting| setting.split_once(':'))
        .map(|(key, value)| (key.to_ascii_lowercase(), value.to_string()))
        .collect();
    let start_ms = parse_vtt_timestamp(start.trim())?;
    let end_ms = parse_vtt_timestamp(end)?;
    if end_ms < start_ms {
        return Err(Error::Subtitle(
            "WebVTT cue ends before it starts".to_string(),
        ));
    }
    Ok(Cue {
        start_ms,
        end_ms,
        settings,
        text: block[time_index + 1..].join("\n"),
    })
}

fn parse_vtt_timestamp(value: &str) -> Result<u64, Error> {
    let parts = value.split(':').collect::<Vec<_>>();
    let (hours, minutes, seconds) = match parts.as_slice() {
        [minutes, seconds] => (0_u64, parse_u64(minutes)?, *seconds),
        [hours, minutes, seconds] => (parse_u64(hours)?, parse_u64(minutes)?, *seconds),
        _ => return Err(Error::Subtitle("invalid WebVTT timestamp".to_string())),
    };
    let (seconds, millis) = seconds
        .split_once('.')
        .ok_or_else(|| Error::Subtitle("WebVTT timestamp lacks milliseconds".to_string()))?;
    let seconds = parse_u64(seconds)?;
    let millis = match millis.len() {
        1 => parse_u64(millis)? * 100,
        2 => parse_u64(millis)? * 10,
        3 => parse_u64(millis)?,
        _ => {
            return Err(Error::Subtitle(
                "invalid WebVTT timestamp precision".to_string(),
            ));
        }
    };
    hours
        .checked_mul(3_600_000)
        .and_then(|value| value.checked_add(minutes.checked_mul(60_000)?))
        .and_then(|value| value.checked_add(seconds.checked_mul(1000)?))
        .and_then(|value| value.checked_add(millis))
        .ok_or_else(|| Error::Subtitle("WebVTT timestamp overflow".to_string()))
}

fn parse_u64(value: &str) -> Result<u64, Error> {
    value
        .parse()
        .map_err(|_| Error::Subtitle("invalid numeric subtitle field".to_string()))
}

fn parse_css(css: &str, styles: &mut BTreeMap<String, CueStyle>) {
    let mut rest = css;
    while let Some(cue) = rest.find("::cue") {
        rest = &rest[cue + 5..];
        let Some(open) = rest.find('{') else { break };
        let selector = rest[..open].trim().trim_matches(['(', ')']).trim();
        let Some(close) = rest[open + 1..].find('}') else {
            break;
        };
        let declarations = &rest[open + 1..open + 1 + close];
        rest = &rest[open + 2 + close..];
        let class = selector.trim_start_matches('.');
        if class.is_empty() {
            continue;
        }
        let name = style_name(class);
        let mut style = CueStyle {
            name: name.clone(),
            ..CueStyle::default()
        };
        for declaration in declarations.split(';') {
            let Some((key, value)) = declaration.split_once(':') else {
                continue;
            };
            let value = value.trim();
            match key.trim().to_ascii_lowercase().as_str() {
                "font-family" => {
                    style.font_family = Some(value.trim_matches(['\'', '"']).to_string())
                }
                "font-size" => style.font_size = value.trim_end_matches("px").parse().ok(),
                "color" => style.color = css_color(value),
                "font-weight" => {
                    style.bold = value.eq_ignore_ascii_case("bold")
                        || value.parse::<u16>().is_ok_and(|weight| weight >= 600)
                }
                "font-style" => style.italic = value.eq_ignore_ascii_case("italic"),
                "text-decoration" => style.underline = value.contains("underline"),
                _ => {}
            }
        }
        styles.insert(class.to_string(), style);
    }
}

fn css_color(value: &str) -> Option<String> {
    let hex = value.strip_prefix('#')?;
    if hex.len() != 6 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    Some(format!("&H00{}{}{}", &hex[4..6], &hex[2..4], &hex[0..2]).to_ascii_uppercase())
}
