use std::collections::BTreeMap;
use std::time::Duration;

use crunchyroll_rs::Locale;

use crate::Error;
use crate::api::CrunchyrollApi;
use crate::plan::PreparedSubtitle;

pub(crate) trait SubtitleFetcher {
    async fn fetch(&self, url: &str) -> Result<String, Error>;
}

impl<T: CrunchyrollApi> SubtitleFetcher for T {
    async fn fetch(&self, url: &str) -> Result<String, Error> {
        self.fetch_subtitle(url).await
    }
}

/// A supported subtitle source format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SubtitleFormat {
    /// Advanced SubStation Alpha.
    Ass,
    /// Web Video Text Tracks.
    WebVtt,
}

impl SubtitleFormat {
    pub(crate) fn parse(value: &str) -> Result<Self, Error> {
        match value.to_ascii_lowercase().as_str() {
            "ass" | "ssa" => Ok(Self::Ass),
            "vtt" | "webvtt" => Ok(Self::WebVtt),
            _ => Err(Error::Subtitle("unsupported subtitle format".to_string())),
        }
    }
}

/// Stable metadata carried from selection into the output subtitle track.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubtitleMetadata {
    /// Subtitle locale.
    pub locale: Locale,
    /// Output track title.
    pub title: String,
    /// Whether this is a closed-caption resource.
    pub is_caption: bool,
    /// Whether this is a signs resource matching selected audio.
    pub is_signs: bool,
    /// Whether players should enable the track by default.
    pub default: bool,
    /// Whether players should force the track on.
    pub forced: bool,
}

/// Explicit structural ASS normalization choices.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AssNormalization {
    /// Add missing PlayResX/PlayResY with these values.
    pub play_resolution: Option<(u32, u32)>,
    /// Add missing LayoutResX/LayoutResY with these values.
    pub layout_resolution: Option<(u32, u32)>,
    /// Add or replace WrapStyle.
    pub wrap_style: Option<u8>,
    /// Add or replace Timer.
    pub timer: Option<f64>,
    /// Add or replace ScaledBorderAndShadow.
    pub scaled_border_and_shadow: Option<bool>,
    /// Remove semicolon comments and the Aegisub Project Garbage section.
    pub remove_project_garbage: bool,
    /// Clamp dialogue ends and remove dialogue starts outside media duration.
    pub clamp_to_duration: bool,
}

/// Options for converting or structurally normalizing one subtitle resource.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SubtitleProcessingOptions {
    /// Normalization settings; `None` preserves raw ASS exactly.
    pub normalization: Option<AssNormalization>,
    /// Known media duration used only when clamping is explicitly enabled.
    pub media_duration: Option<Duration>,
}

/// A processed ASS subtitle ready for Matroska muxing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubtitleTrack {
    /// Exact output metadata selected by the caller.
    pub metadata: SubtitleMetadata,
    /// ASS document, including script info, styles, and events.
    pub ass: String,
    /// Canonical font family names actually referenced by styles or overrides.
    pub referenced_fonts: Vec<String>,
}

/// Convert or normalize a subtitle resource without losing track metadata.
///
/// Raw ASS is byte-for-byte preserved when no normalization is requested.
/// WebVTT is converted to ASS with cue order, basic inline styling, class
/// styles, line breaks, and percentage positioning retained.
///
/// # Errors
///
/// Returns a subtitle error for malformed required sections, timestamps, or
/// unsupported input.
pub fn process_subtitle(
    source: &str,
    format: SubtitleFormat,
    metadata: SubtitleMetadata,
    options: &SubtitleProcessingOptions,
) -> Result<SubtitleTrack, Error> {
    let mut ass = match format {
        SubtitleFormat::Ass => source.to_string(),
        SubtitleFormat::WebVtt => vtt_to_ass(source, &metadata.title)?,
    };
    if let Some(normalization) = &options.normalization {
        ass = normalize_ass(&ass, normalization, options.media_duration)?;
    }
    let referenced_fonts = extract_fonts(&ass)?;
    Ok(SubtitleTrack {
        metadata,
        ass,
        referenced_fonts,
    })
}

#[allow(dead_code)]
pub(crate) async fn download_selected<A: SubtitleFetcher>(
    api: &A,
    subtitles: &[PreparedSubtitle],
    options: &SubtitleProcessingOptions,
) -> Result<Vec<SubtitleTrack>, Error> {
    let mut tracks = Vec::with_capacity(subtitles.len());
    for subtitle in subtitles {
        let source = api.fetch(&subtitle.url).await?;
        let diagnostic = &subtitle.diagnostic;
        tracks.push(process_subtitle(
            &source,
            SubtitleFormat::parse(&diagnostic.format)?,
            SubtitleMetadata {
                locale: diagnostic.locale.clone(),
                title: diagnostic.title.clone(),
                is_caption: diagnostic.is_caption,
                is_signs: diagnostic.is_signs,
                default: diagnostic.default,
                forced: diagnostic.forced,
            },
            options,
        )?);
    }
    Ok(tracks)
}

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

