//! Structural ASS normalization.

use std::time::Duration;

use super::AssNormalization;
use crate::Error;

fn parse_u64(value: &str) -> Result<u64, Error> {
    value
        .parse()
        .map_err(|_| Error::Subtitle("invalid numeric subtitle field".to_string()))
}

fn ass_timestamp(milliseconds: u64) -> String {
    let centiseconds = (milliseconds + 5) / 10;
    let hours = centiseconds / 360_000;
    let minutes = centiseconds / 6_000 % 60;
    let seconds = centiseconds / 100 % 60;
    let fraction = centiseconds % 100;
    format!("{hours}:{minutes:02}:{seconds:02}.{fraction:02}")
}

fn section_index(lines: &[String], name: &str) -> Option<usize> {
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

pub(super) fn normalize_ass(
    source: &str,
    options: &AssNormalization,
    duration: Option<Duration>,
) -> Result<String, Error> {
    let crlf = source.contains("\r\n");
    let trailing_newline = source.ends_with('\n') || source.ends_with('\r');
    let normalized = source.replace("\r\n", "\n").replace('\r', "\n");
    let mut lines = normalized.lines().map(str::to_string).collect::<Vec<_>>();
    if options.remove_project_garbage {
        lines.retain(|line| !line.trim_start().starts_with(';'));
        if let Some(start) = section_index(&lines, "Aegisub Project Garbage") {
            let end = lines[start + 1..]
                .iter()
                .position(|line| is_section(line))
                .map_or(lines.len(), |offset| start + 1 + offset);
            lines.drain(start..end);
        }
    }
    let script_start = section_index(&lines, "Script Info")
        .ok_or_else(|| Error::Subtitle("ASS document has no Script Info section".to_string()))?;
    let script_end = lines[script_start + 1..]
        .iter()
        .position(|line| is_section(line))
        .map_or(lines.len(), |offset| script_start + 1 + offset);
    let mut additions = Vec::new();
    if let Some((x, y)) = options.play_resolution {
        set_or_add(
            &mut lines,
            script_start,
            script_end,
            "PlayResX",
            x.to_string(),
            &mut additions,
        );
        set_or_add(
            &mut lines,
            script_start,
            script_end,
            "PlayResY",
            y.to_string(),
            &mut additions,
        );
    }
    if let Some((x, y)) = options.layout_resolution {
        set_or_add(
            &mut lines,
            script_start,
            script_end,
            "LayoutResX",
            x.to_string(),
            &mut additions,
        );
        set_or_add(
            &mut lines,
            script_start,
            script_end,
            "LayoutResY",
            y.to_string(),
            &mut additions,
        );
    }
    if let Some(wrap_style) = options.wrap_style {
        set_or_add(
            &mut lines,
            script_start,
            script_end,
            "WrapStyle",
            wrap_style.to_string(),
            &mut additions,
        );
    }
    if let Some(timer) = options.timer {
        set_or_add(
            &mut lines,
            script_start,
            script_end,
            "Timer",
            format!("{timer:.4}"),
            &mut additions,
        );
    }
    if let Some(scaled) = options.scaled_border_and_shadow {
        set_or_add(
            &mut lines,
            script_start,
            script_end,
            "ScaledBorderAndShadow",
            if scaled { "yes" } else { "no" }.to_string(),
            &mut additions,
        );
    }
    lines.splice(script_end..script_end, additions);
    if options.clamp_to_duration {
        let duration = duration.ok_or_else(|| {
            Error::Subtitle("duration clamping requested without media duration".to_string())
        })?;
        lines = clamp_dialogues(lines, duration)?;
    }
    let separator = if crlf { "\r\n" } else { "\n" };
    let mut output = lines.join(separator);
    if trailing_newline {
        output.push_str(separator);
    }
    Ok(output)
}

fn set_or_add(
    lines: &mut [String],
    start: usize,
    end: usize,
    key: &str,
    value: String,
    additions: &mut Vec<String>,
) {
    let prefix = format!("{}:", key.to_ascii_lowercase());
    if let Some(line) = lines[start + 1..end]
        .iter_mut()
        .find(|line| line.trim_start().to_ascii_lowercase().starts_with(&prefix))
    {
        *line = format!("{key}: {value}");
    } else {
        additions.push(format!("{key}: {value}"));
    }
}

fn clamp_dialogues(lines: Vec<String>, duration: Duration) -> Result<Vec<String>, Error> {
    let limit = u64::try_from(duration.as_millis())
        .map_err(|_| Error::Subtitle("media duration overflow".to_string()))?;
    let mut output = Vec::with_capacity(lines.len());
    for line in lines {
        if !line.trim_start().starts_with("Dialogue:") {
            output.push(line);
            continue;
        }
        let Some((prefix, body)) = line.split_once(':') else {
            continue;
        };
        let mut fields = body
            .trim_start()
            .splitn(10, ',')
            .map(str::to_string)
            .collect::<Vec<_>>();
        if fields.len() != 10 {
            return Err(Error::Subtitle("malformed ASS Dialogue event".to_string()));
        }
        let start = parse_ass_timestamp(&fields[1])?;
        let end = parse_ass_timestamp(&fields[2])?;
        if start >= limit {
            continue;
        }
        if end > limit {
            fields[2] = ass_timestamp(limit);
        }
        output.push(format!("{prefix}: {}", fields.join(",")));
    }
    Ok(output)
}

fn parse_ass_timestamp(value: &str) -> Result<u64, Error> {
    let (hours, rest) = value
        .trim()
        .split_once(':')
        .ok_or_else(|| Error::Subtitle("invalid ASS timestamp".to_string()))?;
    let (minutes, seconds) = rest
        .split_once(':')
        .ok_or_else(|| Error::Subtitle("invalid ASS timestamp".to_string()))?;
    let (seconds, fraction) = seconds
        .replace(',', ".")
        .split_once('.')
        .map(|(seconds, fraction)| (seconds.to_string(), fraction.to_string()))
        .unwrap_or_else(|| (seconds.to_string(), "0".to_string()));
    let fraction_ms = match fraction.len() {
        0 => 0,
        1 => parse_u64(&fraction)? * 100,
        2 => parse_u64(&fraction)? * 10,
        _ => parse_u64(&fraction.chars().take(3).collect::<String>())?,
    };
    parse_u64(hours)?
        .checked_mul(3_600_000)
        .and_then(|value| value.checked_add(parse_u64(minutes).ok()? * 60_000))
        .and_then(|value| value.checked_add(parse_u64(&seconds).ok()? * 1000))
        .and_then(|value| value.checked_add(fraction_ms))
        .ok_or_else(|| Error::Subtitle("ASS timestamp overflow".to_string()))
}
