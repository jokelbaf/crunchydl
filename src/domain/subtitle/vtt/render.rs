//! ASS rendering helpers for WebVTT cues.

use std::collections::BTreeMap;

use super::CueStyle;
use crate::Error;

pub(super) fn ass_style(
    name: &str,
    font: &str,
    size: f32,
    color: &str,
    bold: bool,
    italic: bool,
    underline: bool,
) -> String {
    format!(
        "Style: {name},{font},{size},{color},&H000000FF,&H00000000,&H00000000,{},{},{},0,100,100,0,0,1,2,0,2,20,20,30,1",
        if bold { -1 } else { 0 },
        if italic { -1 } else { 0 },
        if underline { -1 } else { 0 }
    )
}

pub(super) fn style_name(value: &str) -> String {
    let mut name = String::from("Vtt_");
    name.extend(value.chars().map(|character| {
        if character.is_ascii_alphanumeric() {
            character
        } else {
            '_'
        }
    }));
    name
}

pub(super) fn cue_text_to_ass(text: &str, styles: &BTreeMap<String, CueStyle>) -> (String, String) {
    let mut output = String::new();
    let mut default_style = "Default".to_string();
    let mut rest = text;
    while let Some(open) = rest.find('<') {
        output.push_str(&escape_ass_text(&rest[..open]));
        let Some(close) = rest[open + 1..].find('>') else {
            output.push_str(&escape_ass_text(&rest[open..]));
            rest = "";
            break;
        };
        let tag = &rest[open + 1..open + 1 + close];
        match tag {
            "b" => output.push_str("{\\b1}"),
            "/b" => output.push_str("{\\b0}"),
            "i" => output.push_str("{\\i1}"),
            "/i" => output.push_str("{\\i0}"),
            "u" => output.push_str("{\\u1}"),
            "/u" => output.push_str("{\\u0}"),
            "/c" => output.push_str("{\\r}"),
            _ if tag.starts_with("c.") => {
                let class = tag[2..].split('.').next().unwrap_or_default();
                if let Some(style) = styles.get(class) {
                    if output.is_empty() {
                        default_style.clone_from(&style.name);
                    } else {
                        output.push_str(&format!("{{\\r{}}}", style.name));
                    }
                }
            }
            _ => {}
        }
        rest = &rest[open + close + 2..];
    }
    output.push_str(&escape_ass_text(rest));
    (default_style, output.replace('\n', "\\N"))
}

fn escape_ass_text(value: &str) -> String {
    value
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
        .replace('{', "\\{")
        .replace('}', "\\}")
}

pub(super) fn cue_position(settings: &BTreeMap<String, String>) -> Result<String, Error> {
    if settings.contains_key("vertical") || settings.contains_key("size") {
        return Err(Error::Subtitle(
            "vertical or sized WebVTT cues are outside the supported conversion profile"
                .to_string(),
        ));
    }
    if settings
        .get("line")
        .is_some_and(|line| !line.ends_with('%'))
    {
        return Err(Error::Subtitle(
            "line-number WebVTT positioning is outside the supported conversion profile"
                .to_string(),
        ));
    }
    let x = settings
        .get("position")
        .map(|value| percentage(value, 1280))
        .transpose()?
        .unwrap_or(640);
    let y = settings
        .get("line")
        .filter(|value| value.ends_with('%'))
        .map(|value| percentage(value, 720))
        .transpose()?
        .unwrap_or(648);
    let alignment = match settings.get("align").map(String::as_str) {
        Some("start" | "left") => "\\an1",
        Some("end" | "right") => "\\an3",
        Some("center") | None => "",
        Some(_) => {
            return Err(Error::Subtitle(
                "unsupported WebVTT cue alignment".to_string(),
            ));
        }
    };
    if settings.contains_key("position") || settings.contains_key("line") {
        Ok(format!("{{{alignment}\\pos({x},{y})}}"))
    } else if alignment.is_empty() {
        Ok(String::new())
    } else {
        Ok(format!("{{{alignment}}}"))
    }
}

fn percentage(value: &str, scale: u32) -> Result<u32, Error> {
    let number = value
        .trim_end_matches('%')
        .split(',')
        .next()
        .unwrap_or_default()
        .parse::<f64>()
        .map_err(|_| Error::Subtitle("invalid WebVTT position".to_string()))?;
    if !(0.0..=100.0).contains(&number) {
        return Err(Error::Subtitle(
            "WebVTT position outside viewport".to_string(),
        ));
    }
    Ok((number * f64::from(scale) / 100.0).round() as u32)
}

pub(super) fn ass_timestamp(milliseconds: u64) -> String {
    let centiseconds = (milliseconds + 5) / 10;
    let hours = centiseconds / 360_000;
    let minutes = centiseconds / 6_000 % 60;
    let seconds = centiseconds / 100 % 60;
    let fraction = centiseconds % 100;
    format!("{hours}:{minutes:02}:{seconds:02}.{fraction:02}")
}
