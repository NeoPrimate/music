mod app;
mod events;
mod music;
mod ui;
mod ytdlp;

use app::App;
use clap::Parser;
use color_eyre::eyre::Result;
use crossterm::event::EventStream;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use events::AppEvent;
use futures::StreamExt;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::path::PathBuf;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use ytdlp::DownloadConfig;

#[derive(Parser, Debug)]
#[command(name = "yt-music-tui", about = "YouTube playlists → Apple Music TUI")]
struct Cli {
    #[arg(
        short,
        long,
        default_value = "https://www.youtube.com/@NeotenicApe/playlists"
    )]
    channel: String,

    #[arg(short = 'j', long, default_value_t = 3)]
    parallelism: usize,

    #[arg(long, default_value = "/opt/homebrew/bin/yt-dlp")]
    ytdlp: PathBuf,

    #[arg(long, default_value = "/opt/homebrew/bin/ffmpeg")]
    ffmpeg: PathBuf,

    #[arg(long, default_value = "safari")]
    browser: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    let cli = Cli::parse();

    if let Err(e) = music::check_copy_on_import().await {
        eprintln!("Error: {e}");
        std::process::exit(2);
    }

    eprintln!("Fetching playlists from {} ...", cli.channel);
    let playlists = ytdlp::list_playlists(&cli.channel, &cli.ytdlp).await?;
    let library = music::list_user_playlists().await.unwrap_or_default();

    let cfg = DownloadConfig {
        ytdlp: cli.ytdlp.clone(),
        ffmpeg: cli.ffmpeg.clone(),
        browser: cli.browser.clone(),
    };

    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    stdout.execute(EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let (tx, mut rx) = mpsc::unbounded_channel::<AppEvent>();
    let cancel = CancellationToken::new();
    let mut app = App::new(
        playlists,
        library,
        cli.channel,
        cfg,
        cli.parallelism,
        tx,
        cancel.clone(),
    );

    let res = run(&mut terminal, &mut app, &mut rx).await;

    cancel.cancel();
    let _ = disable_raw_mode();
    let _ = terminal.backend_mut().execute(LeaveAlternateScreen);
    let _ = terminal.show_cursor();

    res
}

async fn run<B>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    rx: &mut mpsc::UnboundedReceiver<AppEvent>,
) -> Result<()>
where
    B: ratatui::backend::Backend,
    <B as ratatui::backend::Backend>::Error:
        std::error::Error + Send + Sync + 'static,
{
    let mut events = EventStream::new();
    let mut tick = tokio::time::interval(std::time::Duration::from_millis(100));

    loop {
        terminal.draw(|f| ui::render(f, app))?;
        if app.should_quit {
            break;
        }

        tokio::select! {
            maybe_event = events.next() => {
                if let Some(Ok(event)) = maybe_event {
                    app.handle_terminal_event(event);
                }
            }
            maybe_app = rx.recv() => {
                if let Some(ae) = maybe_app {
                    app.handle_app_event(ae);
                }
            }
            _ = tick.tick() => {}
        }
    }
    Ok(())
}
