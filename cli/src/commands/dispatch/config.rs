//! Configuration commands and shared command-line parsing helpers.

use super::*;

pub(crate) fn config(paths: &AppPaths, arguments: ConfigArguments) -> Result<()> {
    match arguments.command {
        Some(ConfigCommand::Set(arguments)) => {
            let mut config = Config::load(paths)?;
            if let Some(output_dir) = arguments.output_dir {
                config.output_dir = output_dir;
            }
            if let Some(filename) = arguments.filename {
                crunchydl::FilenameTemplate::compile(&filename)
                    .map_err(|_| Error::InvalidTemplate)?;
                config.filename = filename;
            }
            if let Some(layout) = arguments.output_layout {
                crunchydl::OutputLayoutTemplate::compile(&layout)
                    .map_err(|_| Error::InvalidTemplate)?;
                config.output_layout = Some(layout);
            } else if arguments.flat_output {
                config.output_layout = None;
            }
            if let Some(backend) = arguments.drm_backend {
                config.drm_backend = backend;
            }
            if let Some(device) = arguments.drm_device {
                config.drm_device = Some(device);
            }
            if let Some(endpoint) = arguments.license_endpoint {
                config.license_endpoint = Some(endpoint);
            } else if arguments.clear_license_endpoint {
                config.license_endpoint = None;
            }
            config.save(paths)?;
            print_success(&format!(
                "Configuration saved to {}.",
                paths.config.display()
            ));
            Ok(())
        }
        Some(ConfigCommand::Paths) => {
            let colors = io::stdout().is_terminal();
            print_page_heading("Application paths", colors);
            print_path_setting("Config", &paths.config, colors);
            print_path_setting("Session", &paths.session, colors);
            print_path_setting("Archive", &paths.archive, colors);
            print_path_setting("Queue", &paths.queue, colors);
            print_path_setting("Thumbnails", &paths.thumbnail_cache, colors);
            Ok(())
        }
        None => {
            let config = Config::load(paths)?;
            let colors = io::stdout().is_terminal();
            print_page_heading("Configuration", colors);

            print_settings_group("Output", colors);
            print_setting("Directory", &compact_path(&config.output_dir), "36", colors);
            print_setting(
                "Folder layout",
                config.output_layout.as_deref().unwrap_or("Disabled"),
                "36",
                colors,
            );
            print_setting("Filename", &config.filename, "36", colors);

            println!();
            print_settings_group("DRM", colors);
            print_setting("Backend", &config.drm_backend.to_string(), "36", colors);
            if let Some(device) = config.drm_device.as_deref() {
                print_setting("Device", &compact_path(device), "36", colors);
            } else {
                print_setting("Device", "Not configured", "33", colors);
            }
            print_setting(
                "License endpoint",
                if config.license_endpoint.is_some() {
                    "Custom override"
                } else {
                    "Automatic"
                },
                "36",
                colors,
            );

            println!();
            print_actions(
                &[
                    ("Edit", "crunchydl config set --help"),
                    ("Show paths", "crunchydl config paths"),
                ],
                colors,
            );
            Ok(())
        }
    }
}

fn print_page_heading(label: &str, colors: bool) {
    println!("{}", paint(label, "1;36", colors));
    let rule_width = terminal_width().saturating_sub(1).min(72);
    println!("{}\n", paint(&"─".repeat(rule_width), "2", colors));
}

fn print_settings_group(label: &str, colors: bool) {
    println!("{}", paint(label, "1;35", colors));
}

fn print_setting(label: &str, value: &str, value_color: &str, colors: bool) {
    let available = terminal_width().saturating_sub(5).max(20);
    let value = ellipsize_middle(value, available);
    println!("  {}", paint(label, "2", colors));
    println!("    {}", paint(&value, value_color, colors));
}

fn print_path_setting(label: &str, path: &std::path::Path, colors: bool) {
    let available = terminal_width().saturating_sub(18).max(20);
    let path = ellipsize_middle(&compact_path(path), available);
    println!(
        "  {}  {}",
        paint(&format!("{label:<12}"), "2", colors),
        paint(&path, "36", colors)
    );
}

pub(crate) fn parse_locales(values: Vec<String>) -> Result<Vec<Locale>> {
    values
        .into_iter()
        .map(|value| {
            let locale = Locale::from(value.clone());
            if matches!(locale, Locale::Custom(_)) {
                Err(Error::InvalidLocale(value))
            } else {
                Ok(locale)
            }
        })
        .collect()
}

pub(crate) fn validate_format_arguments(arguments: &DownloadArguments) -> Result<()> {
    if matches!(arguments.format, QueueFormat::Mp4)
        && (!arguments.no_subtitles || !arguments.no_chapters)
    {
        return Err(Error::InvalidTarget(
            "MP4 currently preserves AVC/AAC only; pass --no-subtitles and --no-chapters explicitly, or use Matroska"
                .to_string(),
        ));
    }
    Ok(())
}

pub(crate) fn prompt(label: &str) -> Result<String> {
    print!("{label}");
    io::stdout().flush().map_err(|_| Error::TerminalInput)?;
    let mut value = String::new();
    io::stdin()
        .read_line(&mut value)
        .map_err(|_| Error::TerminalInput)?;
    Ok(value.trim().to_string())
}
