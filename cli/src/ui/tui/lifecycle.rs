use super::*;

pub(crate) async fn run(paths: &AppPaths) -> Result<()> {
    let client = match crate::auth::restore(paths).await {
        Ok(client) => client,
        Err(Error::NotLoggedIn | Error::SessionExpired) => login_before_tui(paths).await?,
        Err(error) => return Err(error),
    };
    let account = account_summary(&client).await;
    let config = Config::load(paths)?;
    let queue_items = Queue::load(&paths.queue)?.items().to_vec();
    let picker = Picker::from_query_stdio().unwrap_or_else(|_| Picker::halfblocks());
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let mut app = App {
        client,
        account,
        config,
        paths: paths.clone(),
        screen: Screen::Search,
        previous_screen: Screen::Search,
        query: String::new(),
        items: Vec::new(),
        selected: 0,
        queue_items,
        queue_selected: 0,
        settings_selected: 0,
        settings_editing: None,
        edit_buffer: String::new(),
        confirmation: None,
        notice: Notice::info("Start typing to search the Crunchyroll catalog"),
        search_deadline: None,
        generation: 0,
        sender,
        selection: Selection::default(),
        browse_parents: Vec::new(),
        queue_running: false,
        queue_cancellation: None,
        progress: DownloadProgress::default(),
        picker,
        thumbnail: None,
        thumbnail_loading: None,
        image_client: reqwest::Client::new(),
        should_quit: false,
    };
    let mut session = TerminalSession::enter()?;
    let mut events = EventStream::new();
    while !app.should_quit {
        session
            .terminal
            .draw(|frame| draw(frame, &mut app))
            .map_err(|_| Error::TerminalInput)?;
        tokio::select! {
            event = events.next() => match event {
                Some(Ok(Event::Key(key))) if key.kind == crossterm::event::KeyEventKind::Press => {
                    handle_key(&mut app, key)?;
                }
                Some(Ok(Event::Resize(_, _))) | Some(Ok(_)) => {}
                Some(Err(_)) | None => return Err(Error::TerminalInput),
            },
            Some(message) = receiver.recv() => handle_message(&mut app, message),
            _ = tokio::time::sleep(search_wakeup(&app)) => {
                if app.search_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                    app.start_search();
                }
            }
        }
    }
    Ok(())
}

fn search_wakeup(app: &App) -> Duration {
    app.search_deadline
        .map_or(Duration::from_secs(3600), |deadline| {
            deadline.saturating_duration_since(Instant::now())
        })
}

async fn login_before_tui(paths: &AppPaths) -> Result<crunchyroll_rs::Crunchyroll> {
    println!("Welcome to crunchydl - sign in to continue.\n");
    print!("Email: ");
    io::Write::flush(&mut io::stdout()).map_err(|_| Error::TerminalInput)?;
    let mut email = String::new();
    io::stdin()
        .read_line(&mut email)
        .map_err(|_| Error::TerminalInput)?;
    let password = rpassword::prompt_password("Password: ")
        .map(zeroize::Zeroizing::new)
        .map_err(|_| Error::PasswordInput)?;
    crate::auth::login(paths, email.trim(), password).await
}