fn vtt_to_ass(source: &str, title: &str) -> Result<String, Error> {
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

fn ass_style(
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

fn style_name(value: &str) -> String {
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

fn cue_text_to_ass(text: &str, styles: &BTreeMap<String, CueStyle>) -> (String, String) {
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

fn cue_position(settings: &BTreeMap<String, String>) -> Result<String, Error> {
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

fn ass_timestamp(milliseconds: u64) -> String {
    let centiseconds = (milliseconds + 5) / 10;
    let hours = centiseconds / 360_000;
    let minutes = centiseconds / 6_000 % 60;
    let seconds = centiseconds / 100 % 60;
    let fraction = centiseconds % 100;
    format!("{hours}:{minutes:02}:{seconds:02}.{fraction:02}")
}

fn normalize_ass(
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

fn extract_fonts(ass: &str) -> Result<Vec<String>, Error> {
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

fn section_index(lines: &[String], name: &str) -> Option<usize> {
    lines.iter().position(|line| {
        section_name(line).is_some_and(|section| section.eq_ignore_ascii_case(name))
    })
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

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::PlannedSubtitle;

    fn metadata() -> SubtitleMetadata {
        SubtitleMetadata {
            locale: Locale::en_US,
            title: "English".to_string(),
            is_caption: false,
            is_signs: true,
            default: false,
            forced: true,
        }
    }

    #[test]
    fn vtt_preserves_styles_position_breaks_and_metadata() {
        let source = "WEBVTT\n\nSTYLE\n::cue(.sign) { font-family: 'Trebuchet MS'; color: #12A0FF; font-weight: bold; }\n\n00:00:01.005 --> 00:00:02.995 line:10% position:25%\n<c.sign><i>Hello</i>\nworld</c>\n";
        let track = process_subtitle(
            source,
            SubtitleFormat::WebVtt,
            metadata(),
            &SubtitleProcessingOptions::default(),
        )
        .expect("convert");
        assert!(track.ass.contains("Style: Vtt_sign,Trebuchet MS,42"));
        assert!(
            track
                .ass
                .contains("{\\pos(320,72)}{\\i1}Hello{\\i0}\\Nworld{\\r}")
        );
        assert!(track.ass.contains("0:00:01.01,0:00:03.00,Vtt_sign"));
        assert_eq!(track.referenced_fonts, ["Arial", "Trebuchet MS"]);
        assert!(track.metadata.is_signs && track.metadata.forced);
    }

    #[test]
    fn raw_ass_is_preserved_and_normalized_ass_is_structural() {
        let source = "[Script Info]\n; generated\nScriptType: v4.00+\n\n[Aegisub Project Garbage]\nLast Style Storage: Sign\n\n[V4+ Styles]\nFormat: Name, Fontname, Fontsize\nStyle: Sign,\"Noto Sans Thai\",30\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:01.00,0:00:12.00,Sign,,0,0,0,,{\\fnArial}Positioned, comma\nDialogue: 0,0:00:11.00,0:00:12.00,Sign,,0,0,0,,Too late\n";
        let raw = process_subtitle(
            source,
            SubtitleFormat::Ass,
            metadata(),
            &SubtitleProcessingOptions::default(),
        )
        .expect("raw");
        assert_eq!(raw.ass, source);
        assert_eq!(raw.referenced_fonts, ["Noto Sans Thai", "Arial"]);

        let options = SubtitleProcessingOptions {
            normalization: Some(AssNormalization {
                play_resolution: Some((1920, 1080)),
                layout_resolution: Some((1920, 1080)),
                wrap_style: Some(0),
                timer: Some(100.0),
                scaled_border_and_shadow: Some(true),
                remove_project_garbage: true,
                clamp_to_duration: true,
            }),
            media_duration: Some(Duration::from_secs(10)),
        };
        let normalized = process_subtitle(source, SubtitleFormat::Ass, metadata(), &options)
            .expect("normalized");
        assert!(!normalized.ass.contains("Project Garbage"));
        assert!(!normalized.ass.contains("; generated"));
        assert!(normalized.ass.contains("PlayResX: 1920"));
        assert!(normalized.ass.contains("LayoutResY: 1080"));
        assert!(
            normalized
                .ass
                .contains("0:00:10.00,Sign,,0,0,0,,{\\fnArial}Positioned, comma")
        );
        assert!(!normalized.ass.contains("Too late"));
    }

    struct FixtureFetcher {
        calls: Mutex<Vec<String>>,
        body: String,
    }

    impl SubtitleFetcher for FixtureFetcher {
        async fn fetch(&self, url: &str) -> Result<String, Error> {
            self.calls.lock().expect("lock").push(url.to_string());
            Ok(self.body.clone())
        }
    }

    #[tokio::test]
    async fn selected_resources_are_downloaded_and_keep_output_metadata() {
        let source = "[Script Info]\nScriptType: v4.00+\n[V4+ Styles]\nFormat: Name, Fontname\nStyle: Default,Arial\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n";
        let fetcher = FixtureFetcher {
            calls: Mutex::new(Vec::new()),
            body: source.to_string(),
        };
        let selected = [PreparedSubtitle {
            diagnostic: PlannedSubtitle {
                locale: Locale::en_US,
                format: "ass".to_string(),
                is_caption: true,
                is_signs: false,
                resource_identity: "https://example.test/sub.ass".to_string(),
                title: "English (CC)".to_string(),
                default: false,
                forced: false,
            },
            url: "https://example.test/sub.ass?token=secret".to_string(),
        }];
        let tracks = download_selected(&fetcher, &selected, &SubtitleProcessingOptions::default())
            .await
            .expect("download");
        assert_eq!(tracks.len(), 1);
        assert!(tracks[0].metadata.is_caption);
        assert_eq!(tracks[0].metadata.title, "English (CC)");
        assert_eq!(tracks[0].referenced_fonts, ["Arial"]);
        assert_eq!(fetcher.calls.lock().expect("lock").len(), 1);
    }
}
