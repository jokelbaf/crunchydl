//! Catalog browsing and headless presentation.

use super::*;

pub(crate) async fn login(paths: &AppPaths, arguments: LoginArguments) -> Result<()> {
    let email = match arguments.email {
        Some(email) => email,
        None => prompt("Email: ")?,
    };
    let password = rpassword::prompt_password("Password: ")
        .map(Zeroizing::new)
        .map_err(|_| Error::PasswordInput)?;
    let client = crate::auth::login(paths, email.trim(), password).await?;
    print_account(&account_summary(&client).await);
    Ok(())
}

pub(crate) async fn logout(paths: &AppPaths) -> Result<()> {
    if crate::auth::logout(paths).await? {
        print_success("Signed out and removed the saved session.");
    } else {
        print_warning("No saved login was present.");
    }
    Ok(())
}

pub(crate) async fn status(paths: &AppPaths) -> Result<()> {
    let client = crate::auth::restore(paths).await?;
    print_account(&account_summary(&client).await);
    Ok(())
}

pub(crate) async fn search(paths: &AppPaths, arguments: SearchArguments) -> Result<()> {
    let client = crate::auth::restore(paths).await?;
    let items = crate::catalog::search(&client, &arguments.query, arguments.limit.into()).await?;
    if arguments.json {
        let document = serde_json::to_string_pretty(&items).map_err(|_| Error::OutputEncoding)?;
        println!("{document}");
    } else if items.is_empty() {
        print_warning("No catalog results found.");
    } else {
        let colors = io::stdout().is_terminal();
        print_list_heading("Search results", items.len(), colors);
        for (index, item) in items.iter().enumerate() {
            let mut facts = vec![
                kind_label(item.kind).to_string(),
                if item.premium_only { "Premium" } else { "Free" }.to_string(),
            ];
            if item.rating.is_some() {
                facts.push(catalog_rating_label(item.rating.as_ref()));
            }
            if let Some(year) = item.release_year {
                facts.push(year.to_string());
            }
            print_catalog_item(&(index + 1).to_string(), item, facts, colors);
        }
        print_actions(
            &[
                ("Browse", "crunchydl browse <series|movie-listing> <ID>"),
                (
                    "Download",
                    "crunchydl download <episode|movie|music-video> <ID>",
                ),
            ],
            colors,
        );
    }
    Ok(())
}

pub(crate) async fn browse(paths: &AppPaths, arguments: BrowseArguments) -> Result<()> {
    let client = crate::auth::restore(paths).await?;
    let items =
        crate::catalog::browse(&client, arguments.kind.catalog_kind(), &arguments.id).await?;
    if arguments.json {
        let document = serde_json::to_string_pretty(&items).map_err(|_| Error::OutputEncoding)?;
        println!("{document}");
        return Ok(());
    }
    if items.is_empty() {
        print_warning("This collection has no items.");
        return Ok(());
    }
    match arguments.kind {
        BrowseKind::Series => print_seasons(&items),
        BrowseKind::Season => print_episodes(&items),
        BrowseKind::MovieListing => print_movies(&items),
    }
    Ok(())
}

fn print_seasons(items: &[crunchydl::CatalogItem]) {
    let colors = io::stdout().is_terminal();
    print_list_heading("Seasons", items.len(), colors);
    for item in items {
        let label = item
            .season_number
            .map_or_else(|| "Season".to_string(), |number| format!("S{number}"));
        let facts = item
            .episode_count
            .map(|count| plural(count, "episode", "episodes"))
            .into_iter()
            .collect();
        print_catalog_item(&label, item, facts, colors);
    }
    print_actions(
        &[
            ("View episodes", "crunchydl browse season <SEASON_ID>"),
            ("Download", "crunchydl download season <SEASON_ID>"),
        ],
        colors,
    );
}

fn print_episodes(items: &[crunchydl::CatalogItem]) {
    let colors = io::stdout().is_terminal();
    print_list_heading("Episodes", items.len(), colors);
    for item in items {
        let label = item
            .episode_number
            .as_deref()
            .map_or_else(|| "Episode".to_string(), |number| format!("E{number}"));
        let facts = vec![
            duration_label(item.duration_millis),
            if item.premium_only { "Premium" } else { "Free" }.to_string(),
        ];
        print_catalog_item(&label, item, facts, colors);
    }
    print_actions(
        &[("Download", "crunchydl download episode <EPISODE_ID>")],
        colors,
    );
}

fn print_movies(items: &[crunchydl::CatalogItem]) {
    let colors = io::stdout().is_terminal();
    print_list_heading("Movies", items.len(), colors);
    for (index, item) in items.iter().enumerate() {
        let facts = vec![
            duration_label(item.duration_millis),
            if item.premium_only { "Premium" } else { "Free" }.to_string(),
        ];
        print_catalog_item(&(index + 1).to_string(), item, facts, colors);
    }
    print_actions(
        &[("Download", "crunchydl download movie <MOVIE_ID>")],
        colors,
    );
}

pub(crate) fn print_list_heading(label: &str, count: usize, colors: bool) {
    println!(
        "{}  {}",
        paint(label, "1;36", colors),
        paint(&format!("· {count}"), "2", colors)
    );
    let rule_width = terminal_width().saturating_sub(1).min(72);
    println!("{}", paint(&"─".repeat(rule_width), "2", colors));
}

fn print_catalog_item(
    label: &str,
    item: &crunchydl::CatalogItem,
    facts: Vec<String>,
    colors: bool,
) {
    let title_width = terminal_width()
        .saturating_sub(label.chars().count() + 6)
        .max(16);
    println!(
        "  {}  {}",
        paint(label, "1;35", colors),
        paint(&ellipsize(&item.title, title_width), "1", colors)
    );

    let mut identity = vec![item.id.clone()];
    identity.extend(facts.into_iter().filter(|fact| fact != "-"));
    println!("     {}", paint(&identity.join("  ·  "), "2", colors));

    if let Some(languages) = catalog_language_summary(item) {
        println!("     {}", paint(&languages, "36", colors));
    }
    println!();
}

pub(crate) fn print_actions(actions: &[(&str, &str)], colors: bool) {
    println!("{}", paint("Actions", "1;36", colors));
    for (label, command) in actions {
        println!("  {:<14} {}", label, paint(command, "1", colors));
    }
}

pub(crate) fn catalog_language_summary(item: &crunchydl::CatalogItem) -> Option<String> {
    let mut facts = Vec::new();
    match item.audio_locales.as_slice() {
        [] => {}
        [locale] => facts.push(format!("{} audio", locale_name(locale))),
        locales => facts.push(plural(locales.len(), "audio language", "audio languages")),
    }
    if !item.subtitle_locales.is_empty() {
        facts.push(plural(
            item.subtitle_locales.len(),
            "subtitle language",
            "subtitle languages",
        ));
    }
    (!facts.is_empty()).then(|| facts.join("  ·  "))
}

fn plural(count: impl std::fmt::Display, singular: &str, plural: &str) -> String {
    let count = count.to_string();
    let label = if count == "1" { singular } else { plural };
    format!("{count} {label}")
}

pub(crate) fn terminal_width() -> usize {
    crossterm::terminal::size()
        .map(|(width, _)| usize::from(width))
        .unwrap_or(80)
        .clamp(40, 120)
}

pub(crate) fn duration_label(duration_millis: Option<u64>) -> String {
    duration_millis.map_or_else(
        || "-".to_string(),
        |millis| {
            let seconds = millis / 1_000;
            let hours = seconds / 3_600;
            let minutes = seconds % 3_600 / 60;
            let seconds = seconds % 60;
            if hours == 0 {
                format!("{minutes:02}:{seconds:02}")
            } else {
                format!("{hours}:{minutes:02}:{seconds:02}")
            }
        },
    )
}
